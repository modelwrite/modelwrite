// SPDX-License-Identifier: AGPL-3.0-or-later
//! The model page: four views over one OKF document - the structure tree, the requirements
//! table, the traceability matrix and the state-and-activity view. Every view reads the SAME
//! parsed document the JSON handlers and the gate read (via [crate::api::load_model]), and
//! the traceability numbers come from the graph crate's own [graph::requirement_coverage],
//! so the page can never disagree with the gate about what is covered or what is unresolved.

use std::collections::{HashMap, HashSet};

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use maud::{html, Markup};
use serde::Deserialize;

use graph::{requirement_coverage, CoverageReport};
use okf::types::{Element, GraphEdge, GraphNode, OkfRoot, Requirement};

use crate::api::{load_model, map_store_error, ApiState};
use crate::auth::{identity as resolve_identity, Identity, Permission};
use crate::error::ApiError;
use crate::store::Commit;
use crate::ui::layout;

/// The query parameters of the model page. An explicit commit hash wins; otherwise the
/// named branch (defaulting to `main`) resolves to its tip through the same store reads
/// the JSON commit handler performs.
#[derive(Deserialize)]
pub struct ModelQuery {
    pub branch: Option<String>,
    pub commit: Option<String>,
}

/// `GET /ui/projects/:project/model?branch=&commit=` - the model an engineer came to see.
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

fn render_model_page(
    state: &ApiState,
    identity: &Identity,
    project: &str,
    query: &ModelQuery,
) -> Result<Markup, ApiError> {
    // The SAME identity, Read-permission and project-scope decisions as the JSON handlers:
    // the workbench can never be a weaker path to the data.
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
    let hash = resolve_hash(state, project, query)?;
    let commit = state
        .store
        .commit(project, &hash)
        .map_err(map_store_error)?
        .ok_or_else(|| ApiError::not_found(format!("commit {}", hash)))?;
    let root = load_model(state.store.as_ref(), project, &hash).map_err(map_store_error)?;
    Ok(model_markup(
        identity,
        state.auth.mechanism(),
        project,
        &commit,
        &root,
    ))
}

/// Resolve the commit to render: an explicit commit hash, else the named branch's tip
/// (defaulting to `main`), using only the existing store reads.
pub(crate) fn resolve_hash(
    state: &ApiState,
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
        .store
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

fn model_markup(
    identity: &Identity,
    mechanism: &str,
    project: &str,
    commit: &Commit,
    root: &OkfRoot,
) -> Markup {
    let nodes = node_index(root);
    let allocations = allocations(root, &nodes);
    let coverage = root.graph.as_ref().map(|_| requirement_coverage(root));
    let ctx = ViewContext {
        coverage,
        nodes,
        allocations,
    };
    let body = html! {
        h1 { "Model" }
        p class="meta" {
            "branch " (commit.branch) " · commit " code { (short_hash(&commit.hash)) }
            @if !commit.message.is_empty() {
                " · " (commit.message)
            }
            " · by " (commit.author)
        }
        p class="meta" {
            a href={ "/ui/projects/" (crate::ui::urlencode(project)) "/diagram?branch=" (crate::ui::urlencode(commit.branch.as_str())) } { "View diagram" }
        }
        (structure_section(root, project, &commit.branch))
        (requirements_section(root, &ctx))
        (traceability_section(root, &ctx))
        (state_activity_section(root, &ctx))
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

// ---------------------------------------------------------------------------
// Structure: the indented tree plus the signal (and interface) element lists.

struct TreeNode<'a> {
    element: &'a Element,
    children: Vec<TreeNode<'a>>,
}

fn structure_section(root: &OkfRoot, project: &str, branch: &str) -> Markup {
    let tree = structure_tree(root);
    html! {
        section class="model-section" id="structure" {
            h2 { "Structure" }
            @if tree.is_empty() {
                p { "This model has no structure elements." }
            } @else {
                ul class="structure-tree" { (render_tree(&tree, project, branch)) }
            }
            @if !root.signals.is_empty() {
                h3 { "Signals" }
                ul class="signals" {
                    @for signal in &root.signals {
                        li class="signal" {
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
                        li class="interface" {
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

fn render_tree(nodes: &[TreeNode<'_>], project: &str, branch: &str) -> Markup {
    html! {
        @for node in nodes {
            li class="element" {
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
                a class="element-edit" href={ "/ui/projects/" (crate::ui::urlencode(project)) "/edit/" (crate::ui::urlencode(node.element.id.as_str())) "?branch=" (crate::ui::urlencode(branch)) } { "edit" }
                @if !node.children.is_empty() {
                    ul { (render_tree(&node.children, project, branch)) }
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
                            span class="broken" { "unresolved " (allocation.element_id) }
                        } @else if allocation.name.is_empty() {
                            span { (allocation.element_id) }
                        } @else {
                            span { (allocation.name) }
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

fn requirements_section(root: &OkfRoot, ctx: &ViewContext) -> Markup {
    html! {
        section class="model-section" id="requirements" {
            h2 { "Requirements" }
            @if root.requirements.is_empty() {
                p { "This model has no requirements." }
            } @else {
                table class="requirements" {
                    thead {
                        tr { th { "id" } th { "reqId" } th { "text" } th { "satisfied by" } }
                    }
                    tbody {
                        @for requirement in &root.requirements {
                            tr class="requirement" {
                                td class="req-id" { (requirement.id) }
                                td class="req-num" { (requirement.req_id) }
                                td class="req-text" { (requirement_text(requirement)) }
                                td class="req-satisfied" {
                                    (allocation_list_markup(ctx.allocations.get(&requirement.id).map(|v| v.as_slice())))
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
                    (total) " requirements: " (covered) " covered, " (uncovered_ids.len()) " uncovered · "
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
                                td class="req-id" { (requirement.id) }
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
            @for activity in &root.activities {
                div class="activity" {
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
