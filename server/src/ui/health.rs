// SPDX-License-Identifier: AGPL-3.0-or-later
//! The model-health view: the ONE screen that answers "what is broken in my model?".
//!
//! Four lists, each computed from the engine's own graph analysis rather than a second
//! implementation, so the page can never disagree with the gate:
//!
//! * orphaned nodes - nodes with no edges (the graph's isolated set),
//! * isolated groups - disconnected components of two or more nodes cut off from the main
//!   body, each with its members,
//! * dangling links - edges naming an endpoint that is not a node,
//! * uncovered requirements - requirements no Satisfy/Refine/Verify/Allocate edge covers.
//!
//! Every element is named by id AND name, and each list carries its count. The page reads
//! through the SAME identity, Read-permission and project-scope decisions as every other
//! workbench page, so it is never a weaker path to the data.

use std::collections::{HashMap, HashSet};

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use maud::{html, Markup};

use graph::{components, graph_stats, requirement_coverage};
use okf::types::OkfRoot;

use crate::api::ApiState;
use crate::auth::{identity as resolve_identity, Identity, Permission};
use crate::error::ApiError;
use crate::ui::layout;
use crate::ui::model::{load_view, view_branch, LoadedView, ModelQuery};

/// GET /ui/projects/:project/health?branch=&commit= - the single model-health screen.
pub async fn health_page(
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
    match render_health_page(&state, &identity, &project, &query) {
        Ok(page) => layout::html_response(StatusCode::OK, page),
        Err(error) => layout::error_page(
            error.status,
            Some(&identity.subject),
            mechanism,
            &error.message,
        ),
    }
}

fn render_health_page(
    state: &ApiState,
    identity: &Identity,
    project: &str,
    query: &ModelQuery,
) -> Result<Markup, ApiError> {
    let nav = layout::Nav::load(state, identity, Some(project))?;
    match load_view(state, identity, project, query)? {
        LoadedView::Empty => {
            let body = html! {
                h1 { "Model health" }
                p class="empty-state" { "This project has no model yet — there is nothing to inspect." }
            };
            Ok(layout::shell(
                &format!("modelwrite — {project} — health"),
                &nav,
                Some(&identity.subject),
                identity.may(Permission::Administer),
                state.auth.mechanism(),
                body,
            ))
        }
        LoadedView::Model { commit, root } => {
            let mut nav = nav;
            nav.section = Some("health");
            nav.branch = Some(view_branch(query, &commit));
            nav.commit = Some(commit.hash.clone());
            Ok(health_markup(
                identity,
                state.auth.mechanism(),
                project,
                &commit.branch,
                &commit.hash,
                &root,
                &nav,
            ))
        }
    }
}

/// One dangling link: an edge that names an endpoint with no matching node.
struct DanglingLink {
    source: String,
    target: String,
    kind: String,
    label: String,
    missing_source: bool,
    missing_target: bool,
}

/// The four findings the health view reports, computed once from the document.
struct HealthReport {
    orphaned: Vec<String>,
    isolated_groups: Vec<Vec<String>>,
    dangling: Vec<DanglingLink>,
    uncovered: Vec<String>,
}

/// Compute the health report from the engine's graph analysis. The orphaned set and the
/// component membership come straight from the graph crate; dangling links and names are the
/// only page-local reads, both over the same document.
fn health_report(root: &OkfRoot) -> HealthReport {
    let stats = graph_stats(root);
    let comps = components(root);

    // A disconnected component of two or more nodes that is not the main body (the largest
    // component) is an isolated group. Singletons with no edges are the orphaned list above,
    // so they are not duplicated here.
    let isolated_groups: Vec<Vec<String>> = comps
        .groups
        .iter()
        .skip(1)
        .filter(|group| group.len() >= 2)
        .cloned()
        .collect();

    let node_ids: HashSet<&str> = root
        .graph
        .as_ref()
        .map(|graph| graph.nodes.iter().map(|n| n.id.as_str()).collect())
        .unwrap_or_default();
    let mut dangling = Vec::new();
    if let Some(graph) = &root.graph {
        for edge in &graph.edges {
            let missing_source = !node_ids.contains(edge.source.as_str());
            let missing_target = !node_ids.contains(edge.target.as_str());
            if missing_source || missing_target {
                dangling.push(DanglingLink {
                    source: edge.source.clone(),
                    target: edge.target.clone(),
                    kind: edge.kind.clone(),
                    label: edge.label.clone(),
                    missing_source,
                    missing_target,
                });
            }
        }
    }
    // Deterministic order: edges are read in document order, which is stable for a given model.
    dangling.sort_by(|a, b| {
        (&a.source, &a.target, &a.kind, &a.label).cmp(&(&b.source, &b.target, &b.kind, &b.label))
    });

    let uncovered = requirement_coverage(root).uncovered;

    HealthReport {
        orphaned: stats.isolated,
        isolated_groups,
        dangling,
        uncovered,
    }
}

/// Every element id to its display name, read from the graph nodes first and supplemented by
/// the structure, interface, signal and requirement lists, so a node the graph carries under an
/// id the other lists also name is still resolved to its human name.
fn name_index(root: &OkfRoot) -> HashMap<String, String> {
    let mut names = HashMap::new();
    if let Some(graph) = &root.graph {
        for node in &graph.nodes {
            names.insert(node.id.clone(), node.name.clone());
        }
    }
    for element in &root.structure {
        names
            .entry(element.id.clone())
            .or_insert_with(|| element.name.clone());
    }
    for element in &root.interfaces {
        names
            .entry(element.id.clone())
            .or_insert_with(|| element.name.clone());
    }
    for element in &root.signals {
        names
            .entry(element.id.clone())
            .or_insert_with(|| element.name.clone());
    }
    for requirement in &root.requirements {
        names
            .entry(requirement.id.clone())
            .or_insert_with(|| requirement.name.clone());
    }
    names
}

/// Render one named element as "id — name", or just the id when no name is known.
fn named(names: &HashMap<String, String>, id: &str) -> Markup {
    match names.get(id).filter(|name| !name.is_empty()) {
        Some(name) => html! { code { (id) } " — " (name.as_str()) },
        None => html! { code { (id) } },
    }
}

#[allow(clippy::too_many_arguments)]
fn health_markup(
    identity: &Identity,
    mechanism: &str,
    project: &str,
    branch: &str,
    commit_hash: &str,
    root: &OkfRoot,
    nav: &layout::Nav,
) -> Markup {
    let report = health_report(root);
    let names = name_index(root);
    let issue_count = report.orphaned.len()
        + report.isolated_groups.len()
        + report.dangling.len()
        + report.uncovered.len();

    let body = html! {
        h1 { "Model health" }
        p class="meta" {
            "branch " (branch) " · commit " code { (&commit_hash[..commit_hash.len().min(8)]) }
        }
        @if issue_count == 0 {
            p class="check-passed" { "Healthy: no orphaned nodes, one connected component, no dangling links, every requirement covered." }
        } @else {
            p class="check-failed" { (issue_count) " issue" @if issue_count != 1 { "s" } " found." }
        }

        section class="model-section" id="orphaned" {
            h2 { "Orphaned nodes (" (report.orphaned.len()) ")" }
            p class="meta" { "Nodes with no edges. They are invisible to coverage and traceability." }
            @if report.orphaned.is_empty() {
                p { "None." }
            } @else {
                ul class="gaps" {
                    @for id in &report.orphaned { li { (named(&names, id)) } }
                }
            }
        }

        section class="model-section" id="isolated-groups" {
            h2 { "Isolated groups (" (report.isolated_groups.len()) ")" }
            p class="meta" { "Disconnected components of two or more nodes, cut off from the main body." }
            @if report.isolated_groups.is_empty() {
                p { "None — one connected component." }
            } @else {
                @for (index, group) in report.isolated_groups.iter().enumerate() {
                    h3 { "Group " (index + 1) " (" (group.len()) " nodes)" }
                    ul class="gaps" {
                        @for id in group { li { (named(&names, id)) } }
                    }
                }
            }
        }

        section class="model-section" id="dangling" {
            h2 { "Dangling links (" (report.dangling.len()) ")" }
            p class="meta" { "Edges naming an endpoint that is not a node in the graph." }
            @if report.dangling.is_empty() {
                p { "None." }
            } @else {
                ul class="unresolved-edges" {
                    @for link in &report.dangling {
                        li {
                            code { (link.source.as_str()) } " → " code { (link.target.as_str()) }
                            @if !link.label.is_empty() { " (" (link.label.as_str()) ")" }
                            @if link.missing_source {
                                " — missing source " code { (link.source.as_str()) }
                            }
                            @if link.missing_target {
                                " — missing target " code { (link.target.as_str()) }
                            }
                        }
                    }
                }
            }
        }

        section class="model-section" id="uncovered" {
            h2 { "Uncovered requirements (" (report.uncovered.len()) ")" }
            p class="meta" { "Requirements no Satisfy, Refine, Verify or Allocate edge covers." }
            @if report.uncovered.is_empty() {
                p { "None." }
            } @else {
                ul class="gaps" {
                    @for id in &report.uncovered { li { (named(&names, id)) } }
                }
            }
        }

        p class="meta" {
            a href={ "/ui/projects/" (crate::ui::urlencode(project)) "/requirements?branch=" (crate::ui::urlencode(branch)) } { "See the requirements" }
            " · "
            a href={ "/ui/projects/" (crate::ui::urlencode(project)) "/diagram?branch=" (crate::ui::urlencode(branch)) } { "See the diagram" }
        }
    };
    let title = format!("modelwrite — {project} — health");
    layout::shell(
        &title,
        nav,
        Some(&identity.subject),
        identity.may(Permission::Administer),
        mechanism,
        body,
    )
}
