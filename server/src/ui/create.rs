// SPDX-License-Identifier: AGPL-3.0-or-later
//! The creation pages: where a person brings a model and its elements into existence.
//!
//! A project is created from the project list, a model is created in an empty project
//! (either from a minimal empty document or from a pasted OKF document), and an element -
//! a block or a requirement - is added to an existing model. Every one of these is a CLIENT
//! of the existing write path, never a second implementation: the model commits run through
//! the SAME [crate::api::commit_core] the JSON commit handler uses, with the same permission,
//! scope and author decisions in the same order, so validation, locking and the audit entry
//! behave identically to a direct API commit.
//!
//! A refusal is information with a way forward, never an error page: an invalid pasted
//! document or a missing requirement number re-renders the form with the validator's errors
//! and preserves what was typed.

use std::collections::HashMap;

use axum::extract::{Form, Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use maud::{html, Markup};

use okf::types::{Element, Graph, GraphNode, OkfRoot, Requirement, StateMachine, Summary};

use crate::api::{
    commit_core, load_model, map_store_error, validate_element_name, validate_name, ApiState,
    CommitCore, CommitFailure,
};
use crate::auth::{identity as resolve_identity, Identity, Permission};
use crate::error::ApiError;
use crate::store::{now_seconds, Commit};
use crate::ui::layout;

/// The branch a first model lands on, matching the model page's default.
const DEFAULT_BRANCH: &str = "main";

// ---------------------------------------------------------------------------
// Create a model (start from empty, or paste an OKF document).

/// The minimal OKF document a "start from empty" first commit stores. The validator requires
/// a project name, a graph section with at least one node and a state machine, so an empty
/// model cannot be literally zero bytes: it is zero STRUCTURE and zero REQUIREMENTS with the
/// one graph node and the state machine the validator demands. The node is invisible to the
/// page (it backs no element), but it is what makes the empty document valid.
fn empty_okf(project: &str) -> OkfRoot {
    OkfRoot {
        okf: "1.0".to_string(),
        project: project.to_string(),
        exported_at: String::new(),
        summary: Summary {
            blocks: 0,
            requirements: 0,
            interfaces: 0,
            signals: 0,
            activities: 0,
            graph_nodes: 1,
            graph_edges: 0,
        },
        structure: Vec::new(),
        interfaces: Vec::new(),
        signals: Vec::new(),
        requirements: Vec::new(),
        state_machine: Some(StateMachine {
            name: "sm".to_string(),
            regions: Vec::new(),
        }),
        activities: Vec::new(),
        graph: Some(Graph {
            nodes: vec![GraphNode {
                id: "root".to_string(),
                kind: "block".to_string(),
                name: "root".to_string(),
                stereotypes: Vec::new(),
            }],
            edges: Vec::new(),
        }),
        provenance: None,
        references: Vec::new(),
    }
}

/// What a submitted model creation produced. A refusal is a RESULT, not an error: it carries
/// the validator's errors back to the form, never a 500.
enum CreateModelOutcome {
    Committed { commit: Box<Commit> },
    Refused { errors: Vec<String> },
}

/// POST /ui/projects/:project/model/new - create the project's first model and land the
/// caller on it. The author is the verified identity, never a field the browser supplies.
pub async fn create_model(
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
    match perform_create_model(&state, &identity, &project, &form) {
        Ok(CreateModelOutcome::Committed { commit }) => Redirect::to(&format!(
            "/ui/projects/{}/model?commit={}",
            crate::ui::urlencode(&project),
            commit.hash
        ))
        .into_response(),
        Ok(CreateModelOutcome::Refused { errors }) => layout::html_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            empty_model_page(&identity, mechanism, &project, &errors),
        ),
        Err(error) => layout::error_page(
            error.status,
            Some(&identity.subject),
            mechanism,
            &error.message,
        ),
    }
}

fn perform_create_model(
    state: &ApiState,
    identity: &Identity,
    project: &str,
    form: &HashMap<String, String>,
) -> Result<CreateModelOutcome, ApiError> {
    // The SAME identity, permission and project-scope decisions as the commit handler, in the
    // SAME order.
    if !identity.may(Permission::Write) {
        return Err(ApiError::forbidden("write permission required"));
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
    let branch = form
        .get("branch")
        .cloned()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_BRANCH.to_string());
    validate_name("branch name", &branch)?;
    let message = form.get("message").cloned().unwrap_or_default();
    if message.trim().is_empty() {
        return Err(ApiError::bad_request("commit message must not be empty"));
    }
    let mode = form.get("mode").cloned().unwrap_or_default();
    let root: OkfRoot = match mode.as_str() {
        "empty" => empty_okf(project),
        "paste" => {
            let text = form.get("okf").cloned().unwrap_or_default();
            if text.trim().is_empty() {
                return Ok(CreateModelOutcome::Refused {
                    errors: vec!["paste an OKF document".to_string()],
                });
            }
            match serde_json::from_str::<OkfRoot>(&text) {
                Ok(root) => root,
                Err(error) => {
                    return Ok(CreateModelOutcome::Refused {
                        errors: vec![format!("the pasted document is not valid JSON: {}", error)],
                    })
                }
            }
        }
        _ => {
            return Ok(CreateModelOutcome::Refused {
                errors: vec!["choose to start from empty or paste a document".to_string()],
            })
        }
    };
    // Validation BEFORE anything is stored, so a refused model leaves no blob behind. The
    // shared commit core re-validates, but rendering the errors next to the form is the
    // refusal-as-information this page promises.
    let report = okf::validate::validate(&root);
    if !report.valid {
        return Ok(CreateModelOutcome::Refused {
            errors: report.errors,
        });
    }
    // Creating a model is a FIRST commit: the branch must not already hold one, or the "start
    // one" door would silently append onto an existing model.
    if state
        .store
        .branch_tip(project, &branch)
        .map_err(map_store_error)?
        .is_some()
    {
        return Err(ApiError::conflict(format!(
            "branch {} already has a model",
            branch
        )));
    }
    let author = identity.subject.clone();
    let bytes = serde_json::to_vec(&root).map_err(|error| {
        eprintln!("model could not be serialised: {}", error);
        ApiError::internal("the model could not be stored")
    })?;
    let now = now_seconds();
    let commit = match commit_core(
        state.store.as_ref(),
        &CommitCore {
            project,
            branch: &branch,
            author: &author,
            message: &message,
            actor: &identity.subject,
            mechanism: state.auth.mechanism(),
            authorizer: state.auth.authorizer().unwrap_or(""),
            candidate: &root,
            bytes: &bytes,
            import: None,
            acceptance: None,
            holder: "",
            now,
            tip: None,
            reference: None,
        },
    ) {
        Ok(commit) => commit,
        Err(CommitFailure::Invalid { errors }) => {
            return Ok(CreateModelOutcome::Refused { errors })
        }
        Err(CommitFailure::Store(error)) => return Err(map_store_error(error)),
    };
    Ok(CreateModelOutcome::Committed {
        commit: Box::new(commit),
    })
}

/// The empty model page: the message the rules require - what to do next, with the action
/// right there - plus the two creation forms and the import link. It is shared by the model
/// page (no errors) and the refused model-creation submit (with the validator's errors).
pub fn empty_model_page(
    identity: &Identity,
    mechanism: &str,
    project: &str,
    errors: &[String],
) -> Markup {
    let can_write = identity.may(Permission::Write);
    let body = html! {
        h1 { "Model" }
        p class="empty-state" {
            "This project has no model yet — start one or import a legacy model."
        }
        @if !errors.is_empty() {
            section class="form-errors" {
                h2 { "The model was not created" }
                ul { @for error in errors { li { (error.as_str()) } } }
            }
        }
        @if can_write {
            (create_model_forms(project, can_write))
            h2 { "Import a legacy model" }
            p {
                a href={ "/ui/projects/" (crate::ui::urlencode(project)) "/import" } { "Import a legacy model" }
            }
        } @else {
            p { "Ask a writer to start the model." }
        }
    };
    let title = format!("modelwrite — {}", project);
    layout::shell(
        &title,
        Some(project),
        Some(&identity.subject),
        mechanism,
        body,
    )
}

/// The two routes to a first model: start from empty, or paste an OKF document. Both post to
/// the SAME route and are distinguished by the hidden mode field, so the handler cannot
/// diverge from the forms.
pub fn create_model_forms(project: &str, can_write: bool) -> Markup {
    html! {
        section class="create-model" {
            h2 { "Start a model" }
            form method="post" action={ "/ui/projects/" (crate::ui::urlencode(project)) "/model/new" } class="create-form" {
                input type="hidden" name="mode" value="empty";
                input type="hidden" name="branch" value="main";
                p {
                    label for="empty-message" { "Commit message" }
                    input type="text" id="empty-message" name="message" value="start the model" required;
                }
                @if can_write { button type="submit" { "Start from empty" } }
                @else { button type="submit" disabled { "Start from empty" } }
            }
            form method="post" action={ "/ui/projects/" (crate::ui::urlencode(project)) "/model/new" } class="create-form" {
                input type="hidden" name="mode" value="paste";
                input type="hidden" name="branch" value="main";
                p {
                    label for="paste-message" { "Commit message" }
                    input type="text" id="paste-message" name="message" required;
                }
                p {
                    label for="okf" { "OKF document (paste JSON)" }
                    textarea id="okf" name="okf" required;
                }
                @if can_write { button type="submit" { "Create from pasted document" } }
                @else { button type="submit" disabled { "Create from pasted document" } }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Add an element (a block or a requirement).

/// The query parameters of the add-element form: the branch the element lands on, defaulting
/// to main exactly as the model page resolves it.
#[derive(serde::Deserialize)]
pub struct ElementQuery {
    pub branch: Option<String>,
}

/// The values the add-element form carries. The author is the verified identity, never a
/// browser field; the branch travels as a hidden field from the URL the form was loaded from.
struct ElementInput {
    kind: String,
    id: String,
    name: String,
    req_id: String,
    text: String,
    branch: String,
    message: String,
}

impl ElementInput {
    fn from_form(form: &HashMap<String, String>) -> Self {
        let kind = form.get("kind").cloned().unwrap_or_default();
        let id = form.get("id").cloned().unwrap_or_default();
        let name = form.get("name").cloned().unwrap_or_default();
        let req_id = form.get("req_id").cloned().unwrap_or_default();
        let text = form.get("text").cloned().unwrap_or_default();
        let branch = form
            .get("branch")
            .cloned()
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| DEFAULT_BRANCH.to_string());
        let message = form.get("message").cloned().unwrap_or_default();
        Self {
            kind,
            id,
            name,
            req_id,
            text,
            branch,
            message,
        }
    }
}

/// What a submitted element addition produced. A refusal is a RESULT, not an error: it carries
/// the validator's errors (a duplicate id, a missing requirement number) back to the form.
enum CreateElementOutcome {
    Committed {
        commit: Box<Commit>,
    },
    Refused {
        input: ElementInput,
        errors: Vec<String>,
    },
}

/// GET /ui/projects/:project/element/new - the form for adding a block or a requirement.
pub async fn element_form(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(project): Path<String>,
    axum::extract::Query(query): axum::extract::Query<ElementQuery>,
) -> Response {
    let mechanism = state.auth.mechanism();
    let identity = match resolve_identity(&state, &headers).await {
        Ok(identity) => identity,
        Err(error) => return layout::sign_in_page(mechanism, &error.message),
    };
    let branch = query
        .branch
        .clone()
        .filter(|branch| !branch.is_empty())
        .unwrap_or_else(|| DEFAULT_BRANCH.to_string());
    match render_element_form(&state, &identity, &project, &branch) {
        Ok(page) => layout::html_response(StatusCode::OK, page),
        Err(error) => layout::error_page(
            error.status,
            Some(&identity.subject),
            mechanism,
            &error.message,
        ),
    }
}

fn render_element_form(
    state: &ApiState,
    identity: &Identity,
    project: &str,
    branch: &str,
) -> Result<Markup, ApiError> {
    // Viewing the form needs read; submitting is refused unless the caller may write, exactly
    // as the edit form does.
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
    validate_name("branch name", branch)?;
    if state
        .store
        .branch_tip(project, branch)
        .map_err(map_store_error)?
        .is_none()
    {
        return Err(ApiError::not_found(format!(
            "branch {} has no commits — start a model first",
            branch
        )));
    }
    let input = ElementInput {
        kind: "block".to_string(),
        id: String::new(),
        name: String::new(),
        req_id: String::new(),
        text: String::new(),
        branch: branch.to_string(),
        message: String::new(),
    };
    Ok(element_page(
        identity,
        state.auth.mechanism(),
        project,
        &input,
        &[],
    ))
}

/// POST /ui/projects/:project/element/new - add the element and commit it through the SAME
/// core as the JSON commit handler. The author is the verified identity; validation runs
/// before anything is stored, and an invalid result renders its errors next to the form.
pub async fn create_element(
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
    match perform_create_element(&state, &identity, &project, &form) {
        Ok(CreateElementOutcome::Committed { commit }) => Redirect::to(&format!(
            "/ui/projects/{}/model?commit={}",
            crate::ui::urlencode(&project),
            commit.hash
        ))
        .into_response(),
        Ok(CreateElementOutcome::Refused { input, errors }) => layout::html_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            element_page(&identity, mechanism, &project, &input, &errors),
        ),
        Err(error) => layout::error_page(
            error.status,
            Some(&identity.subject),
            mechanism,
            &error.message,
        ),
    }
}

fn perform_create_element(
    state: &ApiState,
    identity: &Identity,
    project: &str,
    form: &HashMap<String, String>,
) -> Result<CreateElementOutcome, ApiError> {
    if !identity.may(Permission::Write) {
        return Err(ApiError::forbidden("write permission required"));
    }
    if !identity.may_reach(project) {
        return Err(ApiError::forbidden("project not in scope"));
    }
    let input = ElementInput::from_form(form);
    if let Err(error) = validate_element_name(&input.id) {
        return Ok(CreateElementOutcome::Refused {
            input,
            errors: vec![error.message],
        });
    }
    if let Err(error) = validate_name("branch name", &input.branch) {
        return Ok(CreateElementOutcome::Refused {
            input,
            errors: vec![error.message],
        });
    }
    if input.message.trim().is_empty() {
        return Ok(CreateElementOutcome::Refused {
            input,
            errors: vec!["commit message must not be empty".to_string()],
        });
    }
    if input.kind != "block" && input.kind != "requirement" {
        return Ok(CreateElementOutcome::Refused {
            input,
            errors: vec!["kind must be block or requirement".to_string()],
        });
    }
    if input.kind == "requirement" && input.req_id.trim().is_empty() {
        return Ok(CreateElementOutcome::Refused {
            input,
            errors: vec!["a requirement needs a reqId".to_string()],
        });
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
    let candidate = add_element(&root, &input);

    // The document must still be a valid OKF model. A duplicate id, or a requirement without
    // a reqId that slipped past the field check, renders its errors next to the form and
    // stores nothing.
    let report = okf::validate::validate(&candidate);
    if !report.valid {
        return Ok(CreateElementOutcome::Refused {
            input,
            errors: report.errors,
        });
    }

    let author = identity.subject.clone();
    let bytes = serde_json::to_vec(&candidate).map_err(|error| {
        eprintln!("model could not be serialised: {}", error);
        ApiError::internal("the model could not be stored")
    })?;
    let now = now_seconds();
    let commit = match commit_core(
        state.store.as_ref(),
        &CommitCore {
            project,
            branch: &input.branch,
            author: &author,
            message: &input.message,
            actor: &identity.subject,
            mechanism: state.auth.mechanism(),
            authorizer: state.auth.authorizer().unwrap_or(""),
            candidate: &candidate,
            bytes: &bytes,
            import: None,
            acceptance: None,
            holder: "",
            now,
            tip: Some(&tip),
            reference: Some(&root),
        },
    ) {
        Ok(commit) => commit,
        Err(CommitFailure::Invalid { errors }) => {
            return Ok(CreateElementOutcome::Refused { input, errors })
        }
        Err(CommitFailure::Store(error)) => return Err(map_store_error(error)),
    };
    Ok(CreateElementOutcome::Committed {
        commit: Box::new(commit),
    })
}

/// Add the submitted element to a clone of the current model, and keep the summary counts in
/// step so the document the page renders stays self-consistent. A block lands in the
/// structure list with a matching graph node; a requirement lands in the requirements list
/// with its reqId and text and a matching graph node.
fn add_element(root: &OkfRoot, input: &ElementInput) -> OkfRoot {
    let mut candidate = root.clone();
    match input.kind.as_str() {
        "block" => {
            candidate.structure.push(Element {
                id: input.id.clone(),
                name: input.name.clone(),
                kind: "block".to_string(),
                stereotypes: Vec::new(),
                attributes: Vec::new(),
                documentation: String::new(),
            });
            candidate.summary.blocks += 1;
            if let Some(graph) = candidate.graph.as_mut() {
                graph.nodes.push(GraphNode {
                    id: input.id.clone(),
                    kind: "block".to_string(),
                    name: input.name.clone(),
                    stereotypes: Vec::new(),
                });
                candidate.summary.graph_nodes += 1;
            }
        }
        "requirement" => {
            candidate.requirements.push(Requirement {
                id: input.id.clone(),
                name: input.name.clone(),
                kind: "requirement".to_string(),
                stereotypes: Vec::new(),
                attributes: Vec::new(),
                documentation: String::new(),
                req_id: input.req_id.clone(),
                req_text: input.text.clone(),
            });
            candidate.summary.requirements += 1;
            if let Some(graph) = candidate.graph.as_mut() {
                graph.nodes.push(GraphNode {
                    id: input.id.clone(),
                    kind: "requirement".to_string(),
                    name: input.name.clone(),
                    stereotypes: Vec::new(),
                });
                candidate.summary.graph_nodes += 1;
            }
        }
        _ => {}
    }
    candidate
}

/// The add-element form. The submit is offered only to a caller who may write, exactly as the
/// edit and import forms are.
fn element_page(
    identity: &Identity,
    mechanism: &str,
    project: &str,
    input: &ElementInput,
    errors: &[String],
) -> Markup {
    let can_write = identity.may(Permission::Write);
    let title = format!("modelwrite — {} — add element", project);
    let body = html! {
        h1 { "Add element" }
        p class="meta" { "Adding to branch " code { (input.branch.as_str()) } }
        @if !errors.is_empty() {
            section class="form-errors" {
                h2 { "The element was not added" }
                ul { @for error in errors { li { (error.as_str()) } } }
            }
        }
        form method="post" action={ "/ui/projects/" (crate::ui::urlencode(project)) "/element/new" } class="edit-form" {
            input type="hidden" name="branch" value=(input.branch.as_str());
            p {
                label for="kind" { "Kind" }
                select id="kind" name="kind" {
                    option value="block" selected[input.kind == "block"] { "block" }
                    option value="requirement" selected[input.kind == "requirement"] { "requirement" }
                }
            }
            p { label for="id" { "id" } input type="text" id="id" name="id" value=(input.id.as_str()) required; }
            p { label for="name" { "name" } input type="text" id="name" name="name" value=(input.name.as_str()); }
            p { label for="req_id" { "reqId (requirements only)" } input type="text" id="req_id" name="req_id" value=(input.req_id.as_str()); }
            p { label for="text" { "text (requirements only)" } textarea id="text" name="text" { (input.text.as_str()) } }
            p { label for="message" { "commit message" } input type="text" id="message" name="message" value=(input.message.as_str()) required; }
            @if can_write { button type="submit" { "Add element" } }
            @else { button type="submit" disabled { "Add element" } }
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
