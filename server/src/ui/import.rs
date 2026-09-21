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

use axum::extract::{Form, Multipart, Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use maud::{html, Markup, PreEscaped};

use agent::losses::entry_identity;
use binding::{BindingInfo, Direction, LossReport, Mapping, MappingVerdict};

use crate::api::{map_store_error, validate_name, verify_actor, ApiState};
use crate::auth::{identity as resolve_identity, Identity, Permission};
use crate::binding_api::{
    accept_import_core, decode_artifact, import_core, ImportCore, ImportOutcome,
};
use crate::binding_registry;
use crate::error::ApiError;
use crate::store::Commit;
use crate::ui::layout;

/// The branch an import targets when the form omits one, matching the model page's default.
const DEFAULT_BRANCH: &str = "main";

/// A human-readable byte count, so the page can state the body limit as "512 MiB" rather
/// than a bare 536870912 that a person has to count.
pub(crate) fn human_bytes(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KiB", "MiB", "GiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{} B", bytes)
    } else if value >= 100.0 {
        format!("{:.0} {}", value, UNITS[unit])
    } else {
        format!("{:.1} {}", value, UNITS[unit])
    }
}

/// Whether a form rejection is axum's body-limit refusal. When it is, the page renders the
/// limit and the received size as information instead of a generic error.
fn form_length_limit(rejection: &axum::extract::rejection::FormRejection) -> bool {
    use axum::extract::rejection::{BytesRejection, FailedToBufferBody, FormRejection};
    matches!(
        rejection,
        FormRejection::BytesRejection(BytesRejection::FailedToBufferBody(
            FailedToBufferBody::LengthLimitError(_)
        ))
    )
}

/// The honest boundary of what the engine measured, stated on the page so a person accepting
/// a loss knows which half is measured and which half is the binding's word: the engine diffed
/// the binding's own OKF->XMI->OKF round trip; the native XMI->OKF read is NOT independently
/// measured and rests on the binding's self-reported loss report.
pub(crate) fn fidelity_note_markup(direction: Option<Direction>) -> Markup {
    match direction {
        Some(Direction::ImportOnly) => html! {
            p class="fidelity-note" {
                "This binding is a " strong { "viewer" } " (import-only): it reads the source into "
                "OKF but cannot write it back, so no round trip is measured. The loss report is "
                "the binding's own account of its native read, which is NOT independently measured."
            }
        },
        Some(Direction::ImportAndExport) => html! {
            p class="fidelity-note" {
                "Fidelity: the engine measured the binding's own OKF->XMI->OKF round trip and diffed "
                "it. The native XMI->OKF read of your artifact is NOT independently measured - the "
                "loss report is the binding's own account of that read."
            }
        },
        None => html! {
            p class="fidelity-note" {
                "Fidelity: for a read/write binding the engine measures and diffs the binding's own "
                "round trip; a viewer (import-only) cannot write back, so no round trip is measured. "
                "The native read of your artifact is NOT independently measured - the loss report is "
                "the binding's own account of it."
            }
        },
    }
}

/// A human-readable label for a mapping verdict, shown beside each loss so two entries that
/// name the same subject but drop different things are still distinguishable.
fn verdict_label(verdict: MappingVerdict) -> &'static str {
    match verdict {
        MappingVerdict::Exact => "exact",
        MappingVerdict::Lossy => "lossy",
        MappingVerdict::Unmappable => "unmappable",
    }
}

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
/// checked, so the indexes that actually appear are exactly the entry identities the person
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
        .store_for(identity)
        .project(project)
        .map_err(map_store_error)?
        .is_none()
    {
        return Err(ApiError::not_found(format!("project {}", project)));
    }
    let bindings = binding_registry::bindings();
    let mut nav = layout::Nav::load(state, identity, Some(project))?;
    nav.section = Some("import");
    Ok(import_page_markup(
        identity,
        state.auth.mechanism(),
        project,
        &bindings,
        &nav,
        state.max_body_bytes,
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
    form: Result<Form<HashMap<String, String>>, axum::extract::rejection::FormRejection>,
) -> Response {
    let mechanism = state.auth.mechanism();
    let identity = match resolve_identity(&state, &headers).await {
        Ok(identity) => identity,
        Err(error) => return layout::sign_in_page(mechanism, &error.message),
    };
    let nav = match layout::Nav::load(&state, &identity, Some(&project)) {
        Ok(nav) => nav,
        Err(error) => {
            return layout::error_page(
                error.status,
                Some(&identity.subject),
                mechanism,
                &error.message,
            );
        }
    };
    // A body the limit refused is rendered as information - the limit and the received size -
    // rather than as a crash or a bare axum 413 page.
    let form = match form {
        Ok(Form(form)) => form,
        Err(rejection) => {
            if form_length_limit(&rejection) {
                return layout::html_response(
                    StatusCode::PAYLOAD_TOO_LARGE,
                    too_large_page(
                        &identity,
                        mechanism,
                        &project,
                        &nav,
                        state.max_body_bytes,
                        None,
                    ),
                );
            }
            return layout::error_page(
                rejection.status(),
                Some(&identity.subject),
                mechanism,
                &rejection.body_text(),
            );
        }
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
                &nav,
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
                &artifact_hash,
                &form.branch,
                &form.message,
                &form.holder,
                &loss_report,
                &unaccepted,
                &nav,
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

/// What a streamed (file) import produced. A blocking refusal is a RESULT, not an error,
/// exactly as in the paste form; `TooLarge` is also a result, because the limit is enforced
/// here while the bytes are staged, so the page can name the limit and the received size.
enum StreamImportResult {
    Committed {
        commit: Box<Commit>,
        artifact_hash: String,
        loss_report: LossReport,
    },
    Blocking {
        artifact_hash: String,
        branch: String,
        message: String,
        holder: String,
        loss_report: LossReport,
    },
    TooLarge {
        received: u64,
    },
}

/// POST /ui/projects/:project/import/upload - the workbench's STREAMING import. The artifact
/// arrives as the `artifact` FILE field of a `multipart/form-data` body and is streamed into
/// the content-addressed blob store before import, so a 36 MB (or larger) vendor export does
/// not have to be pasted into a textarea or buffered as a base64 JSON string.
pub async fn submit_import_upload(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(project): Path<String>,
    mut multipart: Multipart,
) -> Response {
    let mechanism = state.auth.mechanism();
    let identity = match resolve_identity(&state, &headers).await {
        Ok(identity) => identity,
        Err(error) => return layout::sign_in_page(mechanism, &error.message),
    };
    let nav = match layout::Nav::load(&state, &identity, Some(&project)) {
        Ok(nav) => nav,
        Err(error) => {
            return layout::error_page(
                error.status,
                Some(&identity.subject),
                mechanism,
                &error.message,
            );
        }
    };
    match perform_stream_import(&state, &identity, &project, &mut multipart).await {
        Ok(StreamImportResult::Committed {
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
                &nav,
            ),
        ),
        Ok(StreamImportResult::Blocking {
            artifact_hash,
            branch,
            message,
            holder,
            loss_report,
        }) => {
            let unaccepted: Vec<Mapping> = loss_report.blocking().into_iter().cloned().collect();
            layout::html_response(
                StatusCode::UNPROCESSABLE_ENTITY,
                blocking_page(
                    &identity,
                    mechanism,
                    &project,
                    &artifact_hash,
                    &branch,
                    &message,
                    &holder,
                    &loss_report,
                    &unaccepted,
                    &nav,
                ),
            )
        }
        Ok(StreamImportResult::TooLarge { received }) => layout::html_response(
            StatusCode::PAYLOAD_TOO_LARGE,
            too_large_page(
                &identity,
                mechanism,
                &project,
                &nav,
                state.max_body_bytes,
                Some(received),
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

/// The streaming import itself: permission and scope first, then the bytes streamed to a
/// staged file while the limit is enforced, then retained content-addressed, then the SAME
/// [`import_core`] the paste form and the JSON endpoint call.
async fn perform_stream_import(
    state: &ApiState,
    identity: &Identity,
    project: &str,
    multipart: &mut Multipart,
) -> Result<StreamImportResult, ApiError> {
    use std::io::Write as _;

    if !identity.may(Permission::Write) {
        return Err(ApiError::forbidden("write permission required"));
    }
    if !identity.may_reach(project) {
        return Err(ApiError::forbidden("project not in scope"));
    }

    let mut binding = String::new();
    let mut branch = String::new();
    let mut message = String::new();
    let mut holder = String::new();
    let mut accept_losses: Vec<String> = Vec::new();

    let mut staged = tempfile::NamedTempFile::new()
        .map_err(|e| ApiError::internal(format!("could not stage the artifact: {}", e)))?;
    let mut received: u64 = 0;
    let mut got_artifact = false;

    loop {
        let field = match multipart.next_field().await {
            Ok(Some(field)) => field,
            Ok(None) => break,
            Err(error) => {
                if error.status() == StatusCode::PAYLOAD_TOO_LARGE {
                    return Ok(StreamImportResult::TooLarge { received });
                }
                return Err(ApiError::bad_request(format!(
                    "could not read the import upload: {}",
                    error.body_text()
                )));
            }
        };
        let name = field.name().unwrap_or_default().to_string();
        match name.as_str() {
            "artifact" => {
                got_artifact = true;
                let mut field = field;
                loop {
                    let chunk = match field.chunk().await {
                        Ok(Some(chunk)) => chunk,
                        Ok(None) => break,
                        Err(error) => {
                            if error.status() == StatusCode::PAYLOAD_TOO_LARGE {
                                return Ok(StreamImportResult::TooLarge { received });
                            }
                            return Err(ApiError::bad_request(format!(
                                "could not read the import upload: {}",
                                error.body_text()
                            )));
                        }
                    };
                    received = received.saturating_add(chunk.len() as u64);
                    if received > state.max_body_bytes {
                        return Ok(StreamImportResult::TooLarge { received });
                    }
                    staged.write_all(&chunk).map_err(|e| {
                        ApiError::internal(format!("could not stage the artifact: {}", e))
                    })?;
                }
            }
            other => {
                let value = field.text().await.map_err(|e| {
                    ApiError::bad_request(format!(
                        "could not read the import upload: {}",
                        e.body_text()
                    ))
                })?;
                match other {
                    "binding" => binding = value,
                    "branch" => branch = value,
                    "message" => message = value,
                    "holder" => holder = value,
                    "acceptLosses" => accept_losses.push(value),
                    _ => {}
                }
            }
        }
    }

    if !got_artifact {
        return Err(ApiError::bad_request(
            "the source artifact must be supplied as the `artifact` file field",
        ));
    }
    if binding.is_empty() {
        return Err(ApiError::bad_request("a binding must be selected"));
    }
    if branch.is_empty() {
        branch = DEFAULT_BRANCH.to_string();
    }
    validate_name("branch name", &branch)?;
    if message.trim().is_empty() {
        return Err(ApiError::bad_request("commit message must not be empty"));
    }
    // The author is the verified identity's subject, exactly as the paste form resolves it.
    let author = identity.subject.clone();
    let holder_opt = if holder.is_empty() {
        None
    } else {
        Some(holder.as_str())
    };
    verify_actor(&state.auth, identity, holder_opt)?;

    // Retain the artifact content-addressed BEFORE import (rule 1), streaming the staged
    // file into the blob store under its hash rather than buffering it.
    let temp_path = staged.into_temp_path();
    let artifact_hash = state
        .store_for(identity)
        .put_blob_file(temp_path.as_ref())
        .map_err(map_store_error)?;
    let artifact_bytes = state
        .store_for(identity)
        .blob(&artifact_hash)
        .map_err(map_store_error)?
        .ok_or_else(|| {
            eprintln!(
                "missing blob {} immediately after storing it",
                artifact_hash
            );
            ApiError::internal("the retained artifact could not be read back")
        })?;

    match import_core(
        state.store_for(identity).as_ref(),
        project,
        &ImportCore {
            binding: &binding,
            branch: &branch,
            author: &author,
            message: &message,
            artifact: &artifact_bytes,
            accept_losses: &accept_losses,
            holder: holder_opt,
            actor: &identity.subject,
            mechanism: state.auth.mechanism(),
            authorizer: state.auth.authorizer().unwrap_or(""),
            acceptance: None,
        },
    )? {
        ImportOutcome::Committed {
            commit,
            artifact_hash,
            loss_report,
            ..
        } => Ok(StreamImportResult::Committed {
            commit: Box::new(*commit),
            artifact_hash,
            loss_report,
        }),
        ImportOutcome::Blocking {
            artifact_hash,
            loss_report,
            ..
        } => Ok(StreamImportResult::Blocking {
            artifact_hash,
            branch,
            message,
            holder,
            loss_report,
        }),
    }
}

/// The values the hash-based acceptance form carries: the retained artifact's hash plus the
/// commit metadata the original import already captured, and the checked losses. There is no
/// artifact and no binding field: both are recovered from the import record the hash names.
pub(crate) struct AcceptForm {
    pub(crate) artifact_hash: String,
    pub(crate) branch: String,
    pub(crate) message: String,
    pub(crate) holder: String,
    pub(crate) accept_losses: Vec<String>,
}

impl AcceptForm {
    pub(crate) fn from_form(form: &HashMap<String, String>) -> Self {
        let artifact_hash = form.get("artifactHash").cloned().unwrap_or_default();
        let branch = form
            .get("branch")
            .cloned()
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| DEFAULT_BRANCH.to_string());
        let message = form.get("message").cloned().unwrap_or_default();
        let holder = form.get("holder").cloned().unwrap_or_default();
        let accept_all = form
            .get("accept_all")
            .map(|value| value == "1")
            .unwrap_or(false);
        // "Accept all" carries every blocking entry identity in ONE hidden field, so a single
        // click accepts the whole report without checking 252 boxes. The identities are
        // newline-joined; an entry identity never contains a newline.
        //
        // Split with str::lines, never with split('\n'): the HTML form-encoding algorithm
        // REQUIRES a browser to write every LF in a field value as CRLF (the urlencoded
        // serializer says so, for legacy reasons), so a real browser submits CRLF-joined
        // identities. Splitting on '\n' alone left a trailing carriage return on all 252 of
        // them, none matched the report, and the one-button acceptance silently refused itself.
        // str::lines splits on LF and strips one trailing CR, which is exactly this encoding.
        let accept_losses = if accept_all {
            form.get("all_losses")
                .map(|raw| {
                    raw.lines()
                        .filter(|identity| !identity.is_empty())
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default()
        } else {
            parse_accept_losses(form)
        };
        AcceptForm {
            artifact_hash,
            branch,
            message,
            holder,
            accept_losses,
        }
    }
}

/// What a hash-based acceptance produced. A blocking refusal is a RESULT here, not an error,
/// exactly as in the paste and stream forms: it carries the losses still awaiting a decision.
enum AcceptImportResult {
    Committed {
        commit: Box<Commit>,
        artifact_hash: String,
        loss_report: LossReport,
    },
    Blocking {
        artifact_hash: String,
        loss_report: LossReport,
        unaccepted: Vec<Mapping>,
    },
}

/// POST /ui/projects/:project/import/accept - complete a refused import from its RETAINED
/// artifact. The form carries the artifact hash and the checked losses (plus the commit
/// metadata already captured on the original import); the bytes are re-read from the blob
/// store by hash, so the person never uploads the file again.
pub async fn accept_import_form(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(project): Path<String>,
    form: Result<Form<HashMap<String, String>>, axum::extract::rejection::FormRejection>,
) -> Response {
    let mechanism = state.auth.mechanism();
    let identity = match resolve_identity(&state, &headers).await {
        Ok(identity) => identity,
        Err(error) => return layout::sign_in_page(mechanism, &error.message),
    };
    let nav = match layout::Nav::load(&state, &identity, Some(&project)) {
        Ok(nav) => nav,
        Err(error) => {
            return layout::error_page(
                error.status,
                Some(&identity.subject),
                mechanism,
                &error.message,
            );
        }
    };
    let form = match form {
        Ok(Form(form)) => form,
        Err(rejection) => {
            return layout::error_page(
                rejection.status(),
                Some(&identity.subject),
                mechanism,
                &rejection.body_text(),
            );
        }
    };
    // Preserve the commit metadata before the import runs, so an incomplete acceptance
    // re-renders the form intact rather than dropping the branch and message.
    let accept = AcceptForm::from_form(&form);
    match perform_accept_import(&state, &identity, &project, &form) {
        Ok(AcceptImportResult::Committed {
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
                &nav,
            ),
        ),
        Ok(AcceptImportResult::Blocking {
            artifact_hash,
            loss_report,
            unaccepted,
        }) => layout::html_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            blocking_page(
                &identity,
                mechanism,
                &project,
                &artifact_hash,
                &accept.branch,
                &accept.message,
                &accept.holder,
                &loss_report,
                &unaccepted,
                &nav,
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

/// The hash-based acceptance itself: permission and scope first, then the SAME
/// [accept_import_core] the JSON endpoint calls, so the workbench and the API cannot diverge.
fn perform_accept_import(
    state: &ApiState,
    identity: &Identity,
    project: &str,
    form: &HashMap<String, String>,
) -> Result<AcceptImportResult, ApiError> {
    if !identity.may(Permission::Write) {
        return Err(ApiError::forbidden("write permission required"));
    }
    if !identity.may_reach(project) {
        return Err(ApiError::forbidden("project not in scope"));
    }
    let input = AcceptForm::from_form(form);
    if input.artifact_hash.is_empty() {
        return Err(ApiError::bad_request("an artifact hash is required"));
    }
    if input.accept_losses.is_empty() {
        return Err(ApiError::bad_request(
            "at least one accepted loss is required",
        ));
    }
    validate_name("branch name", &input.branch)?;
    if input.message.trim().is_empty() {
        return Err(ApiError::bad_request("commit message must not be empty"));
    }
    let author = identity.subject.clone();
    let holder = if input.holder.is_empty() {
        None
    } else {
        Some(input.holder.as_str())
    };
    verify_actor(&state.auth, identity, holder)?;

    match accept_import_core(
        state.store_for(identity).as_ref(),
        project,
        &input.artifact_hash,
        &input.branch,
        &author,
        &input.message,
        holder,
        &input.accept_losses,
        &identity.subject,
        state.auth.mechanism(),
        state.auth.authorizer().unwrap_or(""),
    )? {
        ImportOutcome::Committed {
            commit,
            artifact_hash,
            loss_report,
            ..
        } => Ok(AcceptImportResult::Committed {
            commit: Box::new(*commit),
            artifact_hash,
            loss_report,
        }),
        ImportOutcome::Blocking {
            artifact_hash,
            loss_report,
            unaccepted,
            ..
        } => Ok(AcceptImportResult::Blocking {
            artifact_hash,
            loss_report,
            unaccepted,
        }),
    }
}

/// The refusal page for an oversize upload: the limit and the received size are stated as
/// information, and the next step (raise the limit) is named rather than a crash shown.
#[allow(clippy::too_many_arguments)]
fn too_large_page(
    identity: &Identity,
    mechanism: &str,
    project: &str,
    nav: &layout::Nav,
    limit: u64,
    received: Option<u64>,
) -> Markup {
    let body = html! {
        h1 { "Upload refused: too large" }
        p {
            "The upload was refused before import was attempted because it exceeds the "
            "configured body limit of " strong { (human_bytes(limit)) } " (" (limit) " bytes)."
        }
        @if let Some(received) = received {
            p { "Received " strong { (human_bytes(received)) } " (" (received) " bytes) before refusing." }
        } @else {
            p { "The received size was not declared, so the exact size is unknown." }
        }
        p {
            "Nothing was imported and nothing was stored. To import a larger model, raise "
            "MW_MAX_BODY_BYTES on the server and restart it."
        }
        p { a href={ "/ui/projects/" (crate::ui::urlencode(project)) "/import" } { "Back to import" } }
    };
    let title = format!("modelwrite — {} — import refused: too large", project);
    layout::shell(
        &title,
        nav,
        Some(&identity.subject),
        identity.may(Permission::Administer),
        mechanism,
        body,
    )
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
        state.store_for(identity).as_ref(),
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
            authorizer: state.auth.authorizer().unwrap_or(""),
            acceptance: None,
        },
    )? {
        ImportOutcome::Committed {
            commit,
            artifact_hash,
            loss_report,
            ..
        } => Ok(ImportResult::Committed {
            commit: *commit,
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
    nav: &layout::Nav,
    max_body_bytes: u64,
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
        p class="body-limit" {
            "The largest artifact this server accepts is "
            strong { (human_bytes(max_body_bytes)) }
            " (" (max_body_bytes) " bytes, set by MW_MAX_BODY_BYTES). Uploads larger than "
            "that are refused before import with a 413 that names the limit."
        }
        (fidelity_note_markup(None))
        h2 { "Upload a file" }
        (upload_form_markup(project, bindings, can_import))
        h2 { "Or paste a small artifact" }
        (import_form_markup(project, bindings, can_import))
    };
    let title = format!("modelwrite — {} — import", project);
    layout::shell(
        &title,
        nav,
        Some(&identity.subject),
        identity.may(Permission::Administer),
        mechanism,
        body,
    )
}

/// The file-upload form. Its `enctype` is `multipart/form-data` and its `artifact` is a file
/// input, so the bytes are STREAMED to the content-addressed blob store instead of being
/// pasted through a JSON string; this is the path a 36 MB (or larger) vendor export takes.
fn upload_form_markup(project: &str, bindings: &[BindingInfo], can_import: bool) -> Markup {
    html! {
        form method="post"
             action={ "/ui/projects/" (crate::ui::urlencode(project)) "/import/upload" }
             enctype="multipart/form-data" class="import-form" {
            p {
                label for="binding-upload" { "Binding" }
                select id="binding-upload" name="binding" required {
                    @for binding in bindings {
                        option value=(format!("{}@{}", binding.id, binding.version)) {
                            (binding.id.as_str()) "@" (binding.version.as_str())
                            @if binding.direction == Direction::ImportOnly {
                                " (viewer: reads, does not write back)"
                            }
                        }
                    }
                }
            }
            p {
                label for="branch-upload" { "Branch" }
                input type="text" id="branch-upload" name="branch" value="main";
            }
            p {
                label for="message-upload" { "Commit message" }
                input type="text" id="message-upload" name="message" required;
            }
            p {
                label for="holder-upload" { "Holder (optional)" }
                input type="text" id="holder-upload" name="holder";
            }
            p {
                label for="artifact-upload" { "Source artifact (file)" }
                input type="file" id="artifact-upload" name="artifact" required;
            }
            @if can_import {
                button type="submit" { "Import file" }
            } @else {
                button type="submit" disabled { "Import file" }
            }
        }
    }
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
                                " (viewer: reads, does not write back)"
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

#[allow(clippy::too_many_arguments)]
pub(crate) fn blocking_page(
    identity: &Identity,
    mechanism: &str,
    project: &str,
    artifact_hash: &str,
    branch: &str,
    message: &str,
    holder: &str,
    loss_report: &LossReport,
    unaccepted: &[Mapping],
    nav: &layout::Nav,
) -> Markup {
    let blocking = loss_report.blocking();
    let body = html! {
        h1 { "Import refused: blocking losses" }
        p {
            "This import is lossy and nothing was committed. The source artifact was "
            "retained and content-addressed. Accept the losses below by name to commit the "
            "migration; a loss left unchecked refuses the import again. Accepting re-reads "
            "the retained bytes by their hash - you do not upload the file again."
        }
        (retained_markup(artifact_hash, None))
        (loss_report_markup(loss_report))
        (fidelity_note_markup(Some(loss_report.binding.direction)))
        h2 { "Accept the losses and import" }
        (accept_form_markup(project, artifact_hash, branch, message, holder, &blocking, unaccepted))
    };
    let title = format!("modelwrite — {} — import refused", project);
    layout::shell(
        &title,
        nav,
        Some(&identity.subject),
        identity.may(Permission::Administer),
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
    nav: &layout::Nav,
) -> Markup {
    let body = html! {
        h1 { "Import committed" }
        p { "Commit " code { (short_hash(&commit.hash)) } }
        (retained_markup(artifact_hash, Some(&commit.hash)))
        (loss_report_markup(loss_report))
        (fidelity_note_markup(Some(loss_report.binding.direction)))
        p {
            a href={ "/ui/projects/" (crate::ui::urlencode(project)) "/model?commit=" (crate::ui::urlencode(commit.hash.as_str())) } {
                "View the imported model"
            }
        }
    };
    let title = format!("modelwrite — {} — import committed", project);
    layout::shell(
        &title,
        nav,
        Some(&identity.subject),
        identity.may(Permission::Administer),
        mechanism,
        body,
    )
}

/// The inline enhancement for the loss-acceptance form: the select-all toggle and the live
/// count of how many losses will be accepted. The page still works without it - the accept-all
/// button needs no JavaScript, because it submits every blocking identity in one hidden field.
const SELECT_ALL_SCRIPT: &str = r#"(function () {
  'use strict';
  var form = document.getElementById('import-accept-form');
  if (!form) { return; }
  var selectAll = form.querySelector('.mw-select-all');
  var boxes = form.querySelectorAll('.loss-accept input[type="checkbox"]');
  var count = form.querySelector('.mw-accept-count');
  function sync() {
    var n = 0;
    for (var i = 0; i < boxes.length; i += 1) { if (boxes[i].checked) { n += 1; } }
    if (count) { count.textContent = String(n) + ' selected'; }
    if (selectAll) { selectAll.checked = boxes.length > 0 && n === boxes.length; }
  }
  if (selectAll) {
    selectAll.addEventListener('change', function () {
      for (var i = 0; i < boxes.length; i += 1) { boxes[i].checked = selectAll.checked; }
      sync();
    });
  }
  for (var j = 0; j < boxes.length; j += 1) { boxes[j].addEventListener('change', sync); }
  sync();
})();
"#;

/// The acceptance form: the original inputs carried as hidden fields, plus one checkbox per
/// blocking loss, a select-all toggle and an accept-all action. A checkbox that was already
/// accepted on a prior submit stays checked, so a partial acceptance is never thrown away on
/// the next attempt. The accept-all button submits every blocking identity at once - one click
/// for the 252-loss report, with the per-entry boxes still there for a person who wants to
/// decide each one.
#[allow(clippy::too_many_arguments)]
pub(crate) fn accept_form_markup(
    project: &str,
    artifact_hash: &str,
    branch: &str,
    message: &str,
    holder: &str,
    blocking: &[&Mapping],
    unaccepted: &[Mapping],
) -> Markup {
    let total = blocking.len();
    let already_accepted = total.saturating_sub(unaccepted.len());
    // Every blocking entry identity, newline-joined, so the accept-all button can submit the
    // whole report in one click without any checkbox.
    let all_identities = blocking
        .iter()
        .map(|mapping| entry_identity(mapping))
        .collect::<Vec<_>>()
        .join("\n");
    html! {
        form method="post" id="import-accept-form"
             action={ "/ui/projects/" (crate::ui::urlencode(project)) "/import/accept" } class="import-form" {
            input type="hidden" name="artifactHash" value=(artifact_hash);
            input type="hidden" name="branch" value=(branch);
            input type="hidden" name="message" value=(message);
            input type="hidden" name="holder" value=(holder);
            input type="hidden" name="all_losses" value=(all_identities);
            p class="loss-accept-summary" {
                strong { (total) } " blocking " @if total == 1 { "loss" } @else { "losses" }
                " · " span class="mw-accept-count" { (already_accepted) " selected" }
            }
            p class="loss-select-all" {
                label {
                    input type="checkbox" class="mw-select-all";
                    " Select all " (total) " losses"
                }
            }
            ul class="loss-accept" {
                @for (index, mapping) in blocking.iter().enumerate() {
                    li {
                        label {
                            input type="checkbox"
                                name=(format!("accept_loss_{}", index))
                                value=(entry_identity(mapping))
                                checked[!unaccepted.iter().any(|m| entry_identity(m) == entry_identity(mapping))];
                            span class="loss-subject" { (mapping.subject.as_str()) }
                            span class="loss-verdict" { " (" (verdict_label(mapping.verdict)) ")" }
                            @if !mapping.note.is_empty() {
                                span class="loss-note" { " — " (mapping.note.as_str()) }
                            }
                        }
                    }
                }
            }
            p class="accept-actions" {
                button type="submit" name="accept_all" value="1" { "Accept all " (total) " losses and import" }
                button type="submit" { "Accept the checked losses and import" }
            }
        }
        script { (PreEscaped(SELECT_ALL_SCRIPT)) }
    }
}

// ---------------------------------------------------------------------------
// Retained and lost, stated on the page.

/// What was retained, stated on the page rather than only in a log: the source artifact,
/// content-addressed, and - once committed - the commit it landed as.
pub(crate) fn retained_markup(artifact_hash: &str, commit_hash: Option<&str>) -> Markup {
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
pub(crate) fn loss_report_markup(report: &LossReport) -> Markup {
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
                @if report.binding.direction == Direction::ImportOnly {
                    " — viewer (import-only: reads, does not write back)"
                }
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
                    span class="loss-verdict" { " (" (verdict_label(mapping.verdict)) ")" }
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
