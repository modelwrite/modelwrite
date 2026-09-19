// SPDX-License-Identifier: AGPL-3.0-or-later
//! The diagram view: the model's graph rendered as inline SVG. The SVG is COMPOSED as a
//! string in Rust - there is no drawing library and none is added - and the layout is a
//! deterministic arrangement computed from the model, never a hand-placed drawing.
//!
//! The guarantees the rest of the slice was built around hold here by construction:
//!
//! * DETERMINISTIC: the same model always produces byte-identical SVG, because everything
//!   iterated is sorted (kinds, nodes within a kind, edges, dangling endpoints) and no
//!   hash-map order is ever observed. A diagram that shuffled between loads could not be
//!   diffed, cached or compared.
//! * COMPLETE: every node and edge is drawn. An edge whose endpoint is not a node is drawn
//!   to an explicit "unresolved" marker rather than dropped - the two dangling satisfy
//!   links in the corpus are exactly the defect a picture should make obvious.
//! * SAFE: the SVG contains no script element and no external reference (no href, no
//!   url(...), no image or use), and every label from the model is XML-escaped, so it is
//!   safe to render anywhere and to embed in a report.
//!
//! The page reads through the SAME identity, permission and project-scope decisions as the
//! JSON handlers, in the SAME order, so the workbench can never be a weaker path to the data.

use std::collections::{BTreeSet, HashMap};

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use maud::{html, Markup, PreEscaped};

use okf::types::{Graph, GraphEdge, GraphNode, OkfRoot};

use crate::api::{load_model, map_store_error, ApiState};
use crate::auth::{identity as resolve_identity, Identity, Permission};
use crate::error::ApiError;
use crate::store::Commit;
use crate::ui::layout;
use crate::ui::model::{resolve_hash, ModelQuery};

// ---------------------------------------------------------------------------
// Layout constants.
//
// Nodes are grouped by kind: each kind is a column, and the nodes of a kind are stacked
// vertically, ordered by name then id. An optional left-hand lane holds the "unresolved"
// markers for edge endpoints that name no node. All geometry is a pure function of these
// constants and the sorted model, so the layout can never wobble between renders.

const MARGIN: f64 = 24.0;
const HEADER: f64 = 22.0;
const NODE_W: f64 = 200.0;
const NODE_H: f64 = 64.0;
const H_GAP: f64 = 48.0;
const V_GAP: f64 = 56.0;
const DANGLING_W: f64 = 180.0;
const SPREAD: f64 = 12.0;

/// The column order of the known node kinds. Unknown kinds follow in sorted order.
const KIND_ORDER: &[&str] = &[
    "stateMachine",
    "actor",
    "usecase",
    "requirement",
    "block",
    "signal",
    "state",
    "activity",
];

fn kind_rank(kind: &str) -> usize {
    KIND_ORDER
        .iter()
        .position(|known| *known == kind)
        .unwrap_or(KIND_ORDER.len())
}

/// `GET /ui/projects/:project/diagram?branch=&commit=` - the model's graph as inline SVG.
pub async fn diagram_page(
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
    match render_diagram_page(&state, &identity, &project, &query) {
        Ok(page) => layout::html_response(StatusCode::OK, page),
        Err(error) => layout::error_page(
            error.status,
            Some(&identity.subject),
            mechanism,
            &error.message,
        ),
    }
}

fn render_diagram_page(
    state: &ApiState,
    identity: &Identity,
    project: &str,
    query: &ModelQuery,
) -> Result<Markup, ApiError> {
    // The SAME identity, Read-permission and project-scope decisions as the JSON handlers,
    // in the SAME order: the workbench can never be a weaker path to the data.
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
    let mut nav = layout::Nav::load(state, identity, Some(project))?;
    nav.section = Some("diagram");
    nav.branch = Some(commit.branch.clone());
    nav.commit = Some(commit.hash.clone());
    Ok(diagram_markup(
        identity,
        state.auth.mechanism(),
        project,
        &commit,
        &root,
        &nav,
    ))
}

fn diagram_markup(
    identity: &Identity,
    mechanism: &str,
    project: &str,
    commit: &Commit,
    root: &OkfRoot,
    nav: &layout::Nav,
) -> Markup {
    let body = html! {
        h1 { "Diagram" }
        p class="meta" {
            "branch " (commit.branch) " · commit " code { (short_hash(&commit.hash)) }
            @if !commit.message.is_empty() {
                " · " (commit.message)
            }
            " · by " (commit.author)
        }
        p class="meta" {
            a href={ "/ui/projects/" (crate::ui::urlencode(project)) "/overview?branch=" (crate::ui::urlencode(commit.branch.as_str())) } { "Back to the overview" }
        }
        @if let Some(graph) = &root.graph {
            (PreEscaped(diagram_svg(graph)))
        } @else {
            p { "This model has no graph section, so there is nothing to draw." }
        }
        script src="/ui/app.js" {}
    };
    let title = format!("modelwrite — {} — diagram", project);
    layout::shell(
        &title,
        nav,
        Some(&identity.subject),
        identity.may(Permission::Administer),
        mechanism,
        body,
    )
}

fn diagram_svg(graph: &Graph) -> String {
    let mut svg = String::new();

    // The node index: id -> node, for kind grouping and endpoint resolution.
    let node_by_id: HashMap<&str, &GraphNode> = graph
        .nodes
        .iter()
        .map(|node| (node.id.as_str(), node))
        .collect();

    // Group nodes by kind, and order each kind's nodes deterministically.
    let mut kinds: HashMap<String, Vec<&GraphNode>> = HashMap::new();
    for node in &graph.nodes {
        kinds.entry(node.kind.clone()).or_default().push(node);
    }
    for nodes in kinds.values_mut() {
        nodes.sort_by(|a, b| (&a.name, &a.id).cmp(&(&b.name, &b.id)));
    }
    let mut kind_names: Vec<String> = kinds.keys().cloned().collect();
    kind_names.sort_by(|a, b| kind_rank(a).cmp(&kind_rank(b)).then_with(|| a.cmp(b)));

    // The unresolved endpoints: every edge source or target that is not a node.
    let mut dangling_set: BTreeSet<&str> = BTreeSet::new();
    for edge in &graph.edges {
        if !node_by_id.contains_key(edge.source.as_str()) {
            dangling_set.insert(edge.source.as_str());
        }
        if !node_by_id.contains_key(edge.target.as_str()) {
            dangling_set.insert(edge.target.as_str());
        }
    }
    let dangling_ids: Vec<String> = dangling_set.into_iter().map(|id| id.to_string()).collect();
    let has_dangling = !dangling_ids.is_empty();

    // Place every node: one column per kind, nodes stacked vertically.
    let mut node_centers: HashMap<String, (f64, f64)> = HashMap::new();
    let mut kind_columns: Vec<(String, f64)> = Vec::new();
    let mut max_rows = 0usize;
    for (column, kind) in kind_names.iter().enumerate() {
        let x = column_x(column, has_dangling);
        kind_columns.push((kind.clone(), x));
        if let Some(nodes) = kinds.get(kind) {
            for (row, node) in nodes.iter().enumerate() {
                node_centers.insert(
                    node.id.clone(),
                    (x + NODE_W / 2.0, row_y(row) + NODE_H / 2.0),
                );
            }
            max_rows = max_rows.max(nodes.len());
        }
    }

    // Place the dangling markers in a left-hand lane, stacked and sorted.
    let mut dangling_centers: HashMap<String, (f64, f64)> = HashMap::new();
    for (row, id) in dangling_ids.iter().enumerate() {
        dangling_centers.insert(
            id.clone(),
            (MARGIN + DANGLING_W / 2.0, row_y(row) + NODE_H / 2.0),
        );
    }
    max_rows = max_rows.max(dangling_ids.len());

    let node_cols = kind_names.len();
    let width = if has_dangling {
        MARGIN
            + DANGLING_W
            + H_GAP
            + node_cols as f64 * NODE_W
            + node_cols.saturating_sub(1) as f64 * H_GAP
            + MARGIN
    } else {
        MARGIN + node_cols as f64 * NODE_W + node_cols.saturating_sub(1) as f64 * H_GAP + MARGIN
    };
    let height = if max_rows == 0 {
        MARGIN + HEADER + NODE_H + MARGIN
    } else {
        row_y(max_rows - 1) + NODE_H + MARGIN
    };

    svg.push_str(&format!(
        "<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 {:.0} {:.0}' style='width:100%;height:auto;' role='img' aria-label='Model diagram'>",
        width, height
    ));

    // Kind headers.
    for (kind, x) in &kind_columns {
        svg.push_str(&format!(
            "<text class='kind-header' x='{:.1}' y='{:.1}' text-anchor='middle' font-size='13' font-weight='bold' font-family='sans-serif'>{}</text>",
            *x + NODE_W / 2.0,
            MARGIN + 14.0,
            xml_escape(kind)
        ));
    }

    // Edges, sorted deterministically, behind the nodes.
    let mut edges: Vec<&GraphEdge> = graph.edges.iter().collect();
    edges.sort_by(|a, b| {
        (&a.source, &a.target, &a.kind, &a.label).cmp(&(&b.source, &b.target, &b.kind, &b.label))
    });
    // Several edges may share a source-target pair; spread them apart perpendicular to the
    // line so every one stays visible while remaining a pure function of the sorted edges.
    let mut pair_total: HashMap<(&str, &str), usize> = HashMap::new();
    for edge in &edges {
        *pair_total
            .entry((edge.source.as_str(), edge.target.as_str()))
            .or_insert(0) += 1;
    }
    let mut pair_seen: HashMap<(&str, &str), usize> = HashMap::new();
    for edge in &edges {
        let (sx, sy) = endpoint_center(&edge.source, &node_centers, &dangling_centers);
        let (tx, ty) = endpoint_center(&edge.target, &node_centers, &dangling_centers);
        let key = (edge.source.as_str(), edge.target.as_str());
        let total = pair_total[&key];
        let seen = pair_seen.entry(key).or_insert(0);
        let offset = if total > 1 {
            (*seen as f64 - (total as f64 - 1.0) / 2.0) * SPREAD
        } else {
            0.0
        };
        *seen += 1;

        if (sx - tx).abs() < 0.001 && (sy - ty).abs() < 0.001 {
            // A self-loop would be a zero-length line; draw a small loop above the node.
            let ly = sy - NODE_H / 2.0;
            svg.push_str(&format!(
                "<path class='edge' d='M {:.1} {:.1} C {:.1} {:.1} {:.1} {:.1} {:.1} {:.1}' fill='none' stroke='#a0a8b0' stroke-width='1'/>",
                sx, ly, sx - 34.0, ly - 26.0, sx + 34.0, ly - 26.0, sx, ly
            ));
            push_edge_label(&mut svg, edge, sx, ly - 30.0);
            continue;
        }

        let dx = tx - sx;
        let dy = ty - sy;
        let length = (dx * dx + dy * dy).sqrt();
        let (ox, oy) = if length > 0.0 {
            (-dy / length * offset, dx / length * offset)
        } else {
            (0.0, 0.0)
        };
        let (x1, y1) = (sx + ox, sy + oy);
        let (x2, y2) = (tx + ox, ty + oy);
        svg.push_str(&format!(
            "<line class='edge' x1='{:.1}' y1='{:.1}' x2='{:.1}' y2='{:.1}' stroke='#a0a8b0' stroke-width='1'/>",
            x1, y1, x2, y2
        ));
        push_edge_label(&mut svg, edge, (x1 + x2) / 2.0, (y1 + y2) / 2.0);
    }

    // Nodes on top of the edges.
    for (kind, _) in &kind_columns {
        let Some(nodes) = kinds.get(kind) else {
            continue;
        };
        for node in nodes {
            let (cx, cy) = node_centers[&node.id];
            let (nx, ny) = (cx - NODE_W / 2.0, cy - NODE_H / 2.0);
            let (fill, stroke) = if node.kind == "requirement" {
                ("#fff3cd", "#8a6d1a")
            } else {
                ("#eff6ff", "#2563eb")
            };
            let display_name = if node.name.is_empty() {
                node.id.as_str()
            } else {
                node.name.as_str()
            };
            svg.push_str(&format!(
                "<g class='node' data-mw-id='{}'><rect x='{:.1}' y='{:.1}' width='{:.1}' height='{:.1}' rx='6' fill='{}' stroke='{}'/><text x='{:.1}' y='{:.1}' text-anchor='middle' font-size='13' font-family='sans-serif'>{}</text>",
                xml_escape(&node.id), nx, ny, NODE_W, NODE_H, fill, stroke, cx, ny + 26.0, xml_escape(display_name)
            ));
            if !node.name.is_empty() {
                svg.push_str(&format!(
                    "<text x='{:.1}' y='{:.1}' text-anchor='middle' font-size='9' font-family='monospace' fill='#59636e'>{}</text>",
                    cx, ny + 46.0, xml_escape(&node.id)
                ));
            }
            svg.push_str("</g>");
        }
    }

    // Dangling markers on top, so the line visibly meets the marker.
    for id in &dangling_ids {
        let (cx, cy) = dangling_centers[id];
        svg.push_str(&format!(
            "<g class='dangling'><polygon points='{:.1},{:.1} {:.1},{:.1} {:.1},{:.1} {:.1},{:.1}' fill='#ffebe9' stroke='#cf222e'/><text x='{:.1}' y='{:.1}' text-anchor='middle' font-size='10' font-family='sans-serif' fill='#cf222e'>unresolved</text><text x='{:.1}' y='{:.1}' text-anchor='middle' font-size='9' font-family='monospace' fill='#cf222e'>{}</text></g>",
            cx, cy - 14.0, cx + 14.0, cy, cx, cy + 14.0, cx - 14.0, cy,
            cx, cy + 30.0,
            cx, cy + 44.0,
            xml_escape(id)
        ));
    }

    svg.push_str("</svg>");
    svg
}

fn column_x(column: usize, has_dangling: bool) -> f64 {
    let left = if has_dangling {
        MARGIN + DANGLING_W + H_GAP
    } else {
        MARGIN
    };
    left + column as f64 * (NODE_W + H_GAP)
}

fn row_y(row: usize) -> f64 {
    MARGIN + HEADER + row as f64 * (NODE_H + V_GAP)
}

/// The centre of an edge endpoint: a node when the id names a node, else its dangling
/// marker. Every endpoint is one or the other by construction, so this always resolves.
fn endpoint_center(
    id: &str,
    nodes: &HashMap<String, (f64, f64)>,
    dangling: &HashMap<String, (f64, f64)>,
) -> (f64, f64) {
    nodes
        .get(id)
        .or_else(|| dangling.get(id))
        .copied()
        .expect("every edge endpoint is a node or a dangling marker")
}

fn push_edge_label(svg: &mut String, edge: &GraphEdge, x: f64, y: f64) {
    if edge.label.is_empty() {
        return;
    }
    svg.push_str(&format!(
        "<text class='edge-label' x='{:.1}' y='{:.1}' text-anchor='middle' font-size='10' font-family='sans-serif' fill='#59636e'>{}</text>",
        x, y, xml_escape(&edge.label)
    ));
}

/// Escape a model value for XML text content (and attributes): the model is untrusted input
/// from a colleague, a supplier or an import, and a value like a script tag must render as
/// text, never as markup. Escaping ampersand, angle brackets and both quote characters is
/// valid in both contexts.
fn xml_escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

fn short_hash(hash: &str) -> &str {
    hash.get(..8).unwrap_or(hash)
}
