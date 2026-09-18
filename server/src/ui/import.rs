// SPDX-License-Identifier: AGPL-3.0-or-later
//! The import page: where a person brings a legacy model into the repository and decides,
//! loss by loss, what they are willing to give up.
//!
//! It renders a form to choose a binding from the registry and paste a source artifact, and,
//! after a run, the loss report grouped by verdict with the blocking entries first, each
//! naming its subject. A lossy import is REFUSED until every blocking loss is accepted by
//! name: the refusal is rendered as information with a way forward (check the losses you
//! accept and submit again), never as an error page. The page states, in the page and not
//! only in a log, what was retained (the artifact, content-addressed) and what was lost.
//!
//! The page performs the import through the SAME core the JSON endpoint calls
//! (crate::binding_api::import_core), with the same permission, scope and author decisions
//! in the same order, so the page and the endpoint cannot disagree about what an import did.

use std::collections::HashMap;

use axum::extract::{Form, Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use maud::{html, Markup};

use binding::{BindingInfo, Direction, LossReport, Mapping, MappingVerdict};

use crate::api::{map_store_error, validate_name, verify_actor, ApiState};
use crate::auth::{identity as resolve_identity, Identity, Permission};
use crate::binding_api::{decode_artifact, import_core, ImportCore, ImportOutcome};
use crate::binding_registry;
use crate::error::ApiError;
use crate::store::Commit;
use crate::ui::layout;

/// The branch an import targets when the form omits one, matching the model page's default.
const DEFAULT_BRANCH: &str = "main";

/// The values the form carries. There is no author field: like the editor and the merge
/// form, the author is the verified identity, never a name the browser supplies. The
/// acceptance checkboxes are carried as indexed accept_loss_ fields, so a submitted form
/// names exactly the losses the person checked.
struct ImportForm {
    binding: String,
    branch: String,
    message: String,
    artifact: String,
    holder: String,
    accept_losses: Vec<String>,
}

impl ImportForm {
    fn from_form(form: &HashMap<String, String>) -> Self {
        let binding = form.get("binding").cloned().unwrap_or_default();
        let branch = form
            .get("branch")
            .cloned()
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| DEFAULT_BRANCH.to_string());
        let message = form.get("message").cloned().unwrap_or_default();
        let artifact = form.get("artifact").cloned().unwrap_or_default();
        let holder = form.get("holder").cloned().unwrap_or_default();
        let accept_losses = parse_accept_losses(form);
        ImportForm {
            binding,
            branch,
            message,
            artifact,
            holder,
            accept_losses,
        }
    }
}

/// The checked acceptance boxes, in checkbox order. A checkbox submits its value only when
/// checked, so the indexes that actually appear are exactly the subjects the person
/// accepted.
fn parse_accept_losses(form: &HashMap<String, String>) -> Vec<String> {
    let mut indexes: Vec<usize> = form
        .keys()
        .filter_map(|key| key.strip_prefix("accept_loss_"))
        .filter_map(|index| index.parse::<usize>().ok())
        .collect();
    indexes.sort_unstable();
    indexes
        .iter()
        .filter_map(|index| form.get(&format!("accept_loss_{}", index)).cloned())
        .collect()
}

/// What a submitted import produced. A blocking refusal is a RESULT here, not an error: it
/// carries the form (so the acceptance form can preserve what was typed) and the losses that
/// still need a decision.
enum ImportResult {
    Committed {
        commit: Commit,
        artifact_hash: String,
        loss_report: LossReport,
    },
    Blocking {
        form: ImportForm,
        artifact_hash: String,
        loss_report: LossReport,
        unaccepted: Vec<Mapping>,
    },
}

/// GET /ui/projects/:project/import - the form, with a binding chosen from the registry.
pub async fn import_page(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(project): Path<String>,
) -> Response {
    let mechanism = state.auth.mechanism();
    let identity = match resolve_identity(&state, &headers).await {
        Ok(identity) => identity,
        Err(error) => return layout::sign_in_page(mechanism, &error.message),
    };
    match render_import_page(&state, &identity, &project) {
        Ok(page) => layout::html_response(StatusCode::OK, page),
        Err(error) => layout::error_page(
            error.status,
            Some(&identity.subject),
            mechanism,
            &error.message,
        ),
    }
}

fn render_import_page(
    state: &ApiState,
    identity: &Identity,
    project: &str,
) -> Result<Markup, ApiError> {
    // The SAME identity, permission and project-scope decisions as the read handlers, in
    // the SAME order: the workbench can never be a weaker path to the data. Viewing the
    // form needs read; submitting it is refused unless the caller may write.
    if !identity.may(Permission::Read) {
        return Err(ApiError::forbidden("read permission required"));
    }
    if !identity.may_reach(project) {
        return Err(ApiError::forbidden("project not in scope"));
    }
    if state
        .store
        .project(project)
        .map_err(map_store_error)?
        .is_none()
    {
        return Err(ApiError::not_found(format!("project {}", project)));
    }
    let bindings = binding_registry::bindings();
    Ok(import_page_markup(
        identity,
        state.auth.mechanism(),
        project,
        &bindings,
    ))
}

/// POST /ui/projects/:project/import - perform the import through the same core as the JSON
/// endpoint, and render the outcome: a committed page naming what was retained and lost, or
/// a refusal page showing the losses and asking for acceptance. A refusal is information,
/// never an error page.
pub async fn submit_import(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(project): Path<String>,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    let mechanism = state.auth.mechanism();
    let identity = match resolve_identity(&state, &headers).await {
        Ok(identity) => identity,
        Err(error) => return layout::sign_in_page(mechanism, &error.message),
    };
    match perform_import(&state, &identity, &project, &form) {
        Ok(ImportResult::Committed {
            commit,
            artifact_hash,
            loss_report,
        }) => layout::html_response(
            StatusCode::CREATED,
            committed_page(
                &identity,
                mechanism,
                &project,
                &commit,
                &artifact_hash,
                &loss_report,
            ),
        ),
        Ok(ImportResult::Blocking {
            form,
            artifact_hash,
            loss_report,
            unaccepted,
        }) => layout::html_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            blocking_page(
                &identity,
                mechanism,
                &project,
                &form,
                &artifact_hash,
                &loss_report,
                &unaccepted,
            ),
        ),
        Err(error) => layout::error_page(
            error.status,
            Some(&identity.subject),
            mechanism,
            &error.message,
        ),
    }
}

/// The import itself, through the SAME core the JSON handler uses
/// (crate::binding_api::import_core), with the same permission, scope and author decisions
/// and the same audit entries. The author is the verified identity, never a browser field.
fn perform_import(
    state: &ApiState,
    identity: &Identity,
    project: &str,
    form: &HashMap<String, String>,
) -> Result<ImportResult, ApiError> {
    if !identity.may(Permission::Write) {
        return Err(ApiError::forbidden("write permission required"));
    }
    if !identity.may_reach(project) {
        return Err(ApiError::forbidden("project not in scope"));
    }
    let input = ImportForm::from_form(form);
    if input.binding.is_empty() {
        return Err(ApiError::bad_request("a binding must be selected"));
    }
    validate_name("branch name", &input.branch)?;
    if input.message.trim().is_empty() {
        return Err(ApiError::bad_request("commit message must not be empty"));
    }
    if input.artifact.is_empty() {
        return Err(ApiError::bad_request(
            "the source artifact must not be empty",
        ));
    }
    // The author is the verified identity's subject, exactly as the editor and the merge
    // form resolve it: the form has no author field, so a browser cannot put a name into
    // the commit record.
    let author = identity.subject.clone();
    let holder = if input.holder.is_empty() {
        None
    } else {
        Some(input.holder.as_str())
    };
    verify_actor(&state.auth, identity, holder)?;

    // The artifact is pasted as raw text or base64; decode once, then hand the bytes to the
    // SAME core the endpoint calls.
    let artifact_bytes = decode_artifact(&input.artifact);
    match import_core(
        state.store.as_ref(),
        project,
        &ImportCore {
            binding: &input.binding,
            branch: &input.branch,
            author: &author,
            message: &input.message,
            artifact: &artifact_bytes,
            accept_losses: &input.accept_losses,
            holder,
            actor: &identity.subject,
            mechanism: state.auth.mechanism(),
        },
    )? {
        ImportOutcome::Committed {
            commit,
            artifact_hash,
            loss_report,
            ..
        } => Ok(ImportResult::Committed {
            commit,
            artifact_hash,
            loss_report,
        }),
        ImportOutcome::Blocking {
            artifact_hash,
            loss_report,
            unaccepted,
            ..
        } => Ok(ImportResult::Blocking {
            form: input,
            artifact_hash,
            loss_report,
            unaccepted,
        }),
    }
}

// ---------------------------------------------------------------------------
// The form.

fn import_page_markup(
    identity: &Identity,
    mechanism: &str,
    project: &str,
    bindings: &[BindingInfo],
) -> Markup {
    // The submit is offered only to a caller who may write: a reviewer who cannot write
    // must not be invited to fill a form that will only be refused on submit, exactly as
    // the merge and edit forms are only offered to writers.
    let can_import = identity.may(Permission::Write);
    let body = html! {
        h1 { "Import" }
        p {
            "Import a legacy model through a binding. The source artifact is retained "
            "byte-for-byte and content-addressed before anything else happens; the loss "
            "report names everything the binding could not carry, and nothing lossy is "
            "committed until its loss is accepted by name."
        }
        (import_form_markup(project, bindings, can_import))
    };
    let title = format!("modelwrite — {} — import", project);
    layout::shell(
        &title,
        Some(project),
        Some(&identity.subject),
        mechanism,
        body,
    )
}

fn import_form_markup(project: &str, bindings: &[BindingInfo], can_import: bool) -> Markup {
    html! {
        form method="post" action={ "/ui/projects/" (crate::ui::urlencode(project)) "/import" } class="import-form" {
            p {
                label for="binding" { "Binding" }
                select id="binding" name="binding" required {
                    @for binding in bindings {
                        option value=(format!("{}@{}", binding.id, binding.version)) {
                            (binding.id.as_str()) "@" (binding.version.as_str())
                            @if binding.direction == Direction::ImportOnly {
                                " (viewer)"
                            }
                        }
                    }
                }
            }
            p {
                label for="branch" { "Branch" }
                input type="text" id="branch" name="branch" value="main";
            }
            p {
                label for="message" { "Commit message" }
                input type="text" id="message" name="message" required;
            }
            p {
                label for="holder" { "Holder (optional)" }
                input type="text" id="holder" name="holder";
            }
            p {
                label for="artifact" { "Source artifact (paste raw text or base64)" }
                textarea id="artifact" name="artifact" required;
            }
            @if can_import {
                button type="submit" { "Import" }
            } @else {
                button type="submit" disabled { "Import" }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// The outcome pages: retained, lost, and (when refused) the acceptance control.

fn blocking_page(
    identity: &Identity,
    mechanism: &str,
    project: &str,
    form: &ImportForm,
    artifact_hash: &str,
    loss_report: &LossReport,
    unaccepted: &[Mapping],
) -> Markup {
    let blocking = loss_report.blocking();
    let body = html! {
        h1 { "Import refused: blocking losses" }
        p {
            "This import is lossy and nothing was committed. The source artifact was "
            "retained and content-addressed. Accept the losses below by name to commit the "
            "migration; a loss left unchecked refuses the import again."
        }
        (retained_markup(artifact_hash, None))
        (loss_report_markup(loss_report))
        h2 { "Accept the losses and import" }
        (accept_form_markup(project, form, &blocking, unaccepted))
    };
    let title = format!("modelwrite — {} — import refused", project);
    layout::shell(
        &title,
        Some(project),
        Some(&identity.subject),
        mechanism,
        body,
    )
}

fn committed_page(
    identity: &Identity,
    mechanism: &str,
    project: &str,
    commit: &Commit,
    artifact_hash: &str,
    loss_report: &LossReport,
) -> Markup {
    let body = html! {
        h1 { "Import committed" }
        p { "Commit " code { (short_hash(&commit.hash)) } }
        (retained_markup(artifact_hash, Some(&commit.hash)))
        (loss_report_markup(loss_report))
        p {
            a href={ "/ui/projects/" (crate::ui::urlencode(project)) "/model?commit=" (crate::ui::urlencode(commit.hash.as_str())) } {
                "View the imported model"
            }
        }
    };
    let title = format!("modelwrite — {} — import committed", project);
    layout::shell(
        &title,
        Some(project),
        Some(&identity.subject),
        mechanism,
        body,
    )
}

/// The acceptance form: the original inputs carried as hidden fields, plus one checkbox per
/// blocking loss. A checkbox that was already accepted on a prior submit stays checked, so a
/// partial acceptance is never thrown away on the next attempt.
fn accept_form_markup(
    project: &str,
    form: &ImportForm,
    blocking: &[&Mapping],
    unaccepted: &[Mapping],
) -> Markup {
    html! {
        form method="post" action={ "/ui/projects/" (crate::ui::urlencode(project)) "/import" } class="import-form" {
            input type="hidden" name="binding" value=(form.binding.as_str());
            input type="hidden" name="branch" value=(form.branch.as_str());
            input type="hidden" name="message" value=(form.message.as_str());
            input type="hidden" name="holder" value=(form.holder.as_str());
            input type="hidden" name="artifact" value=(form.artifact.as_str());
            ul class="loss-accept" {
                @for (index, mapping) in blocking.iter().enumerate() {
                    li {
                        label {
                            input type="checkbox"
                                name=(format!("accept_loss_{}", index))
                                value=(mapping.subject.as_str())
                                checked[!unaccepted.iter().any(|m| m.subject == mapping.subject)];
                            span class="loss-subject" { (mapping.subject.as_str()) }
                        }
                    }
                }
            }
            button type="submit" { "Accept the checked losses and import" }
        }
    }
}

// ---------------------------------------------------------------------------
// Retained and lost, stated on the page.

/// What was retained, stated on the page rather than only in a log: the source artifact,
/// content-addressed, and - once committed - the commit it landed as.
fn retained_markup(artifact_hash: &str, commit_hash: Option<&str>) -> Markup {
    html! {
        section class="model-section" id="retained" {
            h2 { "Retained" }
            p {
                "Source artifact retained byte-for-byte, content-addressed as "
                code { (artifact_hash) }
                "."
            }
            @if let Some(hash) = commit_hash {
                p { "Imported model committed as " code { (hash) } "." }
            }
        }
    }
}

/// The loss report, grouped by verdict with the blocking entries first: unmappable, then
/// lossy, then exact. Every entry names its subject, and the summary is taken from the
/// report itself, so the page can never claim a lossless migration the report contradicts.
fn loss_report_markup(report: &LossReport) -> Markup {
    let mut unmappable: Vec<&Mapping> = Vec::new();
    let mut lossy: Vec<&Mapping> = Vec::new();
    let mut exact: Vec<&Mapping> = Vec::new();
    for mapping in &report.mappings {
        match mapping.verdict {
            MappingVerdict::Unmappable => unmappable.push(mapping),
            MappingVerdict::Lossy => lossy.push(mapping),
            MappingVerdict::Exact => exact.push(mapping),
        }
    }
    let blocking = unmappable.len() + lossy.len();
    html! {
        section class="model-section" id="loss-report" {
            h2 { "Loss report" }
            p class="meta" {
                "binding " (report.binding.id.as_str()) "@" (report.binding.version.as_str())
            }
            @if blocking == 0 {
                p { "This import is lossless: every mapping is exact." }
            } @else {
                p {
                    "This import is " strong { "lossy" } ": "
                    (unmappable.len()) " unmappable, " (lossy.len()) " lossy. "
                    "Each must be accepted by name before the import can commit."
                }
            }
            (verdict_group("Unmappable", &unmappable))
            (verdict_group("Lossy", &lossy))
            (verdict_group("Exact", &exact))
        }
    }
}

/// One verdict group. Empty groups render nothing, so a report of only losses does not
/// invent an empty "exact" section, and a lossless report shows nothing but its summary.
fn verdict_group(heading: &str, mappings: &[&Mapping]) -> Markup {
    if mappings.is_empty() {
        return Markup::default();
    }
    html! {
        h3 { (heading) }
        ul class="loss-list" {
            @for mapping in mappings {
                li class="loss" {
                    span class="loss-subject" { (mapping.subject.as_str()) }
                    @if !mapping.note.is_empty() {
                        span class="loss-note" { " — " (mapping.note.as_str()) }
                    }
                }
            }
        }
    }
}

fn short_hash(hash: &str) -> &str {
    hash.get(..8).unwrap_or(hash)
}
