// SPDX-License-Identifier: AGPL-3.0-or-later
//! The element editor: a form for one element's id, name, documentation, stereotypes and
//! attributes that, on submit, becomes a commit through the SAME code path the JSON commit
//! handler uses - the same validation before storage, the same lock guard inside the
//! transaction, the same audit entry and the same permission decisions. The page takes a
//! short lease on the element for the duration of the request and refuses to submit while
//! another holder has it, naming that holder, so two people can never silently overwrite
//! each other. The author is the verified identity's subject, never a field the browser
//! supplies.

use std::collections::HashMap;

use axum::extract::{Form, Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use maud::{html, Markup};

use okf::types::{Attribute, Element, OkfRoot};

use crate::api::{
    commit_refusal_guard, load_model, lock_refusal_message, map_store_error, record_refusal,
    touched_elements, validate_element_name, validate_name, ApiState,
};
use crate::auth::{identity as resolve_identity, Identity, Permission};
use crate::error::ApiError;
use crate::store::{now_seconds, AuditEntry, Commit, CommitGuard, Lock, StoreError};
use crate::ui::layout;

/// The branch the editor targets. One branch keeps the first editing form simple; the model
/// page resolves its branch the same way, defaulting to `main`.
const DEFAULT_BRANCH: &str = "main";

/// The lease the edit request holds on its element, in seconds. It is released the moment
/// the request finishes, so this value is only a ceiling for a crashed request.
const EDIT_TTL_SECONDS: i64 = 60;

/// The values the form carries. On GET it is filled from the stored element; on POST it is
/// filled from the submitted fields, so a refused edit re-renders with what the caller typed.
struct EditInput {
    id: String,
    name: String,
    documentation: String,
    stereotypes: String,
    attributes: Vec<Attribute>,
    branch: String,
    message: String,
}

impl EditInput {
    fn from_element(branch: &str, element: &Element) -> Self {
        Self {
            id: element.id.clone(),
            name: element.name.clone(),
            documentation: element.documentation.clone(),
            stereotypes: element.stereotypes.join(", "),
            attributes: element.attributes.clone(),
            branch: branch.to_string(),
            message: String::new(),
        }
    }

    /// Build the input from the submitted form. An absent id keeps the element's existing id
    /// (the one in the URL); a present-but-empty id is an explicit request to empty it, which
    /// the validator then refuses, exactly as the JSON commit handler would refuse an empty id.
    fn from_form(form: &HashMap<String, String>, fallback_id: &str) -> Self {
        let id = form
            .get("id")
            .cloned()
            .unwrap_or_else(|| fallback_id.to_string());
        let name = form.get("name").cloned().unwrap_or_default();
        let documentation = form.get("documentation").cloned().unwrap_or_default();
        let stereotypes = form.get("stereotypes").cloned().unwrap_or_default();
        let attributes = parse_attributes(form);
        let branch = form
            .get("branch")
            .cloned()
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| DEFAULT_BRANCH.to_string());
        let message = form.get("message").cloned().unwrap_or_default();
        Self {
            id,
            name,
            documentation,
            stereotypes,
            attributes,
            branch,
            message,
        }
    }
}

/// What a submitted edit produced. A refusal is a RESULT here, not an error: the page renders
/// the validator's errors or the holder's name next to the form, never a 500.
enum EditOutcome {
    Committed {
        commit: Commit,
    },
    Invalid {
        input: EditInput,
        errors: Vec<String>,
    },
    Locked {
        input: EditInput,
        message: String,
    },
}

/// `GET /ui/projects/:project/edit/:element` - the form, prefilled from the stored element
/// and showing which other holders currently hold it. A held element renders with its submit
/// disabled, naming the holder and how to proceed.
pub async fn edit_form(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path((project, element)): Path<(String, String)>,
) -> Response {
    let mechanism = state.auth.mechanism();
    let identity = match resolve_identity(&state, &headers).await {
        Ok(identity) => identity,
        Err(error) => return layout::sign_in_page(mechanism, &error.message),
    };
    match render_edit_form(&state, &identity, &project, &element) {
        Ok(page) => layout::html_response(StatusCode::OK, page),
        Err(error) => layout::error_page(
            error.status,
            Some(&identity.subject),
            mechanism,
            &error.message,
        ),
    }
}

fn render_edit_form(
    state: &ApiState,
    identity: &Identity,
    project: &str,
    element_id: &str,
) -> Result<Markup, ApiError> {
    // The SAME identity, permission and project-scope decisions as the read handlers, in the
    // SAME order: the workbench can never be a weaker path to the data.
    if !identity.may(Permission::Read) {
        return Err(ApiError::forbidden("read permission required"));
    }
    if !identity.may_reach(project) {
        return Err(ApiError::forbidden("project not in scope"));
    }
    validate_element_name(element_id)?;
    if state
        .store
        .project(project)
        .map_err(map_store_error)?
        .is_none()
    {
        return Err(ApiError::not_found(format!("project {}", project)));
    }
    let tip = state
        .store
        .branch_tip(project, DEFAULT_BRANCH)
        .map_err(map_store_error)?
        .ok_or_else(|| ApiError::not_found(format!("branch {} has no commits", DEFAULT_BRANCH)))?;
    let root = load_model(state.store.as_ref(), project, &tip).map_err(map_store_error)?;
    let element = element_in(&root, element_id)
        .ok_or_else(|| ApiError::not_found(format!("element {}", element_id)))?;
    let input = EditInput::from_element(DEFAULT_BRANCH, element);
    let holders = state
        .store
        .holders_of(project, &[element_id.to_string()], now_seconds())
        .map_err(map_store_error)?;
    Ok(edit_page(
        identity,
        state.auth.mechanism(),
        project,
        element_id,
        &input,
        &holders,
        &[],
    ))
}

/// `POST /ui/projects/:project/edit/:element` - apply the edit and commit it through the
/// existing commit path. The author is the verified identity, a lock is taken on the element
/// for the duration of the request, and an invalid or locked result is rendered as
/// information rather than as an error page.
pub async fn submit_edit(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path((project, element)): Path<(String, String)>,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    let mechanism = state.auth.mechanism();
    let identity = match resolve_identity(&state, &headers).await {
        Ok(identity) => identity,
        Err(error) => return layout::sign_in_page(mechanism, &error.message),
    };
    match perform_edit(&state, &identity, &project, &element, &form) {
        Ok(EditOutcome::Committed { commit }) => layout::html_response(
            StatusCode::CREATED,
            edit_success_page(&identity, mechanism, &project, &commit),
        ),
        Ok(EditOutcome::Invalid { input, errors }) => {
            let holders = current_holders(&state, &project, &element);
            layout::html_response(
                StatusCode::UNPROCESSABLE_ENTITY,
                edit_page(
                    &identity, mechanism, &project, &element, &input, &holders, &errors,
                ),
            )
        }
        Ok(EditOutcome::Locked { input, message }) => {
            let holders = current_holders(&state, &project, &element);
            layout::html_response(
                StatusCode::CONFLICT,
                edit_page(
                    &identity,
                    mechanism,
                    &project,
                    &element,
                    &input,
                    &holders,
                    &[message],
                ),
            )
        }
        Err(error) => layout::error_page(
            error.status,
            Some(&identity.subject),
            mechanism,
            &error.message,
        ),
    }
}

/// The edit itself, mirroring [crate::api::create_commit] step for step with the same public
/// functions and the same permission decisions. Validation runs BEFORE anything is stored or
/// locked, so an invalid edit leaves no blob and no lock behind. A lock is then taken on the
/// element for the duration of the request and released before returning, whatever the
/// commit's result.
fn perform_edit(
    state: &ApiState,
    identity: &Identity,
    project: &str,
    element_id: &str,
    form: &HashMap<String, String>,
) -> Result<EditOutcome, ApiError> {
    if !identity.may(Permission::Write) {
        return Err(ApiError::forbidden("write permission required"));
    }
    if !identity.may_reach(project) {
        return Err(ApiError::forbidden("project not in scope"));
    }
    validate_element_name(element_id)?;
    let input = EditInput::from_form(form, element_id);
    validate_name("branch name", &input.branch)?;
    if input.message.trim().is_empty() {
        return Err(ApiError::bad_request("commit message must not be empty"));
    }
    if state
        .store
        .project(project)
        .map_err(map_store_error)?
        .is_none()
    {
        return Err(ApiError::not_found(format!("project {}", project)));
    }
    let tip = state
        .store
        .branch_tip(project, &input.branch)
        .map_err(map_store_error)?
        .ok_or_else(|| ApiError::not_found(format!("branch {} has no commits", input.branch)))?;
    let root = load_model(state.store.as_ref(), project, &tip).map_err(map_store_error)?;
    if element_in(&root, element_id).is_none() {
        return Err(ApiError::not_found(format!("element {}", element_id)));
    }

    let mut candidate = root.clone();
    if let Some(element) = find_element(&mut candidate, element_id) {
        apply_edit(element, &input);
    }

    // The document must still be a valid OKF model. The edit changes the fields the
    // validator watches over (an element id), so this is a real gate, not a formality: an
    // invalid result renders its errors next to the form and stores nothing.
    let report = okf::validate::validate(&candidate);
    if !report.valid {
        return Ok(EditOutcome::Invalid {
            input,
            errors: report.errors,
        });
    }

    // A lock is taken for the request, so a second editor cannot slip in between validation
    // and commit. The holder is the verified identity, never a field the browser supplies.
    let now = now_seconds();
    let holder = identity.subject.as_str();
    let locks = match state.store.acquire_locks(
        project,
        &input.branch,
        &[element_id.to_string()],
        holder,
        EDIT_TTL_SECONDS,
        now,
        None,
    ) {
        Ok(locks) => locks,
        Err(StoreError::Locked {
            element,
            holder: locked_by,
            expires_at,
        }) => {
            let message = lock_refusal_message(&element, &locked_by, expires_at);
            // Record the refusal, exactly as the commit path records a lock-refused commit:
            // the overwrite the lock prevented is the highest-value event this feature
            // produces, and the log exists to show what was tried, not only what succeeded.
            if let Err(recording) = record_refusal(
                state.store.as_ref(),
                project,
                &identity.subject,
                state.auth.mechanism(),
                "commit.refused",
                &input.branch,
                &message,
            ) {
                eprintln!("could not record the edit refusal: {:?}", recording);
            }
            return Ok(EditOutcome::Locked { input, message });
        }
        Err(error) => return Err(map_store_error(error)),
    };
    let lock_ids: Vec<String> = locks.iter().map(|lock| lock.id.clone()).collect();

    // One call, one transaction: the parents come from the tip the store reads inside the
    // same lock that writes the commit, the lock guard runs inside that transaction, and the
    // audit row rides the SAME transaction, so the commit and its record succeed or fail
    // together. The author is the verified identity's subject.
    let author = identity.subject.clone();
    let commit_result: Result<Commit, StoreError> = (|| {
        let bytes = serde_json::to_vec(&candidate).map_err(|error| {
            eprintln!("edited model could not be serialised: {}", error);
            StoreError::Backend("the edited model could not be stored".to_string())
        })?;
        let okf_hash = state.store.put_blob(&bytes)?;
        let touched = touched_elements(&root, &candidate);
        let guard = CommitGuard {
            holder,
            elements: &touched,
            now,
            expected_tip: Some(&tip),
        };
        let audit = AuditEntry {
            id: 0,
            project: project.to_string(),
            at: now,
            actor: identity.subject.clone(),
            mechanism: state.auth.mechanism().to_string(),
            action: "commit.create".to_string(),
            subject: input.branch.clone(),
            detail: input.message.clone(),
        };
        commit_refusal_guard(
            state.store.as_ref(),
            project,
            &input.branch,
            &identity.subject,
            state.auth.mechanism(),
            state.store.commit_model(
                project,
                &input.branch,
                &okf_hash,
                &author,
                &input.message,
                Some(guard),
                Some(&audit),
            ),
        )
    })();

    // The lease was for the duration of the request. Release it whatever the commit did, so
    // the element is never left locked after the request finishes.
    if let Err(release) = state.store.release_locks(project, holder, &lock_ids, None) {
        eprintln!("could not release the edit lock: {:?}", release);
    }

    match commit_result {
        Ok(commit) => Ok(EditOutcome::Committed { commit }),
        Err(StoreError::Locked {
            element,
            holder: locked_by,
            expires_at,
        }) => Ok(EditOutcome::Locked {
            input,
            message: lock_refusal_message(&element, &locked_by, expires_at),
        }),
        Err(error) => Err(map_store_error(error)),
    }
}

/// The live leases on the element, best-effort for re-rendering a refused edit. The
/// authoritative lock check already ran in [perform_edit]; this only makes the page truthful
/// about who holds the element when it refuses.
fn current_holders(state: &ApiState, project: &str, element_id: &str) -> Vec<Lock> {
    state
        .store
        .holders_of(project, &[element_id.to_string()], now_seconds())
        .unwrap_or_default()
}

/// The edit form itself. When another holder has a live lease the submit button is disabled
/// and the holder is named; when the result was invalid the validator's errors are listed.
fn edit_page(
    identity: &Identity,
    mechanism: &str,
    project: &str,
    element_id: &str,
    input: &EditInput,
    holders: &[Lock],
    errors: &[String],
) -> Markup {
    let blocked: Vec<&Lock> = holders
        .iter()
        .filter(|lock| lock.holder != identity.subject)
        .collect();
    let editable = blocked.is_empty();
    let title = format!("modelwrite — {} — edit {}", project, element_id);
    let body = html! {
        h1 { "Edit element" }
        p class="meta" { code { (element_id) } }
        @if !blocked.is_empty() {
            section class="lock-banner" {
                h2 { "Held by another holder" }
                @for lock in &blocked {
                    p { (lock.element.as_str()) " is held by " strong { (lock.holder.as_str()) } " until " (lock.expires_at) "." }
                }
                p { "Wait for the lease to expire, or ask the holder to release it, then reload." }
            }
        }
        @if !errors.is_empty() {
            section class="form-errors" {
                h2 { "The edit was not stored" }
                ul {
                    @for error in errors { li { (error.as_str()) } }
                }
            }
        }
        form method="post" action={ "/ui/projects/" (crate::ui::urlencode(project)) "/edit/" (crate::ui::urlencode(element_id)) } class="edit-form" {
            p { label for="branch" { "branch" } input type="text" id="branch" name="branch" value=(input.branch.as_str()) required; }
            p { label for="message" { "commit message" } input type="text" id="message" name="message" value=(input.message.as_str()) required; }
            p { label for="id" { "id" } input type="text" id="id" name="id" value=(input.id.as_str()); }
            p { label for="name" { "name" } input type="text" id="name" name="name" value=(input.name.as_str()); }
            p {
                label for="documentation" { "documentation" }
                textarea id="documentation" name="documentation" { (input.documentation.as_str()) }
            }
            p { label for="stereotypes" { "stereotypes (comma-separated)" } input type="text" id="stereotypes" name="stereotypes" value=(input.stereotypes.as_str()); }
            fieldset {
                legend { "Attributes" }
                input type="hidden" name="attr_count" value=(input.attributes.len());
                @for (index, attribute) in input.attributes.iter().enumerate() {
                    (attribute_row(index, attribute))
                }
            }
            @if editable {
                button type="submit" { "Commit" }
            } @else {
                button type="submit" disabled { "Commit" }
            }
        }
    };
    layout::shell(
        &title,
        Some(project),
        Some(&identity.subject),
        mechanism,
        body,
    )
}

/// One attribute's four fields, indexed so the submitted form carries them unambiguously.
fn attribute_row(index: usize, attribute: &Attribute) -> Markup {
    html! {
        div class="attribute-row" {
            input type="text" name=(format!("attr_name_{}", index)) value=(attribute.name.as_str()) placeholder="name";
            input type="text" name=(format!("attr_type_{}", index)) value=(attribute.attr_type.as_str()) placeholder="type";
            input type="text" name=(format!("attr_aggregation_{}", index)) value=(attribute.aggregation.as_str()) placeholder="aggregation";
            input type="text" name=(format!("attr_default_{}", index)) value=(attribute.default.as_str()) placeholder="default";
        }
    }
}

fn edit_success_page(
    identity: &Identity,
    mechanism: &str,
    project: &str,
    commit: &Commit,
) -> Markup {
    let title = format!("modelwrite — {} — committed", project);
    let body = html! {
        h1 { "Edit committed" }
        p { "Commit " code { (short_hash(&commit.hash)) } }
        p { "author " strong { (commit.author.as_str()) } }
        p {
            a href={ "/ui/projects/" (crate::ui::urlencode(project)) "/model?commit=" (commit.hash.as_str()) } {
                "View the edited model"
            }
        }
    };
    layout::shell(
        &title,
        Some(project),
        Some(&identity.subject),
        mechanism,
        body,
    )
}

/// The parsed attributes from the submitted form: every indexed row whose name is non-empty.
/// An emptied name drops the attribute rather than storing a nameless row.
fn parse_attributes(form: &HashMap<String, String>) -> Vec<Attribute> {
    let count = form
        .get("attr_count")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);
    let mut attributes = Vec::new();
    for index in 0..count {
        let name = form
            .get(&format!("attr_name_{}", index))
            .cloned()
            .unwrap_or_default();
        if name.trim().is_empty() {
            continue;
        }
        let attr_type = form
            .get(&format!("attr_type_{}", index))
            .cloned()
            .unwrap_or_default();
        let aggregation = form
            .get(&format!("attr_aggregation_{}", index))
            .cloned()
            .unwrap_or_default();
        let default = form
            .get(&format!("attr_default_{}", index))
            .cloned()
            .unwrap_or_default();
        attributes.push(Attribute {
            name,
            attr_type,
            aggregation,
            default,
        });
    }
    attributes
}

/// Split a comma-separated stereotype list, trimming blanks, so an empty field round-trips to
/// no stereotypes and `Block, Signal` round-trips to two.
fn split_stereotypes(text: &str) -> Vec<String> {
    text.split(',')
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(String::from)
        .collect()
}

fn apply_edit(element: &mut Element, input: &EditInput) {
    element.id = input.id.clone();
    element.name = input.name.clone();
    element.documentation = input.documentation.clone();
    element.stereotypes = split_stereotypes(&input.stereotypes);
    element.attributes = input.attributes.clone();
}

/// Read-only lookup of an element across the three lists that hold the `Element` type.
fn element_in<'a>(root: &'a OkfRoot, id: &str) -> Option<&'a Element> {
    root.structure
        .iter()
        .chain(root.interfaces.iter())
        .chain(root.signals.iter())
        .find(|element| element.id == id)
}

/// Mutable lookup of an element, for applying the submitted edit.
fn find_element<'a>(root: &'a mut OkfRoot, id: &str) -> Option<&'a mut Element> {
    if let Some(element) = root.structure.iter_mut().find(|element| element.id == id) {
        return Some(element);
    }
    if let Some(element) = root.interfaces.iter_mut().find(|element| element.id == id) {
        return Some(element);
    }
    root.signals.iter_mut().find(|element| element.id == id)
}

fn short_hash(hash: &str) -> &str {
    hash.get(..8).unwrap_or(hash)
}
