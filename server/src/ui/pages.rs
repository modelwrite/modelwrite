// SPDX-License-Identifier: AGPL-3.0-or-later
//! The workbench pages: the project list and the per-project branch list. Each page is a
//! read-only view over the store, guarded by the SAME identity resolution, `Read`
//! permission and project-scope checks as the JSON handlers, so the workbench can never
//! be a second implementation with weaker rules.

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use maud::{html, Markup};

use crate::api::{map_store_error, ApiState};
use crate::auth::{identity as resolve_identity, Identity, Permission};
use crate::error::ApiError;
use crate::store::{Commit, Store};
use crate::ui::layout;
use crate::ui::review::merge_form_markup;

struct ProjectRow {
    name: String,
    branch_count: usize,
    latest: Option<Commit>,
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
    match render_project_list(&state, &identity) {
        Ok(page) => layout::html_response(StatusCode::OK, page),
        Err(error) => layout::error_page(
            error.status,
            Some(&identity.subject),
            mechanism,
            &error.message,
        ),
    }
}

fn render_project_list(state: &ApiState, identity: &Identity) -> Result<Markup, ApiError> {
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
        rows.push(ProjectRow {
            name: project.name,
            branch_count,
            latest,
        });
    }
    Ok(project_list_page(identity, state.auth.mechanism(), &rows))
}

fn project_list_page(identity: &Identity, mechanism: &str, rows: &[ProjectRow]) -> Markup {
    let body = html! {
        h1 { "Projects" }
        @if rows.is_empty() {
            p { "No projects yet. A project created through the API appears here." }
        } @else {
            ul class="projects" {
                @for row in rows {
                    li {
                        a class="project-name" href={ "/ui/projects/" (crate::ui::urlencode(row.name.as_str())) } { (row.name.as_str()) }
                        span class="meta" {
                            (row.branch_count) " "
                            @if row.branch_count == 1 { "branch" } @else { "branches" }
                        }
                        @if let Some(latest) = &row.latest {
                            span class="message" { (latest.message.as_str()) }
                            span class="meta" { (latest.author.as_str()) }
                        }
                    }
                }
            }
        }
    };
    layout::shell(
        "modelwrite — projects",
        None,
        Some(&identity.subject),
        mechanism,
        body,
    )
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
    match render_project_page(&state, &identity, &project) {
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
    Ok(branch_list_page(
        identity,
        state.auth.mechanism(),
        project,
        &rows,
    ))
}

fn branch_list_page(
    identity: &Identity,
    mechanism: &str,
    project: &str,
    rows: &[BranchRow],
) -> Markup {
    let title = format!("modelwrite — {}", project);
    let body = html! {
        h1 { "Branches" }
        @if rows.is_empty() {
            p { "This project has no branches yet." }
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
        @if identity.may(Permission::Write) || identity.may(Permission::Review) {
            h2 { "Gate" }
            p {
                a href={ "/ui/projects/" (crate::ui::urlencode(project)) "/gate" } { "View gate runs" }
            }
        }
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
        // Importing WRITES, so the link is offered only to a caller who may perform it. A
        // read-only reviewer can still open the page by URL to read a loss report, but the
        // person this page exists for is the one deciding what to give up, and that person
        // can write.
        @if identity.may(Permission::Write) {
            h2 { "Import" }
            p {
                a href={ "/ui/projects/" (crate::ui::urlencode(project)) "/import" } { "Import a legacy model" }
            }
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
        Some(project),
        Some(&identity.subject),
        mechanism,
        body,
    )
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

fn short_hash(hash: &str) -> &str {
    hash.get(..8).unwrap_or(hash)
}
