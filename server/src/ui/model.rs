// SPDX-License-Identifier: AGPL-3.0-or-later
//! The model page: four views over one OKF document - the structure tree, the requirements
//! table, the traceability matrix and the state-and-activity view. Every view reads the SAME
//! parsed document the JSON handlers and the gate read (via [crate::api::load_model]), and
//! the traceability numbers come from the graph crate's own [graph::requirement_coverage],
//! so the page can never disagree with the gate about what is covered or what is unresolved.

use std::collections::{HashMap, HashSet};

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use maud::{html, Markup};
use serde::{Deserialize, Serialize};

use graph::{requirement_coverage, CoverageReport};
use okf::types::{Element, GraphEdge, GraphNode, OkfRoot, Requirement};

use crate::api::{load_model, map_store_error, ApiState};
use crate::assist::reasoner_status;
use crate::auth::{identity as resolve_identity, Identity, Permission};
use crate::error::ApiError;
use crate::store::{Commit, GateRun};
use crate::ui::assist::assist_panel_markup;
use crate::ui::layout;

/// The query parameters of the model page. An explicit commit hash wins; otherwise the
/// named branch (defaulting to `main`) resolves to its tip through the same store reads
/// the JSON commit handler performs.
#[derive(Deserialize)]
pub struct ModelQuery {
    pub branch: Option<String>,
    pub commit: Option<String>,
    /// The diagram view: `structure` (default) or `process`.
    #[serde(default)]
    pub view: Option<String>,
    /// The element the enhancement should highlight, read by app.js from the URL; parsed here
    /// only so the canonical version redirect preserves it.
    pub select: Option<String>,
}

/// `GET /ui/projects/:project/model?branch=&commit=` - the model an engineer came to see.
/// The legacy IDE address; it does not canonicalise, so the existing callers that treat it as
/// a plain 200 stay correct.
pub async fn model_page(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(project): Path<String>,
    Query(query): Query<ModelQuery>,
) -> Response {
    let mechanism = state.auth.mechanism();
    let identity = match resolve_identity(&state, &headers).await {
        Ok(identity) => identity,
        Err(error) => return layout::sign_in_page(mechanism, &error.message),
    };
    match render_model_page(&state, &identity, &project, &query) {
        Ok(page) => layout::html_response(StatusCode::OK, page),
        Err(error) => layout::error_page(
            error.status,
            Some(&identity.subject),
            mechanism,
            &error.message,
        ),
    }
}

/// `GET /ui/projects/:project/overview?branch=&commit=` - the Overview section. A bare URL
/// canonicalises to the default branch so the version is always in the address: the page can
/// be bookmarked and shared with no hidden state. The existing query (the diagram view, the
/// ?select= highlight) is preserved.
pub async fn overview_page(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(project): Path<String>,
    Query(query): Query<ModelQuery>,
) -> Response {
    let mechanism = state.auth.mechanism();
    let identity = match resolve_identity(&state, &headers).await {
        Ok(identity) => identity,
        Err(error) => return layout::sign_in_page(mechanism, &error.message),
    };
    if query.branch.as_deref().is_none_or(str::is_empty)
        && query.commit.as_deref().is_none_or(str::is_empty)
    {
        match default_branch(&state, &identity, &project) {
            Ok(Some(branch)) => {
                let mut url = format!(
                    "/ui/projects/{}/overview?branch={}",
                    crate::ui::urlencode(&project),
                    crate::ui::urlencode(&branch)
                );
                if let Some(view) = query.view.as_deref().filter(|value| !value.is_empty()) {
                    url.push_str(&format!("&view={}", crate::ui::urlencode(view)));
                }
                if let Some(select) = query.select.as_deref().filter(|value| !value.is_empty()) {
                    url.push_str(&format!("&select={}", crate::ui::urlencode(select)));
                }
                return Redirect::temporary(&url).into_response();
            }
            Ok(None) => {}
            Err(error) => {
                return layout::error_page(
                    error.status,
                    Some(&identity.subject),
                    mechanism,
                    &error.message,
                );
            }
        }
    }
    match render_model_page(&state, &identity, &project, &query) {
        Ok(page) => layout::html_response(StatusCode::OK, page),
        Err(error) => layout::error_page(
            error.status,
            Some(&identity.subject),
            mechanism,
            &error.message,
        ),
    }
}

/// A project has either no model at all (nothing committed to any branch), or a model at
/// the resolved commit. The empty case is a state to act on, not an error.
#[allow(clippy::large_enum_variant)]
pub(crate) enum LoadedView {
    Empty,
    Model { commit: Commit, root: OkfRoot },
}

/// The shared read path for every commit-scoped view: the SAME identity, Read-permission and
/// project-scope decisions as the JSON handlers, then the commit and its parsed document.
pub(crate) fn load_view(
    state: &ApiState,
    identity: &Identity,
    project: &str,
    query: &ModelQuery,
) -> Result<LoadedView, ApiError> {
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
    let hash = match resolve_hash(state, identity, project, query) {
        Ok(hash) => hash,
        Err(error) => {
            // A project with no commits at all has no model to render. The page says what to
            // do next - with the action right there - instead of a bare 404. This is the
            // empty state, not an error: a named branch that has no tip while another does
            // is still a real 404.
            let branches = state
                .store_for(identity)
                .list_branches(project)
                .map_err(map_store_error)?;
            if branches.is_empty() {
                return Ok(LoadedView::Empty);
            }
            return Err(error);
        }
    };
    let commit = state
        .store_for(identity)
        .commit(project, &hash)
        .map_err(map_store_error)?
        .ok_or_else(|| ApiError::not_found(format!("commit {}", hash)))?;
    let root =
        load_model(state.store_for(identity).as_ref(), project, &hash).map_err(map_store_error)?;
    Ok(LoadedView::Model { commit, root })
}

/// The overview: the model's counts, coverage, versions and last check, plus the links out to
/// every section, above the full four-section view the three-pane workbench enhances.
fn render_model_page(
    state: &ApiState,
    identity: &Identity,
    project: &str,
    query: &ModelQuery,
) -> Result<Markup, ApiError> {
    let nav = layout::Nav::load(state, identity, Some(project))?;
    match load_view(state, identity, project, query)? {
        LoadedView::Empty => Ok(crate::ui::create::empty_model_page(
            identity,
            state.auth.mechanism(),
            project,
            &[],
            &nav,
        )),
        LoadedView::Model { commit, root } => {
            let mut nav = nav;
            nav.section = Some("overview");
            nav.branch = Some(view_branch(query, &commit));
            nav.commit = Some(commit.hash.clone());
            let branches = state
                .store_for(identity)
                .list_branches(project)
                .map_err(map_store_error)?;
            let latest_run = if identity.may(Permission::Write) || identity.may(Permission::Review)
            {
                state
                    .store_for(identity)
                    .gate_runs(project)
                    .map_err(map_store_error)?
                    .into_iter()
                    .next()
            } else {
                None
            };
            Ok(model_markup(
                identity,
                state.auth.mechanism(),
                project,
                &commit,
                &root,
                &branches,
                latest_run.as_ref(),
                &nav,
            ))
        }
    }
}

/// The branch name the view is scoped to: the query's branch when given (which may be a
/// different branch than the commit was created on - a freshly-created version points at an
/// existing tip), otherwise the commit's own branch.
pub(crate) fn view_branch(query: &ModelQuery, commit: &Commit) -> String {
    match query.branch.as_deref().filter(|branch| !branch.is_empty()) {
        Some(branch) => branch.to_string(),
        None => commit.branch.clone(),
    }
}

/// The branch a bare overview URL resolves to: the main line when it exists, otherwise the
/// alphabetically-first branch (the store returns branches sorted by name). Returns None for a
/// project with no branches at all.
fn default_branch(
    state: &ApiState,
    identity: &Identity,
    project: &str,
) -> Result<Option<String>, ApiError> {
    let branches = state
        .store_for(identity)
        .list_branches(project)
        .map_err(map_store_error)?;
    if branches.is_empty() {
        return Ok(None);
    }
    if branches.iter().any(|(name, _)| name == "main") {
        return Ok(Some("main".to_string()));
    }
    Ok(branches.first().map(|(name, _)| name.clone()))
}

/// Resolve the commit to render: an explicit commit hash, else the named branch's tip
/// (defaulting to `main`), using only the existing store reads.
pub(crate) fn resolve_hash(
    state: &ApiState,
    identity: &Identity,
    project: &str,
    query: &ModelQuery,
) -> Result<String, ApiError> {
    if let Some(commit) = &query.commit {
        if !commit.is_empty() {
            return Ok(commit.clone());
        }
    }
    let branch = query
        .branch
        .as_deref()
        .filter(|branch| !branch.is_empty())
        .unwrap_or("main");
    state
        .store_for(identity)
        .branch_tip(project, branch)
        .map_err(map_store_error)?
        .ok_or_else(|| ApiError::not_found(format!("branch {} has no commits", branch)))
}

/// Everything the four sections need, computed once. The coverage report is the engine's,
/// never the page's: a second implementation would disagree with the gate.
struct ViewContext {
    coverage: Option<CoverageReport>,
    nodes: HashMap<String, GraphNode>,
    allocations: HashMap<String, Vec<Allocation>>,
}

/// Everything the sections need, computed once from the document.
fn view_context(root: &OkfRoot) -> ViewContext {
    let nodes = node_index(root);
    let allocations = allocations(root, &nodes);
    let coverage = root.graph.as_ref().map(|_| requirement_coverage(root));
    ViewContext {
        coverage,
        nodes,
        allocations,
    }
}

/// The overview body: counts, coverage, versions and last check above the full four-section
/// view, with a link out to every section.
#[allow(clippy::too_many_arguments)]
fn model_markup(
    identity: &Identity,
    mechanism: &str,
    project: &str,
    commit: &Commit,
    root: &OkfRoot,
    branches: &[(String, String)],
    latest_run: Option<&GateRun>,
    nav: &layout::Nav,
) -> Markup {
    let ctx = view_context(root);
    // The edit link is offered only to a caller who may write, so a read-only reviewer is
    // not invited into a form that will only be refused on submit.
    let can_edit = identity.may(Permission::Write);
    let body = html! {
        h1 { "Overview" }
        (overview_summary(root, &ctx, branches, latest_run, nav))
        p class="meta" {
            "branch " (commit.branch) " · commit " code { (short_hash(&commit.hash)) }
            @if !commit.message.is_empty() {
                " · " (commit.message)
            }
            " · by " (commit.author)
        }
        p class="meta" {
            // The COMMIT, not the branch: viewing a historical model and then opening the
            // branch's current diagram would show a different model than the one being read.
            a href={ "/ui/projects/" (crate::ui::urlencode(project)) "/diagram?commit=" (crate::ui::urlencode(commit.hash.as_str())) } { "View diagram" }
        }
        // Adding an element WRITES, so the link is offered only to a caller who may write.
        @if can_edit {
            p class="meta" {
                a href={ "/ui/projects/" (crate::ui::urlencode(project)) "/element/new?branch=" (crate::ui::urlencode(commit.branch.as_str())) } { "Add element" }
            }
        }
        // The assist panel WRITES a proposal, so it is offered only to a caller who may
        // write; a read-only reviewer is not invited into a form that will only be refused.
        @if can_edit {
            (assist_panel_markup(project, &commit.branch, &reasoner_status()))
        }
        (structure_section(root, project, &commit.branch, can_edit))
        (requirements_section(root, &ctx))
        (traceability_section(root, &ctx))
        (state_activity_section(root, &ctx))
        script src="/ui/app.js" {}
    };
    let title = format!("modelwrite — {}", project);
    layout::shell(
        &title,
        nav,
        Some(&identity.subject),
        identity.may(Permission::Administer),
        mechanism,
        body,
    )
}

/// The overview cards plus the section links: what this model is, and where to go next.
fn overview_summary(
    root: &OkfRoot,
    ctx: &ViewContext,
    branches: &[(String, String)],
    latest_run: Option<&GateRun>,
    nav: &layout::Nav,
) -> Markup {
    let covered = ctx
        .coverage
        .as_ref()
        .map(|report| report.covered)
        .unwrap_or(0);
    let total = ctx
        .coverage
        .as_ref()
        .map(|report| report.total)
        .unwrap_or(root.requirements.len());
    let uncovered = ctx
        .coverage
        .as_ref()
        .map(|report| report.uncovered.len())
        .unwrap_or(root.requirements.len());
    html! {
        div class="overview-summary" {
            div class="overview-card" {
                span class="ov-label" { "Coverage" }
                div class="ov-value" { (covered) " / " (total) }
                div class="ov-note" {
                    span class="covered" { (covered) " covered" }
                    " · "
                    span class="uncovered" { (uncovered) " uncovered" }
                }
            }
            div class="overview-card" {
                span class="ov-label" { "Elements" }
                div class="ov-value" { (root.structure.len()) }
                div class="ov-note" {
                    (root.signals.len()) " signals · " (root.interfaces.len()) " interfaces"
                }
            }
            div class="overview-card" {
                span class="ov-label" { "Requirements" }
                div class="ov-value" { (root.requirements.len()) }
                div class="ov-note" { (root.activities.len()) " activities" }
            }
            div class="overview-card" {
                span class="ov-label" { "Versions" }
                div class="ov-value" { (branches.len()) }
                div class="ov-note" {
                    @for (name, tip) in branches {
                        (name) " " code { (short_hash(tip)) } "; "
                    }
                }
            }
            @if let Some(run) = latest_run {
                div class="overview-card" {
                    span class="ov-label" { "Last check" }
                    div class="ov-value" {
                        @if run.passed {
                            span class="covered" { "passed" }
                        } @else {
                            span class="uncovered" { "failed" }
                        }
                    }
                    div class="ov-note" { code { (short_hash(&run.candidate_hash)) } }
                }
            }
        }
        nav class="section-links" aria-label="model sections" {
            @for (key, label) in layout::SECTIONS.iter().copied() {
                a.current[nav.section == Some(key)] href=(layout::section_href(nav, key)) { (label) }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// The per-section addresses: each section of the overview at its own URL, reusing the same
// section markup the overview renders rather than a second implementation.

/// `GET /ui/projects/:project/structure` - the containment tree plus the state and activity
/// views, at their own address rather than buried in one long page.
pub async fn structure_page(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(project): Path<String>,
    Query(query): Query<ModelQuery>,
) -> Response {
    let mechanism = state.auth.mechanism();
    let identity = match resolve_identity(&state, &headers).await {
        Ok(identity) => identity,
        Err(error) => return layout::sign_in_page(mechanism, &error.message),
    };
    match render_section_page(&state, &identity, &project, &query, "structure") {
        Ok(page) => layout::html_response(StatusCode::OK, page),
        Err(error) => layout::error_page(
            error.status,
            Some(&identity.subject),
            mechanism,
            &error.message,
        ),
    }
}

/// `GET /ui/projects/:project/requirements` - the requirement table at its own address.
pub async fn requirements_page(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(project): Path<String>,
    Query(query): Query<ModelQuery>,
) -> Response {
    let mechanism = state.auth.mechanism();
    let identity = match resolve_identity(&state, &headers).await {
        Ok(identity) => identity,
        Err(error) => return layout::sign_in_page(mechanism, &error.message),
    };
    match render_section_page(&state, &identity, &project, &query, "requirements") {
        Ok(page) => layout::html_response(StatusCode::OK, page),
        Err(error) => layout::error_page(
            error.status,
            Some(&identity.subject),
            mechanism,
            &error.message,
        ),
    }
}

/// `GET /ui/projects/:project/traceability` - the traceability matrix at its own address.
pub async fn traceability_page(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(project): Path<String>,
    Query(query): Query<ModelQuery>,
) -> Response {
    let mechanism = state.auth.mechanism();
    let identity = match resolve_identity(&state, &headers).await {
        Ok(identity) => identity,
        Err(error) => return layout::sign_in_page(mechanism, &error.message),
    };
    match render_section_page(&state, &identity, &project, &query, "traceability") {
        Ok(page) => layout::html_response(StatusCode::OK, page),
        Err(error) => layout::error_page(
            error.status,
            Some(&identity.subject),
            mechanism,
            &error.message,
        ),
    }
}

/// The shared renderer for the three section pages.
fn render_section_page(
    state: &ApiState,
    identity: &Identity,
    project: &str,
    query: &ModelQuery,
    section: &'static str,
) -> Result<Markup, ApiError> {
    let nav = layout::Nav::load(state, identity, Some(project))?;
    match load_view(state, identity, project, query)? {
        LoadedView::Empty => Ok(crate::ui::create::empty_model_page(
            identity,
            state.auth.mechanism(),
            project,
            &[],
            &nav,
        )),
        LoadedView::Model { commit, root } => {
            let mut nav = nav;
            nav.section = Some(section);
            nav.branch = Some(view_branch(query, &commit));
            nav.commit = Some(commit.hash.clone());
            Ok(section_markup(
                identity,
                state.auth.mechanism(),
                project,
                &commit,
                &root,
                section,
                &nav,
            ))
        }
    }
}

fn section_markup(
    identity: &Identity,
    mechanism: &str,
    project: &str,
    commit: &Commit,
    root: &OkfRoot,
    section: &'static str,
    nav: &layout::Nav,
) -> Markup {
    let ctx = view_context(root);
    let can_edit = identity.may(Permission::Write);
    // Structure and Requirements carry selectable elements, so they get the three-pane
    // enhancement (tree + properties). Traceability is a read-only matrix: it uses the
    // whole width instead of the reading-width cap.
    let (body, main_class) = match section {
        "structure" => (
            html! {
                h1 { "Structure" }
                p class="meta" { "The containment tree of blocks, actors and use cases, plus the model's signals and interfaces." }
                @if can_edit {
                    p class="meta" {
                        a href={ "/ui/projects/" (crate::ui::urlencode(project)) "/element/new?branch=" (crate::ui::urlencode(commit.branch.as_str())) } { "Add element" }
                    }
                }
                (structure_section(root, project, &commit.branch, can_edit))
                (state_activity_section(root, &ctx))
                script src="/ui/app.js" {}
            },
            "",
        ),
        "requirements" => (
            html! {
                h1 { "Requirements" }
                p class="meta" { "Every requirement, its coverage verdict and the elements allocated to it." }
                (requirements_section(root, &ctx))
                script src="/ui/app.js" {}
            },
            "",
        ),
        "traceability" => (
            html! {
                h1 { "Traceability" }
                p class="meta" { "The engine's coverage report and the requirement-to-element allocations." }
                (traceability_section(root, &ctx))
            },
            "mw-wide",
        ),
        _ => (html! { h1 { "Overview" } }, ""),
    };
    let title = format!("modelwrite — {} — {}", project, section);
    layout::shell_with_main_class(
        &title,
        nav,
        Some(&identity.subject),
        identity.may(Permission::Administer),
        mechanism,
        main_class,
        body,
    )
}

// ---------------------------------------------------------------------------
// Structure: the indented tree plus the signal (and interface) element lists.

struct TreeNode<'a> {
    element: &'a Element,
    children: Vec<TreeNode<'a>>,
}

fn structure_section(root: &OkfRoot, project: &str, branch: &str, can_edit: bool) -> Markup {
    let tree = structure_tree(root);
    html! {
        section class="model-section" id="structure" {
            h2 { "Structure" }
            @if tree.is_empty() {
                p { "This model has no structure elements." }
            } @else {
                ul class="structure-tree" { (render_tree(&tree, project, branch, can_edit)) }
            }
            @if !root.signals.is_empty() {
                h3 { "Signals" }
                ul class="signals" {
                    @for signal in &root.signals {
                        li class="signal"
                           data-mw-id=(signal.id)
                           data-mw-name=(signal.name)
                           data-mw-kind=(signal.kind)
                           data-mw-stereotypes=(json_attr(&signal.stereotypes))
                           data-mw-attributes=(json_attr(&signal.attributes))
                           data-mw-documentation=(signal.documentation) {
                            span class="element-name" { (signal.name) }
                            @if !signal.kind.is_empty() {
                                span class="element-kind" { (signal.kind) }
                            }
                            @if !signal.documentation.is_empty() {
                                p class="element-documentation" { (signal.documentation) }
                            }
                        }
                    }
                }
            }
            @if !root.interfaces.is_empty() {
                h3 { "Interfaces" }
                ul class="interfaces" {
                    @for interface in &root.interfaces {
                        li class="interface"
                           data-mw-id=(interface.id)
                           data-mw-name=(interface.name)
                           data-mw-kind=(interface.kind)
                           data-mw-stereotypes=(json_attr(&interface.stereotypes))
                           data-mw-attributes=(json_attr(&interface.attributes))
                           data-mw-documentation=(interface.documentation) {
                            span class="element-name" { (interface.name) }
                            @if !interface.kind.is_empty() {
                                span class="element-kind" { (interface.kind) }
                            }
                            @if !interface.documentation.is_empty() {
                                p class="element-documentation" { (interface.documentation) }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// The structure section is a forest, not a tree: the corpus has seventeen roots, and one
/// block is a part of two different parents. Every element renders exactly once, at the
/// depth of its first-discovered parent, so a DAG cannot duplicate or drop an element.
fn structure_tree<'a>(root: &'a OkfRoot) -> Vec<TreeNode<'a>> {
    let elements: HashMap<&'a str, &'a Element> = root
        .structure
        .iter()
        .map(|element| (element.id.as_str(), element))
        .collect();
    let mut children: HashMap<&'a str, Vec<&'a str>> = HashMap::new();
    let mut has_parent: HashSet<&'a str> = HashSet::new();
    if let Some(graph) = &root.graph {
        for edge in &graph.edges {
            if (edge.kind == "part" || edge.kind == "contains")
                && elements.contains_key(edge.source.as_str())
                && elements.contains_key(edge.target.as_str())
            {
                children
                    .entry(edge.source.as_str())
                    .or_default()
                    .push(edge.target.as_str());
                has_parent.insert(edge.target.as_str());
            }
        }
    }
    let mut roots: Vec<&'a Element> = root
        .structure
        .iter()
        .filter(|element| !has_parent.contains(element.id.as_str()))
        .collect();
    roots.sort_by(|a, b| a.name.cmp(&b.name));
    // The ROOTS are marked visited as they are emitted. Without that the unreached-element
    // sweep below would treat every root as unreached and render it twice.
    let mut visited: HashSet<&'a str> = HashSet::new();
    let mut nodes: Vec<TreeNode<'a>> = Vec::with_capacity(roots.len());
    for element in roots {
        visited.insert(element.id.as_str());
        nodes.push(build_node(element, &elements, &children, &mut visited));
    }

    // Anything the walk did not reach is still rendered.
    //
    // A part/contains CYCLE leaves every element in the cycle with a parent but no root to
    // reach it from, so the walk never arrives and those elements would simply be missing -
    // which is the failure this whole platform exists to prevent, and the model is untrusted
    // input from a colleague, a supplier or an import. The corpus is acyclic, so its test
    // could never catch this for any other model.
    //
    // Emitting the unreached elements as their own roots means the section can under-render
    // NOTHING: every element in the document appears exactly once, whatever shape the graph
    // turns out to be.
    let mut orphaned: Vec<&'a Element> = root
        .structure
        .iter()
        .filter(|element| !visited.contains(element.id.as_str()))
        .collect();
    orphaned.sort_by(|a, b| a.name.cmp(&b.name));
    for element in orphaned {
        visited.insert(element.id.as_str());
        nodes.push(build_node(element, &elements, &children, &mut visited));
    }
    nodes
}

fn build_node<'a>(
    element: &'a Element,
    elements: &HashMap<&'a str, &'a Element>,
    children: &HashMap<&'a str, Vec<&'a str>>,
    visited: &mut HashSet<&'a str>,
) -> TreeNode<'a> {
    let mut kids: Vec<TreeNode<'a>> = Vec::new();
    if let Some(ids) = children.get(element.id.as_str()) {
        let mut child_elements: Vec<&'a Element> = ids
            .iter()
            .filter_map(|id| elements.get(*id).copied())
            .collect();
        child_elements.sort_by(|a, b| a.name.cmp(&b.name));
        for child in child_elements {
            if visited.insert(child.id.as_str()) {
                kids.push(build_node(child, elements, children, visited));
            }
        }
    }
    TreeNode {
        element,
        children: kids,
    }
}

fn render_tree(nodes: &[TreeNode<'_>], project: &str, branch: &str, can_edit: bool) -> Markup {
    html! {
        @for node in nodes {
            li class="element"
               data-mw-id=(node.element.id)
               data-mw-name=(node.element.name)
               data-mw-kind=(node.element.kind)
               data-mw-stereotypes=(json_attr(&node.element.stereotypes))
               data-mw-attributes=(json_attr(&node.element.attributes))
               data-mw-documentation=(node.element.documentation) {
                span class="element-name" { (node.element.name) }
                @if !node.element.kind.is_empty() {
                    span class="element-kind" { (node.element.kind) }
                }
                @if !node.element.stereotypes.is_empty() {
                    span class="element-stereotypes" { "«" (node.element.stereotypes.join(", ")) "»" }
                }
                @if !node.element.documentation.is_empty() {
                    p class="element-documentation" { (node.element.documentation) }
                }
                @if can_edit {
                    a class="element-edit" href={ "/ui/projects/" (crate::ui::urlencode(project)) "/edit/" (crate::ui::urlencode(node.element.id.as_str())) "?branch=" (crate::ui::urlencode(branch)) } { "edit" }
                }
                @if !node.children.is_empty() {
                    ul { (render_tree(&node.children, project, branch, can_edit)) }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Requirements and traceability.

/// One edge that connects a requirement to the element allocated to it. The relation is the
/// dependency label; an allocation whose endpoint is not a graph node is recorded as
/// unresolved rather than dropped, so a broken satisfy link stays visible.
#[derive(Serialize)]
struct Allocation {
    relation: String,
    element_id: String,
    name: String,
    unresolved: bool,
}

fn node_index(root: &OkfRoot) -> HashMap<String, GraphNode> {
    let mut map = HashMap::new();
    if let Some(graph) = &root.graph {
        for node in &graph.nodes {
            map.insert(node.id.clone(), node.clone());
        }
    }
    map
}

/// The elements allocated to each requirement, read from dependency edges. This is display
/// of the model's own edges; the AUTHORITATIVE covered/uncovered verdict is the engine's
/// [graph::requirement_coverage], which the page renders rather than recomputes.
fn allocations(
    root: &OkfRoot,
    nodes: &HashMap<String, GraphNode>,
) -> HashMap<String, Vec<Allocation>> {
    let req_ids: HashSet<&str> = root
        .requirements
        .iter()
        .map(|requirement| requirement.id.as_str())
        .collect();
    let mut map: HashMap<String, Vec<Allocation>> = HashMap::new();
    if let Some(graph) = &root.graph {
        for edge in &graph.edges {
            if edge.kind != "dependency" {
                continue;
            }
            let (req_id, element_id, relation) = match edge.label.as_str() {
                "Satisfy" if req_ids.contains(edge.target.as_str()) => {
                    (edge.target.clone(), edge.source.clone(), "satisfy")
                }
                "Refine" if req_ids.contains(edge.target.as_str()) => {
                    (edge.target.clone(), edge.source.clone(), "refine")
                }
                "Verify" if req_ids.contains(edge.target.as_str()) => {
                    (edge.target.clone(), edge.source.clone(), "verify")
                }
                "Allocate" if req_ids.contains(edge.target.as_str()) => {
                    (edge.target.clone(), edge.source.clone(), "allocate")
                }
                "Allocate" if req_ids.contains(edge.source.as_str()) => {
                    (edge.source.clone(), edge.target.clone(), "allocate")
                }
                _ => continue,
            };
            let resolved = nodes.get(&element_id);
            map.entry(req_id).or_default().push(Allocation {
                relation: relation.to_string(),
                element_id: element_id.clone(),
                name: resolved.map(|node| node.name.clone()).unwrap_or_default(),
                unresolved: resolved.is_none(),
            });
        }
    }
    map
}

fn allocation_list_markup(list: Option<&[Allocation]>) -> Markup {
    match list {
        Some(list) if !list.is_empty() => html! {
            ul class="allocations" {
                @for allocation in list {
                    li class={ "allocation " (allocation.relation) } {
                        span class="relation" { (allocation.relation) ": " }
                        @if allocation.unresolved {
                            span class="broken" title=(allocation.element_id) { "unresolved link " (allocation.element_id) }
                        } @else if allocation.name.is_empty() {
                            span title=(allocation.element_id) { (allocation.element_id) }
                        } @else {
                            span title=(allocation.element_id) { (allocation.name) }
                        }
                    }
                }
            }
        },
        _ => html! { span class="none" { "—" } },
    }
}

/// The human-facing text of a requirement. A folder requirement (the ten uncovered
/// ones in the corpus) carries only a name and an empty reqText, so the name is the text;
/// a leaf requirement carries both, so they are joined.
fn requirement_text(requirement: &Requirement) -> String {
    match (requirement.name.is_empty(), requirement.req_text.is_empty()) {
        (true, true) => String::new(),
        (true, false) => requirement.req_text.clone(),
        (false, true) => requirement.name.clone(),
        (false, false) => format!("{}: {}", requirement.name, requirement.req_text),
    }
}

/// Serialise a value to JSON for a `data-*` attribute. maud HTML-escapes the attribute
/// value on the way out, so a quote, an angle bracket or a script-looking string in model
/// data round-trips as inert data; the enhancement script reads it back with `dataset` and
/// `JSON.parse` and renders it with `textContent`, never `innerHTML`.
fn json_attr<T: Serialize>(value: &T) -> String {
    serde_json::to_string(value).expect("attribute serialisation cannot fail")
}

/// The ENGINE's coverage verdict for one requirement, read from the report computed once in
/// [model_markup]. The traceability section renders the same verdict from the same report;
/// repeating it here as a data attribute never recomputes coverage.
fn coverage_status(ctx: &ViewContext, requirement: &Requirement) -> &'static str {
    match &ctx.coverage {
        Some(report) if report.uncovered.iter().any(|id| id == &requirement.id) => "uncovered",
        Some(_) => "covered",
        None => "unknown",
    }
}

/// A coverage verdict as a text chip. The tint always comes with the word, so the state is
/// never carried by colour alone.
fn coverage_badge(status: &str) -> Markup {
    match status {
        "covered" => {
            html! { span class="covered" title="Requirement coverage: covered by at least one link" { "covered" } }
        }
        "uncovered" => {
            html! { span class="uncovered" title="Requirement coverage: not covered by any link" { "uncovered" } }
        }
        _ => {
            html! { span class="mw-badge-unknown" title="Requirement coverage: could not be reported" { "not reported" } }
        }
    }
}

fn requirements_section(root: &OkfRoot, ctx: &ViewContext) -> Markup {
    html! {
        section class="model-section" id="requirements" {
            h2 { "Requirements" }
            @if root.requirements.is_empty() {
                p { "This model has no requirements." }
            } @else {
                table class="requirements" {
                    thead {
                        tr { th { "id" } th { "reqId" } th { "text" } th { "satisfied by" } th { "coverage" } }
                    }
                    tbody {
                        @for requirement in &root.requirements {
                            tr class="requirement"
                               data-mw-id=(requirement.id)
                               data-mw-name=(requirement.name)
                               data-mw-kind=(requirement.kind)
                               data-mw-stereotypes=(json_attr(&requirement.stereotypes))
                               data-mw-attributes=(json_attr(&requirement.attributes))
                               data-mw-documentation=(requirement.documentation)
                               data-mw-reqid=(requirement.req_id)
                               data-mw-reqtext=(requirement.req_text)
                               data-mw-coverage=(coverage_status(ctx, requirement)) {
                                td class="req-id" title=(requirement.id) { (requirement.id) }
                                td class="req-num" { (requirement.req_id) }
                                td class="req-text" { (requirement_text(requirement)) }
                                td class="req-satisfied" {
                                    (allocation_list_markup(ctx.allocations.get(&requirement.id).map(|v| v.as_slice())))
                                }
                                td class="req-coverage" {
                                    (coverage_badge(coverage_status(ctx, requirement)))
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

fn traceability_section(root: &OkfRoot, ctx: &ViewContext) -> Markup {
    // The numbers are the ENGINE's: requirement_coverage is the same function the gate
    // reports, so a reviewer reads the same "15 covered / 10 uncovered" here and there.
    let (total, covered, uncovered_ids, satisfied, refined, verified, allocated) =
        match &ctx.coverage {
            Some(c) => {
                let ids: HashSet<&str> = c.uncovered.iter().map(|s| s.as_str()).collect();
                (
                    c.total,
                    c.covered,
                    ids,
                    c.satisfied,
                    c.refined,
                    c.verified,
                    c.allocated,
                )
            }
            // No graph section means no relationships, so the engine has nothing to measure
            // and returns no report at all. Reporting "0 covered" would be a NUMBER the
            // engine never produced, which is exactly the kind of quiet invention this page
            // is not allowed to make; the section says there is nothing to measure instead.
            None => {
                let ids: HashSet<&str> = root
                    .requirements
                    .iter()
                    .map(|requirement| requirement.id.as_str())
                    .collect();
                (root.requirements.len(), 0, ids, 0, 0, 0, 0)
            }
        };
    let measurable = ctx.coverage.is_some();
    let unresolved = unresolved_edges(root);
    html! {
        section class="model-section" id="traceability" {
            h2 { "Traceability" }
            @if measurable {
                p class="coverage-summary" {
                    (total) " requirements: "
                    span class="covered" { (covered) " covered" }
                    ", "
                    span class="uncovered" { (uncovered_ids.len()) " uncovered" }
                    " · "
                    (satisfied) " satisfy, " (refined) " refine, " (verified) " verify, " (allocated) " allocate"
                }
            } @else {
                p class="coverage-summary" {
                    "This model has no graph section, so there are no relationships to measure. "
                    "Coverage cannot be reported."
                }
            }
            @if root.requirements.is_empty() {
                p { "This model has no requirements." }
            } @else {
                table class="traceability" {
                    thead {
                        tr {
                            th { "id" } th { "reqId" } th { "text" } th { "allocated elements" } th { "status" }
                        }
                    }
                    tbody {
                        @for requirement in &root.requirements {
                            tr class="trace-row" {
                                td class="req-id" title=(requirement.id) { (requirement.id) }
                                td class="req-num" { (requirement.req_id) }
                                td class="req-text" { (requirement_text(requirement)) }
                                td class="trace-allocations" {
                                    (allocation_list_markup(ctx.allocations.get(&requirement.id).map(|v| v.as_slice())))
                                }
                                td class="trace-status" {
                                    @if uncovered_ids.contains(requirement.id.as_str()) {
                                        span class="uncovered" { "uncovered" }
                                    } @else {
                                        span class="covered" { "covered" }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            @if !unresolved.is_empty() {
                h3 { "Unresolved edges" }
                p class="meta" {
                    (unresolved.len()) " edge(s) point at an element that is not a graph node."
                }
                ul class="unresolved-edges" {
                    @for edge in &unresolved {
                        li class="unresolved-edge" {
                            @if edge.source_missing {
                                span class="broken" { "unresolved " (edge.source) }
                            } @else {
                                code { (edge.source) }
                            }
                            " → "
                            @if edge.target_missing {
                                span class="broken" { "unresolved " (edge.target) }
                            } @else {
                                code { (edge.target) }
                            }
                            " [" (edge.kind) " " (edge.label) "]"
                        }
                    }
                }
            }
        }
    }
}

/// A graph edge whose source or target names an element that was never emitted as a node.
/// The corpus has two; a broken link that is hidden is worse than one that is shown.
struct UnresolvedEdge {
    source: String,
    target: String,
    kind: String,
    label: String,
    source_missing: bool,
    target_missing: bool,
}

fn unresolved_edges(root: &OkfRoot) -> Vec<UnresolvedEdge> {
    let mut out = Vec::new();
    if let Some(graph) = &root.graph {
        let ids: HashSet<&str> = graph.nodes.iter().map(|node| node.id.as_str()).collect();
        for edge in &graph.edges {
            let source_missing = !ids.contains(edge.source.as_str());
            let target_missing = !ids.contains(edge.target.as_str());
            if source_missing || target_missing {
                out.push(UnresolvedEdge {
                    source: edge.source.clone(),
                    target: edge.target.clone(),
                    kind: edge.kind.clone(),
                    label: edge.label.clone(),
                    source_missing,
                    target_missing,
                });
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// State and activity.

fn state_activity_section(root: &OkfRoot, ctx: &ViewContext) -> Markup {
    html! {
        section class="model-section" id="state-activity" {
            h2 { "State and activity" }
            (state_machine_markup(root, ctx))
            (activities_markup(root))
        }
    }
}

fn state_machine_markup(root: &OkfRoot, ctx: &ViewContext) -> Markup {
    let Some(state_machine) = &root.state_machine else {
        return html! { p { "This model has no state machine." } };
    };
    let transitions: Vec<&GraphEdge> = root
        .graph
        .as_ref()
        .map(|graph| {
            graph
                .edges
                .iter()
                .filter(|edge| edge.kind == "transition")
                .collect()
        })
        .unwrap_or_default();
    html! {
        h3 { "State machine: " (state_machine.name) }
        @if state_machine.regions.is_empty() {
            p { "This state machine has no regions." }
        } @else {
            @for (i, region) in state_machine.regions.iter().enumerate() {
                h4 { "Region " (i + 1) }
                @if region.states.is_empty() {
                    p { "This region has no states." }
                } @else {
                    ul class="states" {
                        @for state in &region.states {
                            li class="state" {
                                span class="state-name" {
                                    @if state.name.is_empty() { (state.id) } @else { (state.name) }
                                }
                                @if let Some(entry) = &state.entry {
                                    span class="state-entry" { "entry: " (entry) }
                                }
                                @if let Some(do_activity) = &state.do_activity {
                                    span class="state-do" { "do: " (do_activity) }
                                }
                                @if let Some(exit) = &state.exit {
                                    span class="state-exit" { "exit: " (exit) }
                                }
                            }
                        }
                    }
                }
            }
        }
        @if transitions.is_empty() {
            p { "This state machine has no transitions." }
        } @else {
            h4 { "Transitions" }
            ul class="transitions" {
                @for transition in &transitions {
                    li class="transition" {
                        (state_label(&transition.source, ctx))
                        " → "
                        (state_label(&transition.target, ctx))
                        @if !transition.label.is_empty() { " [" (transition.label) "]" }
                    }
                }
            }
        }
    }
}

/// The display name of a state or element referenced by a transition, falling back to its id
/// when the model did not carry a name.
fn state_label(id: &str, ctx: &ViewContext) -> String {
    match ctx.nodes.get(id) {
        Some(node) if !node.name.is_empty() => node.name.clone(),
        _ => id.to_string(),
    }
}

fn activities_markup(root: &OkfRoot) -> Markup {
    html! {
        h3 { "Activities" }
        @if root.activities.is_empty() {
            p { "This model has no activities." }
        } @else {
            @for (index, activity) in root.activities.iter().enumerate() {
                div class="activity"
                   data-mw-id=(format!("activity:{}", index))
                   data-mw-name=(activity.name)
                   data-mw-kind="activity" {
                    h4 class="activity-name" {
                        @if activity.name.is_empty() { "(unnamed activity)" } @else { (activity.name) }
                    }
                    @if activity.nodes.is_empty() {
                        p class="meta" { "no nodes" }
                    } @else {
                        ul class="activity-nodes" {
                            @for node in &activity.nodes {
                                li class="activity-node" {
                                    @if node.node_type.is_empty() && node.name.is_empty() {
                                        (node.id)
                                    } @else {
                                        span class="node-type" { (node.node_type) }
                                        @if !node.name.is_empty() { " — " (node.name) }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

fn short_hash(hash: &str) -> &str {
    hash.get(..8).unwrap_or(hash)
}
