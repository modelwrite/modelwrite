// SPDX-License-Identifier: AGPL-3.0-or-later
//! The global find-element search: one box in the shell header, reachable on every page
//! inside a project. Type a name fragment and get every matching element - by id, name and
//! kind - with a link to open it in the model page (selected) or to reveal it in the diagram.
//!
//! This is NOT the tree filter: it searches the whole model (graph nodes, structure,
//! requirements, interfaces and signals), and it is reachable from the Changes page, the
//! composition page, the checks page and every other page the tree filter was absent from.
//!
//! It reads through the SAME identity, Read-permission and project-scope decisions as every
//! other workbench page.

use std::collections::HashSet;

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use maud::{html, Markup};
use serde::Deserialize;

use okf::types::OkfRoot;

use crate::api::ApiState;
use crate::auth::{identity as resolve_identity, Identity, Permission};
use crate::error::ApiError;
use crate::ui::layout;
use crate::ui::model::{load_view, view_branch, LoadedView, ModelQuery};

/// GET /ui/projects/:project/search?q=&branch=&commit= - the global find-element results.
#[derive(Deserialize)]
pub struct SearchQuery {
    pub q: Option<String>,
    pub branch: Option<String>,
    pub commit: Option<String>,
}

pub async fn search_page(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(project): Path<String>,
    Query(query): Query<SearchQuery>,
) -> Response {
    let mechanism = state.auth.mechanism();
    let identity = match resolve_identity(&state, &headers).await {
        Ok(identity) => identity,
        Err(error) => return layout::sign_in_page(mechanism, &error.message),
    };
    match render_search_page(&state, &identity, &project, &query) {
        Ok(page) => layout::html_response(StatusCode::OK, page),
        Err(error) => layout::error_page(
            error.status,
            Some(&identity.subject),
            mechanism,
            &error.message,
        ),
    }
}

fn render_search_page(
    state: &ApiState,
    identity: &Identity,
    project: &str,
    query: &SearchQuery,
) -> Result<Markup, ApiError> {
    let nav = layout::Nav::load(state, identity, Some(project))?;
    let model_query = ModelQuery {
        branch: query.branch.clone(),
        commit: query.commit.clone(),
        view: None,
        select: None,
    };
    match load_view(state, identity, project, &model_query)? {
        LoadedView::Empty => {
            let body = html! {
                h1 { "Find element" }
                p class="empty-state" { "This project has no model yet — there is nothing to search." }
            };
            Ok(layout::shell(
                &format!("modelwrite — {project} — find element"),
                &nav,
                Some(&identity.subject),
                identity.may(Permission::Administer),
                state.auth.mechanism(),
                body,
            ))
        }
        LoadedView::Model { commit, root } => {
            let mut nav = nav;
            nav.branch = Some(view_branch(&model_query, &commit));
            nav.commit = Some(commit.hash.clone());
            let needle = query
                .q
                .as_deref()
                .map(str::trim)
                .unwrap_or("")
                .to_lowercase();
            Ok(search_markup(
                identity,
                state.auth.mechanism(),
                project,
                &view_branch(&model_query, &commit),
                &needle,
                &root,
                &nav,
            ))
        }
    }
}

/// One element the search can return, de-duplicated by id across the document's lists.
struct Hit {
    id: String,
    name: String,
    kind: String,
}

fn push_hit(hits: &mut Vec<Hit>, seen: &mut HashSet<String>, id: &str, name: &str, kind: &str) {
    if id.is_empty() || !seen.insert(id.to_string()) {
        return;
    }
    hits.push(Hit {
        id: id.to_string(),
        name: name.to_string(),
        kind: kind.to_string(),
    });
}

/// Every searchable element, de-duplicated by id: graph nodes first, then the structure,
/// requirement, interface and signal lists so an element the graph names by id still resolves
/// to its human name and kind.
fn collect_hits(root: &OkfRoot) -> Vec<Hit> {
    let mut hits = Vec::new();
    let mut seen = HashSet::new();
    if let Some(graph) = &root.graph {
        for node in &graph.nodes {
            push_hit(&mut hits, &mut seen, &node.id, &node.name, &node.kind);
        }
    }
    for element in &root.structure {
        push_hit(
            &mut hits,
            &mut seen,
            &element.id,
            &element.name,
            &element.kind,
        );
    }
    for requirement in &root.requirements {
        push_hit(
            &mut hits,
            &mut seen,
            &requirement.id,
            &requirement.name,
            &requirement.kind,
        );
    }
    for interface in &root.interfaces {
        push_hit(
            &mut hits,
            &mut seen,
            &interface.id,
            &interface.name,
            &interface.kind,
        );
    }
    for signal in &root.signals {
        push_hit(&mut hits, &mut seen, &signal.id, &signal.name, &signal.kind);
    }
    hits
}

fn hit_matches(hit: &Hit, needle: &str) -> bool {
    hit.id.to_lowercase().contains(needle)
        || hit.name.to_lowercase().contains(needle)
        || hit.kind.to_lowercase().contains(needle)
}

fn search_markup(
    identity: &Identity,
    mechanism: &str,
    project: &str,
    branch: &str,
    needle: &str,
    root: &OkfRoot,
    nav: &layout::Nav,
) -> Markup {
    let mut hits = collect_hits(root);
    if !needle.is_empty() {
        hits.retain(|hit| hit_matches(hit, needle));
    }
    hits.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.id.cmp(&b.id)));

    let body = html! {
        h1 { "Find element" }
        form method="get" action={ "/ui/projects/" (crate::ui::urlencode(project)) "/search" } class="site-search" {
            input type="hidden" name="branch" value=(branch);
            input type="search" name="q" value=(needle) placeholder="Find element (name / id / kind)" aria-label="Find an element";
            button type="submit" { "Search" }
        }
        @if needle.is_empty() {
            p class="meta" { "Type a name fragment, id or kind to search the whole model." }
        } @else if hits.is_empty() {
            p { "No elements match " strong { (needle) } "." }
        } @else {
            p class="meta" { (hits.len()) " match" @if hits.len() != 1 { "es" } " for " strong { (needle) } }
            ul class="search-hits" {
                @for hit in &hits {
                    li class="search-hit" {
                        code { (hit.id.as_str()) }
                        @if !hit.name.is_empty() {
                            " — " span class="hit-name" { (hit.name.as_str()) }
                        }
                        span class="hit-kind" { (hit.kind.as_str()) }
                        span class="hit-actions" {
                            a href={ "/ui/projects/" (crate::ui::urlencode(project)) "/model?branch=" (crate::ui::urlencode(branch)) "&select=" (crate::ui::urlencode(hit.id.as_str())) } { "open" }
                            a href={ "/ui/projects/" (crate::ui::urlencode(project)) "/diagram?branch=" (crate::ui::urlencode(branch)) "&select=" (crate::ui::urlencode(hit.id.as_str())) } { "in diagram" }
                        }
                    }
                }
            }
        }
    };
    let title = format!("modelwrite — {project} — find element");
    layout::shell(
        &title,
        nav,
        Some(&identity.subject),
        identity.may(Permission::Administer),
        mechanism,
        body,
    )
}
