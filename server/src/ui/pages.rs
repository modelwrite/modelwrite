// SPDX-License-Identifier: AGPL-3.0-or-later
//! The workbench pages: the project list and the per-project branch list. Each page is a
//! read-only view over the store, guarded by the SAME identity resolution, `Read`
//! permission and project-scope checks as the JSON handlers, so the workbench can never
//! be a second implementation with weaker rules.

use std::collections::HashMap;

use axum::extract::{Form, Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use maud::{html, Markup};

use crate::api::{
    create_branch_core, create_project_core, load_model, map_store_error, validate_name, ApiState,
};
use crate::auth::{identity as resolve_identity, Identity, Permission};
use crate::error::ApiError;
use crate::store::{Commit, Store};
use crate::ui::layout;
use crate::ui::review::merge_form_markup;

struct ProjectRow {
    name: String,
    branch_count: usize,
    latest: Option<Commit>,
    blocks: Option<u64>,
    requirements: Option<u64>,
}

struct BranchRow {
    name: String,
    tip: String,
    commit: Option<Commit>,
}

/// `GET /ui` - the project list: one entry per project the caller may reach, with its
/// branch count and its latest commit.
pub async fn project_list(State(state): State<ApiState>, headers: HeaderMap) -> Response {
    let mechanism = state.auth.mechanism();
    let identity = match resolve_identity(&state, &headers).await {
        Ok(identity) => identity,
        Err(error) => return layout::sign_in_page(mechanism, &error.message),
    };
    match render_project_list(&state, &identity, None) {
        Ok(page) => layout::html_response(StatusCode::OK, page),
        Err(error) => layout::error_page(
            error.status,
            Some(&identity.subject),
            mechanism,
            &error.message,
        ),
    }
}

/// `POST /ui` - create a project and land the caller on it. The SAME permission, scope and
/// validation decisions as the JSON handler, through the SAME shared core, so the page and
/// the endpoint cannot disagree about what a create did.
pub async fn create_project(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    let mechanism = state.auth.mechanism();
    let identity = match resolve_identity(&state, &headers).await {
        Ok(identity) => identity,
        Err(error) => return layout::sign_in_page(mechanism, &error.message),
    };
    // Permissions first, in the same order as the JSON handler.
    if !identity.may(Permission::Administer) {
        return layout::error_page(
            StatusCode::FORBIDDEN,
            Some(&identity.subject),
            mechanism,
            "admin permission required",
        );
    }
    let name = form.get("name").cloned().unwrap_or_default();
    if !identity.may_reach(&name) {
        return layout::error_page(
            StatusCode::FORBIDDEN,
            Some(&identity.subject),
            mechanism,
            "project not in scope",
        );
    }
    if let Err(error) = validate_name("project name", &name) {
        return render_project_list_refused(
            &state,
            &identity,
            mechanism,
            StatusCode::UNPROCESSABLE_ENTITY,
            &error.message,
        );
    }
    match create_project_core(
        state.store.as_ref(),
        &identity.subject,
        mechanism,
        state.auth.authorizer().unwrap_or(""),
        &name,
    ) {
        Ok(_) => {
            Redirect::to(&format!("/ui/projects/{}", crate::ui::urlencode(&name))).into_response()
        }
        Err(error) => {
            let api = map_store_error(error);
            render_project_list_refused(&state, &identity, mechanism, api.status, &api.message)
        }
    }
}

/// Re-render the project list with a refusal as information: the caller keeps the page and
/// sees what was wrong, rather than a bare error page.
fn render_project_list_refused(
    state: &ApiState,
    identity: &Identity,
    mechanism: &str,
    status: StatusCode,
    notice: &str,
) -> Response {
    match render_project_list(state, identity, Some(notice)) {
        Ok(page) => layout::html_response(status, page),
        Err(error) => layout::error_page(
            error.status,
            Some(&identity.subject),
            mechanism,
            &error.message,
        ),
    }
}

fn render_project_list(
    state: &ApiState,
    identity: &Identity,
    notice: Option<&str>,
) -> Result<Markup, ApiError> {
    if !identity.may(Permission::Read) {
        return Err(ApiError::forbidden("read permission required"));
    }
    let projects = state.store.list_projects().map_err(map_store_error)?;
    let mut rows = Vec::new();
    for project in projects {
        // A caller scoped to particular projects must not learn the names of the others:
        // the listing is filtered, exactly as the JSON handler filters it.
        if !identity.may_reach(&project.name) {
            continue;
        }
        let branch_count = state
            .store
            .list_branches(&project.name)
            .map_err(map_store_error)?
            .len();
        let latest = latest_commit(state.store.as_ref(), &project.name)?;
        let (blocks, requirements) =
            model_size(state.store.as_ref(), &project.name, latest.as_ref());
        rows.push(ProjectRow {
            name: project.name,
            branch_count,
            latest,
            blocks,
            requirements,
        });
    }
    let nav = layout::Nav::load(state, identity, None)?;
    Ok(project_list_page(
        identity,
        state.auth.mechanism(),
        &rows,
        notice,
        &nav,
    ))
}

fn project_list_page(
    identity: &Identity,
    mechanism: &str,
    rows: &[ProjectRow],
    notice: Option<&str>,
    nav: &layout::Nav,
) -> Markup {
    let body = html! {
        h1 { "Projects" }
        p class="meta" { "Every model is a project; open one to read its sections and gate history." }
        @if let Some(notice) = notice {
            section class="form-errors" {
                h2 { "The project was not created" }
                p { (notice) }
            }
        }
        @if rows.is_empty() {
            p { "No projects yet. Create one below." }
        } @else {
            ul class="projects" {
                @for row in rows {
                    li {
                        a class="project-card" href={ "/ui/projects/" (crate::ui::urlencode(row.name.as_str())) } {
                            span class="project-name" { (row.name.as_str()) }
                            span class="project-meta" {
                                (row.branch_count) " "
                                @if row.branch_count == 1 { "branch" } @else { "branches" }
                                @if let (Some(blocks), Some(requirements)) = (row.blocks, row.requirements) {
                                    span class="dot" { "·" }
                                    (blocks) " blocks"
                                    span class="dot" { "·" }
                                    (requirements) " requirements"
                                }
                            }
                            @if let Some(latest) = &row.latest {
                                span class="project-message" { (latest.message.as_str()) }
                                span class="project-author" { "by " (latest.author.as_str()) }
                            }
                            span class="project-open" { "Open →" }
                        }
                    }
                }
            }
        }
        (create_project_form(identity))
    };
    layout::shell_with_main_class(
        "modelwrite — projects",
        nav,
        Some(&identity.subject),
        identity.may(Permission::Administer),
        mechanism,
        "mw-wide",
        body,
    )
}

/// The create-project form, offered only to a caller who may administer: a form the caller
/// cannot submit is a trap, and the refusal is still enforced server-side for a direct request.
fn create_project_form(identity: &Identity) -> Markup {
    if !identity.may(Permission::Administer) {
        return Markup::default();
    }
    html! {
        section class="create-project" {
            h2 { "New project" }
            form method="post" action="/ui" class="create-form" {
                p {
                    label for="project-name" { "Project name" }
                    input type="text" id="project-name" name="name" required;
                }
                button type="submit" { "Create project" }
            }
        }
    }
}

/// `GET /ui/projects/:project` - the project's branches, each with its tip and the tip's
/// message and author.
pub async fn project_page(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(project): Path<String>,
) -> Response {
    let mechanism = state.auth.mechanism();
    let identity = match resolve_identity(&state, &headers).await {
        Ok(identity) => identity,
        Err(error) => return layout::sign_in_page(mechanism, &error.message),
    };
    match render_project_page(&state, &identity, &project, None) {
        Ok(page) => layout::html_response(StatusCode::OK, page),
        Err(error) => layout::error_page(
            error.status,
            Some(&identity.subject),
            mechanism,
            &error.message,
        ),
    }
}

fn render_project_page(
    state: &ApiState,
    identity: &Identity,
    project: &str,
    notice: Option<&str>,
) -> Result<Markup, ApiError> {
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
    let branches = state
        .store
        .list_branches(project)
        .map_err(map_store_error)?;
    let mut rows = Vec::new();
    for (name, tip) in branches {
        let commit = state.store.commit(project, &tip).map_err(map_store_error)?;
        rows.push(BranchRow { name, tip, commit });
    }
    let mut nav = layout::Nav::load(state, identity, Some(project))?;
    nav.section = Some("changes");
    Ok(branch_list_page(
        identity,
        state.auth.mechanism(),
        project,
        &rows,
        notice,
        &nav,
    ))
}

fn branch_list_page(
    identity: &Identity,
    mechanism: &str,
    project: &str,
    rows: &[BranchRow],
    notice: Option<&str>,
    nav: &layout::Nav,
) -> Markup {
    let title = format!("modelwrite — {}", project);
    let body = html! {
        h1 { "Changes" }
        p class="meta" { "Every version is a branch; its identity is its tip commit." }
        @if let Some(notice) = notice {
            section class="form-errors" {
                h2 { "The branch was not created" }
                p { (notice) }
            }
        }
        @if rows.is_empty() {
            p { "This project has no branches yet." }
            @if identity.may(Permission::Write) {
                p {
                    a href={ "/ui/projects/" (crate::ui::urlencode(project)) "/model" } { "Start a model" }
                }
            }
        } @else {
            ul class="branches" {
                @for row in rows {
                    li {
                        a class="branch-name" href={ "/ui/projects/" (crate::ui::urlencode(project)) "/model?branch=" (crate::ui::urlencode(row.name.as_str())) } { (row.name.as_str()) }
                        code class="tip" { (short_hash(&row.tip)) }
                        @if let Some(commit) = &row.commit {
                            span class="message" { (commit.message.as_str()) }
                            span class="meta" { "by " (commit.author.as_str()) }
                        } @else {
                            span class="meta" { "tip commit unavailable" }
                        }
                    }
                }
            }
        }
        // The Changes section holds ONLY its own subject: the version history, a compare of
        // two versions, and creating a version. Proposals, Checks and Import each have their
        // own section in the navigator, so their entries live there rather than duplicated here.
        // Comparing READS two commits, so it needs only read permission - the same permission
        // the compare route itself checks. Gating it behind write would hide a read-only
        // reviewer's most useful tool, and saying so would have been untrue.
        @if identity.may(Permission::Read) {
            h2 { "Compare" }
            form method="get" action={ "/ui/projects/" (crate::ui::urlencode(project)) "/compare" } class="compare-form" {
                p {
                    label for="from" { "from" }
                    input type="text" id="from" name="from" placeholder="branch or commit";
                }
                p {
                    label for="to" { "to" }
                    input type="text" id="to" name="to" placeholder="branch or commit";
                }
                button type="submit" { "Compare" }
            }
        }
        // Creating a branch WRITES and needs an existing commit to start from, so the form is
        // offered only to a caller who may write AND only once there is a commit.
        @if identity.may(Permission::Write) {
            (create_branch_form(project, rows))
        }
        // Merging WRITES, so the form is offered only to a caller who may perform it. A form
        // the caller cannot submit is a trap: it invites an action, then refuses it after the
        // work is typed. The refusal is still enforced server-side for a direct request.
        @if identity.may(Permission::Write) {
            h2 { "Merge" }
            (merge_form_markup(project, "", ""))
        }
    };
    layout::shell(
        &title,
        nav,
        Some(&identity.subject),
        identity.may(Permission::Administer),
        mechanism,
        body,
    )
}

/// The create-branch form. A branch starts from an existing commit, so the form offers each
/// branch's tip as a choice; it renders nothing when there is no commit to branch from.
fn create_branch_form(project: &str, rows: &[BranchRow]) -> Markup {
    if rows.is_empty() {
        return Markup::default();
    }
    html! {
        section class="create-branch" {
            h2 { "New branch" }
            form method="post" action={ "/ui/projects/" (crate::ui::urlencode(project)) "/branch" } class="create-form" {
                p {
                    label for="branch-name" { "Branch name" }
                    input type="text" id="branch-name" name="name" required;
                }
                p {
                    label for="branch-from" { "Branch from" }
                    select id="branch-from" name="from" {
                        @for row in rows {
                            option value=(row.tip.as_str()) { (row.name.as_str()) " @ " (short_hash(&row.tip)) }
                        }
                    }
                }
                button type="submit" { "Create branch" }
            }
        }
    }
}

/// `POST /ui/projects/:project/branch` - create a branch and land the caller back on the
/// project page. The SAME permission, scope and validation decisions as the JSON handler,
/// through the SAME shared core.
pub async fn create_branch(
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
    // Permissions first, in the same order as the JSON handler.
    if !identity.may(Permission::Write) {
        return layout::error_page(
            StatusCode::FORBIDDEN,
            Some(&identity.subject),
            mechanism,
            "write permission required",
        );
    }
    if !identity.may_reach(&project) {
        return layout::error_page(
            StatusCode::FORBIDDEN,
            Some(&identity.subject),
            mechanism,
            "project not in scope",
        );
    }
    let name = form.get("name").cloned().unwrap_or_default();
    let from = form.get("from").cloned().unwrap_or_default();
    if let Err(error) = validate_name("branch name", &name) {
        return render_project_page_refused(
            &state,
            &identity,
            mechanism,
            &project,
            StatusCode::UNPROCESSABLE_ENTITY,
            &error.message,
        );
    }
    match create_branch_core(
        state.store.as_ref(),
        &project,
        &identity.subject,
        mechanism,
        state.auth.authorizer().unwrap_or(""),
        &name,
        &from,
    ) {
        // Land the caller ON the new version, with the version in the address.
        Ok(()) => Redirect::to(&format!(
            "/ui/projects/{}/overview?branch={}",
            crate::ui::urlencode(&project),
            crate::ui::urlencode(&name)
        ))
        .into_response(),
        Err(error) => {
            let api = map_store_error(error);
            render_project_page_refused(
                &state,
                &identity,
                mechanism,
                &project,
                api.status,
                &api.message,
            )
        }
    }
}

/// Re-render the project page with a refusal as information, so a refused branch create keeps
/// the caller on the page and says what was wrong rather than showing a bare error page.
fn render_project_page_refused(
    state: &ApiState,
    identity: &Identity,
    mechanism: &str,
    project: &str,
    status: StatusCode,
    notice: &str,
) -> Response {
    match render_project_page(state, identity, project, Some(notice)) {
        Ok(page) => layout::html_response(status, page),
        Err(error) => layout::error_page(
            error.status,
            Some(&identity.subject),
            mechanism,
            &error.message,
        ),
    }
}

/// The latest commit in a project: the most recently created commit among the branch
/// tips. The store exposes commits by branch and by hash, so the newest tip is the newest
/// commit the workbench can reach without adding a store method.
fn latest_commit(store: &dyn Store, project: &str) -> Result<Option<Commit>, ApiError> {
    let branches = store.list_branches(project).map_err(map_store_error)?;
    let mut latest: Option<Commit> = None;
    for (_name, tip) in branches {
        if let Some(commit) = store.commit(project, &tip).map_err(map_store_error)? {
            let is_newest = match &latest {
                None => true,
                Some(current) => created_at(&commit) > created_at(current),
            };
            if is_newest {
                latest = Some(commit);
            }
        }
    }
    Ok(latest)
}

fn created_at(commit: &Commit) -> i64 {
    commit.created_at.parse::<i64>().unwrap_or(0)
}

/// The model's declared size at the latest commit: blocks and requirements, read from the
/// model's own summary. It is computed from the branch tip on every request and cached
/// nowhere; a missing or unreadable model reports no size rather than failing the list.
fn model_size(
    store: &dyn Store,
    project: &str,
    latest: Option<&Commit>,
) -> (Option<u64>, Option<u64>) {
    let Some(commit) = latest else {
        return (None, None);
    };
    match load_model(store, project, &commit.hash) {
        Ok(root) => (Some(root.summary.blocks), Some(root.summary.requirements)),
        Err(_) => (None, None),
    }
}

fn short_hash(hash: &str) -> &str {
    hash.get(..8).unwrap_or(hash)
}
