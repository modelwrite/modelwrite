// SPDX-License-Identifier: AGPL-3.0-or-later
//! The workbench pages: the project list and the per-project branch list. Each page is a
//! read-only view over the store, guarded by the SAME identity resolution, `Read`
//! permission and project-scope checks as the JSON handlers, so the workbench can never
//! be a second implementation with weaker rules.
//!
//! The project list is the FRONT DOOR, and it answers the question the front door is for:
//! which of these models is broken? Every card carries the NAMED gap counts for its default
//! branch head - orphaned nodes, isolated groups, dangling links and uncovered requirements -
//! computed by the model-health view's own detector ([crate::ui::health::health_summary]),
//! never a second one, and links straight to that view for the exact commit it measured. A
//! project with no commits says "nothing to measure" rather than showing zeros, because
//! "nothing to measure" is not "nothing wrong".

use std::collections::HashMap;

use axum::extract::{Form, Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use maud::{html, Markup};

use crate::api::{
    create_branch_core, create_project_core, load_model, map_store_error, validate_name, ApiState,
};
use crate::auth::{identity as resolve_identity, Identity, Permission};
use crate::binding_registry;
use crate::error::ApiError;
use crate::store::{Commit, Store};
use crate::ui::dropzone;
use crate::ui::health::{health_summary, HealthSummary};
use crate::ui::layout;
use crate::ui::model::default_branch_name;
use crate::ui::review::merge_form_markup;

/// What the front door can honestly say about a project's model health, for its default
/// branch head. "Nothing to measure" and "nothing wrong" are different states, and the card
/// must never let zeros for a project with no model read as a clean one.
enum CardHealth {
    /// The project has no commit on any branch: there is nothing to measure.
    NoCommits,
    /// A branch exists but no readable document could be loaded from its head, so the graph
    /// analysis has no input. Unmeasured, not clean.
    NotMeasurable,
    /// The head commit's document, measured by the health view's own detector.
    Measured(HealthSummary),
}

struct ProjectRow {
    name: String,
    branch_count: usize,
    /// The default branch's head commit: the version the card describes, and the commit its
    /// health summary and its health link both name.
    head: Option<Commit>,
    blocks: Option<u64>,
    requirements: Option<u64>,
    health: CardHealth,
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
        state.store_for(&identity).as_ref(),
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
    let projects = state
        .store_for(identity)
        .list_projects()
        .map_err(map_store_error)?;
    let mut rows = Vec::new();
    for project in projects {
        // A caller scoped to particular projects must not learn the names of the others:
        // the listing is filtered, exactly as the JSON handler filters it.
        if !identity.may_reach(&project.name) {
            continue;
        }
        let branches = state
            .store_for(identity)
            .list_branches(&project.name)
            .map_err(map_store_error)?;
        let branch_count = branches.len();
        // The card describes the DEFAULT BRANCH head - the same commit a bare model or health
        // URL resolves to - so the counts on the card and the page it links to are read from
        // one version. The branch read is reused rather than repeated: this runs per project
        // on every page load.
        let head = default_head(state.store_for(identity).as_ref(), &project.name, &branches)?;
        // No branches at all is "nothing to measure"; a branch whose head cannot be read is
        // also unmeasured, never clean.
        let mut health = if branches.is_empty() {
            CardHealth::NoCommits
        } else {
            CardHealth::NotMeasurable
        };
        let (blocks, requirements) = match &head {
            None => (None, None),
            Some(commit) => match load_model(
                state.store_for(identity).as_ref(),
                &project.name,
                &commit.hash,
            ) {
                Ok(root) => {
                    health = health_summary(&root)
                        .map_or(CardHealth::NotMeasurable, CardHealth::Measured);
                    (Some(root.summary.blocks), Some(root.summary.requirements))
                }
                Err(_) => (None, None),
            },
        };
        rows.push(ProjectRow {
            name: project.name,
            branch_count,
            head,
            blocks,
            requirements,
            health,
        });
    }
    let nav = layout::Nav::load(state, identity, None)?;
    Ok(project_list_page(
        identity,
        state.auth.mechanism(),
        &rows,
        notice,
        &binding_registry::bindings(),
        &nav,
    ))
}

fn project_list_page(
    identity: &Identity,
    mechanism: &str,
    rows: &[ProjectRow],
    notice: Option<&str>,
    bindings: &[binding::BindingInfo],
    nav: &layout::Nav,
) -> Markup {
    // The drop zone is the FIRST thing on the page: the front door is "bring your model", not
    // "pick from this list". The list, the New project box and the expert binding dropdown all
    // stay, below it, for the person who already knows what they want.
    let body = html! {
        (dropzone::dropzone_markup(bindings, identity.may(Permission::Write)))
        h2 { "Projects" }
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
                            @if let Some(head) = &row.head {
                                span class="project-message" { (head.message.as_str()) }
                                span class="project-author" { "by " (head.author.as_str()) }
                            }
                            span class="project-open" { "Open →" }
                        }
                        (health_band(row))
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
        .store_for(identity)
        .project(project)
        .map_err(map_store_error)?
        .is_none()
    {
        return Err(ApiError::not_found(format!("project {}", project)));
    }
    let branches = state
        .store_for(identity)
        .list_branches(project)
        .map_err(map_store_error)?;
    let mut rows = Vec::new();
    for (name, tip) in branches {
        let commit = state
            .store_for(identity)
            .commit(project, &tip)
            .map_err(map_store_error)?;
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
        state.store_for(&identity).as_ref(),
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

/// The default branch's head commit: the branch [default_branch_name] picks, with its tip.
/// `None` when the project has no branches at all; a branch that exists but whose tip the
/// store cannot resolve is reported as unmeasured by the caller, never as clean.
fn default_head(
    store: &dyn Store,
    project: &str,
    branches: &[(String, String)],
) -> Result<Option<Commit>, ApiError> {
    let Some(branch) = default_branch_name(branches) else {
        return Ok(None);
    };
    let Some(tip) = store
        .branch_tip(project, &branch)
        .map_err(map_store_error)?
    else {
        return Ok(None);
    };
    store.commit(project, &tip).map_err(map_store_error)
}

/// The per-card health summary: the NAMED counts the model-health view computes for the
/// default branch head, on the card, so a person with twelve models can triage the list
/// without opening twelve models. A clean model is plainly clean; a model with gaps wears the
/// warn band and a link to the health view that names every one of them. There is no score
/// and no percentage on purpose - this product names gaps, it does not summarise them.
///
/// The counts are also published as `data-mw-*` attributes on the band, so the numbers the
/// card carries are machine-readable and the test that pins them to the health view's own
/// output reads the same values a reviewer sees rather than a coincidence of proximity.
fn health_band(row: &ProjectRow) -> Markup {
    match &row.health {
        CardHealth::NoCommits => html! {
            p class="project-health is-unmeasured" {
                "no commits yet — nothing to measure"
            }
        },
        CardHealth::NotMeasurable => html! {
            p class="project-health is-unmeasured" {
                "no readable model — nothing to measure"
            }
        },
        CardHealth::Measured(summary) if summary.is_clean() => html! {
            p class="project-health is-clean"
                data-mw-orphaned="0"
                data-mw-isolated-groups="0"
                data-mw-isolated-group-nodes="0"
                data-mw-dangling="0"
                data-mw-uncovered="0" {
                span class="health-verdict" { "no gaps" }
                (health_link(row))
            }
        },
        CardHealth::Measured(summary) => {
            // Only the gaps this model HAS are named, in the health view's own order. A zero
            // on a card is noise; the point of the band is the finding, not the form.
            let mut gaps: Vec<Markup> = Vec::new();
            if summary.orphaned > 0 {
                gaps.push(named_count(summary.orphaned, "orphan", "orphans"));
            }
            if summary.isolated_groups > 0 {
                let nodes = if summary.isolated_group_nodes == 1 {
                    "node"
                } else {
                    "nodes"
                };
                gaps.push(html! {
                    (named_count(summary.isolated_groups, "isolated group", "isolated groups"))
                    " (" (summary.isolated_group_nodes) " " (nodes) ")"
                });
            }
            if summary.dangling > 0 {
                gaps.push(named_count(
                    summary.dangling,
                    "dangling link",
                    "dangling links",
                ));
            }
            if summary.uncovered > 0 {
                gaps.push(named_count(
                    summary.uncovered,
                    "uncovered requirement",
                    "uncovered requirements",
                ));
            }
            html! {
                p class="project-health has-gaps"
                    data-mw-orphaned=(summary.orphaned)
                    data-mw-isolated-groups=(summary.isolated_groups)
                    data-mw-isolated-group-nodes=(summary.isolated_group_nodes)
                    data-mw-dangling=(summary.dangling)
                    data-mw-uncovered=(summary.uncovered) {
                    span class="health-label" { "gaps" }
                    // A count and the separator that introduces it are one wrapping
                    // unit: the dot can never be left dangling at a line break.
                    @for (index, gap) in gaps.iter().enumerate() {
                        span class="health-gap" {
                            @if index > 0 { span class="dot" { "·" } }
                            (gap)
                        }
                    }
                    (health_link(row))
                }
            }
        }
    }
}

/// The health view for this card, BY COMMIT HASH: commits are immutable, so the page the
/// reviewer lands on is the page the counts were taken from, even if the branch moves on.
/// A project with nothing measured has no health view to offer and gets no link.
fn health_link(row: &ProjectRow) -> Markup {
    match (&row.health, &row.head) {
        (CardHealth::Measured(_), Some(head)) => html! {
            a class="health-link" href={
                "/ui/projects/" (crate::ui::urlencode(row.name.as_str()))
                "/health?commit=" (head.hash.as_str())
            } { "Health →" }
        },
        _ => Markup::default(),
    }
}

/// A named count with the right plural: "1 orphan", "10 uncovered requirements". The gap is
/// always NAMED, because a bare number is what this product refuses to show.
fn named_count(count: usize, singular: &str, plural: &str) -> Markup {
    html! {
        (count) " "
        @if count == 1 { (singular) } @else { (plural) }
    }
}

fn short_hash(hash: &str) -> &str {
    hash.get(..8).unwrap_or(hash)
}
