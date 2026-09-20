// SPDX-License-Identifier: AGPL-3.0-or-later
//! The diagram view: the model's graph rendered as inline SVG from a deterministic layout
//! computed in the engine (graph::layout). The SVG is COMPOSED as a string in Rust - there is
//! no drawing library and none is added - and the layout is a pure function of the model, never
//! a hand-placed drawing.
//!
//! The guarantees the rest of the slice was built around hold here by construction:
//!
//! * DETERMINISTIC: the same model always produces byte-identical SVG, because the engine
//!   layout is deterministic (sorted iteration, id tie-breaks) and everything the renderer
//!   iterates is sorted. A diagram that shuffled between loads could not be diffed or cached.
//! * COMPLETE: every node and every edge is drawn. An edge whose endpoint is not a node is drawn
//!   to an explicit "unresolved" marker rather than dropped - the two dangling satisfy links in
//!   the corpus are exactly the defect a picture should make obvious.
//! * SAFE: the SVG contains no script element and no external reference (no href, no url(...),
//!   no image or use), and every label from the model is XML-escaped, so it is safe to render
//!   anywhere and to embed in a report. Arrowheads are drawn as plain polygons, never as marker
//!   references, so the safety guarantee holds even for an edge.
//!
//! The page reads through the SAME identity, permission and project-scope decisions as the JSON
//! handlers, in the SAME order, so the workbench can never be a weaker path to the data.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use maud::{html, Markup, PreEscaped};

use graph::layout::{self as graph_layout, DiagramLayout, NodeBox};
use okf::types::{Graph, GraphEdge, GraphNode, OkfRoot};

use crate::api::{load_model, map_store_error, ApiState};
use crate::auth::{identity as resolve_identity, Identity, Permission};
use crate::error::ApiError;
use crate::store::Commit;
use crate::ui::layout;
use crate::ui::model::{resolve_hash, ModelQuery};

// ---------------------------------------------------------------------------
// Geometry constants local to the renderer. Node sizes come from the engine
// layout; these only cover the dangling-marker lane and the edge spread.
// ---------------------------------------------------------------------------

const MARGIN: f64 = 28.0;
const DANGLING_W: f64 = 180.0;
const DANGLING_H: f64 = 48.0;
const DANG_GAP: f64 = 44.0;
/// The perpendicular spread between parallel edges that share a source/target pair.
const SPREAD: f64 = 11.0;

/// The two diagram types. The choice is kept in the URL view query like every other view.
#[derive(Clone, Copy, PartialEq)]
enum DiagramView {
    Structure,
    Process,
}

/// GET /ui/projects/:project/diagram?branch=&commit=&view= - the model's graph as inline SVG.
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
        return Err(ApiError::not_found(format!("project {project}")));
    }
    let hash = resolve_hash(state, project, query)?;
    let commit = state
        .store
        .commit(project, &hash)
        .map_err(map_store_error)?
        .ok_or_else(|| ApiError::not_found(format!("commit {hash}")))?;
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
        query,
    ))
}

fn diagram_markup(
    identity: &Identity,
    mechanism: &str,
    project: &str,
    commit: &Commit,
    root: &OkfRoot,
    nav: &layout::Nav,
    query: &ModelQuery,
) -> Markup {
    let body = if let Some(graph) = &root.graph {
        let structure = structure_svg(graph);
        let process = process_svg(graph);
        let view = if query.view.as_deref() == Some("process") && process.is_some() {
            DiagramView::Process
        } else {
            DiagramView::Structure
        };
        let svg: &str = match view {
            DiagramView::Structure => structure.as_str(),
            DiagramView::Process => process.as_ref().expect("process chosen only when present"),
        };
        let kinds = distinct_kinds(graph);
        html! {
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
            (diagram_toolbar(project, commit, view, process.is_some(), &kinds))
            div class="diagram-viewport" {
                (PreEscaped(svg))
            }
            script src="/ui/app.js" {}
        }
    } else {
        html! {
            h1 { "Diagram" }
            p { "This model has no graph section, so there is nothing to draw." }
        }
    };
    let title = format!("modelwrite — {project} — diagram");
    layout::shell_with_main_class(
        &title,
        nav,
        Some(&identity.subject),
        identity.may(Permission::Administer),
        mechanism,
        "mw-diagram",
        body,
    )
}

/// The toolbar above the canvas: the view toggle (server-rendered links, so the choice works
/// without JavaScript), the zoom/fit controls and search box (wired by the enhancement), and a
/// kind filter chip per node kind present in the model.
fn diagram_toolbar(
    project: &str,
    commit: &Commit,
    view: DiagramView,
    has_process: bool,
    kinds: &[(String, usize)],
) -> Markup {
    let base = format!("/ui/projects/{}/diagram", crate::ui::urlencode(project));
    let structure_href = format!(
        "{base}?commit={}&view=structure",
        crate::ui::urlencode(&commit.hash)
    );
    let process_href = format!(
        "{base}?commit={}&view=process",
        crate::ui::urlencode(&commit.hash)
    );
    html! {
        div class="diagram-toolbar" {
            span class="view-toggle" role="group" aria-label="Diagram type" {
                a.view-option.current[view == DiagramView::Structure] href=(structure_href) { "Structure" }
                @if has_process {
                    a.view-option.current[view == DiagramView::Process] href=(process_href) { "Process" }
                }
            }
            span class="diagram-controls" {
                button type="button" class="mw-zoom-out" title="Zoom out" aria-label="Zoom out" { "\u{2212}" }
                button type="button" class="mw-zoom-in" title="Zoom in" aria-label="Zoom in" { "+" }
                button type="button" class="mw-fit" title="Fit to view" aria-label="Fit to view" { "Fit" }
                input type="search" class="mw-diagram-search" placeholder="Search name / id" aria-label="Search the diagram";
            }
            @if !kinds.is_empty() {
                span class="diagram-filters" role="group" aria-label="Filter by kind" {
                    @for (kind, count) in kinds {
                        button type="button" class="kind-filter" data-mw-kind=(kind) {
                            (kind_label(kind)) " " span class="kind-count" { (count) }
                        }
                    }
                }
            }
        }
    }
}

/// The distinct node kinds present, ordered for the filter bar with their counts.
fn distinct_kinds(graph: &Graph) -> Vec<(String, usize)> {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for node in &graph.nodes {
        *counts.entry(node.kind.clone()).or_default() += 1;
    }
    let mut kinds: Vec<(String, usize)> = counts.into_iter().collect();
    kinds.sort_by(|a, b| kind_sort_key(&a.0).cmp(&kind_sort_key(&b.0)));
    kinds
}

fn kind_sort_key(kind: &str) -> (usize, &str) {
    const ORDER: &[&str] = &[
        "block",
        "requirement",
        "activity",
        "signal",
        "interface",
        "state",
        "stateMachine",
        "actor",
        "usecase",
    ];
    let pos = ORDER.iter().position(|k| *k == kind).unwrap_or(ORDER.len());
    (pos, kind)
}

/// The human label of a node kind, for the filter chip and the node's small kind line.
fn kind_label(kind: &str) -> &str {
    match kind {
        "block" => "block",
        "requirement" => "requirement",
        "activity" => "activity",
        "signal" => "signal",
        "interface" => "interface",
        "state" => "state",
        "stateMachine" => "state machine",
        "actor" => "actor",
        "usecase" => "use case",
        other => other,
    }
}

// ---------------------------------------------------------------------------
// SVG composition.
// ---------------------------------------------------------------------------

fn structure_svg(graph: &Graph) -> String {
    let layout = graph_layout::structure_layout(graph);
    render_graph_svg(graph, &layout)
}

fn process_svg(graph: &Graph) -> Option<String> {
    let layout = graph_layout::process_layout(graph)?;
    Some(render_graph_svg(graph, &layout))
}

/// One endpoint of an edge as the renderer sees it: a placed node box, or a dangling marker.
enum Anchor<'a> {
    Node(&'a NodeBox),
    Dangling(f64, f64),
}

impl Anchor<'_> {
    fn center(&self) -> (f64, f64) {
        match self {
            Anchor::Node(b) => (b.center_x(), b.center_y()),
            Anchor::Dangling(x, y) => (*x, *y),
        }
    }
}

fn render_graph_svg(graph: &Graph, layout: &DiagramLayout) -> String {
    let node_by_id: HashMap<&str, &GraphNode> =
        graph.nodes.iter().map(|n| (n.id.as_str(), n)).collect();
    let box_by_id: HashMap<&str, &NodeBox> =
        layout.nodes.iter().map(|n| (n.id.as_str(), n)).collect();

    let mut dangling_set: BTreeSet<&str> = BTreeSet::new();
    for edge in &graph.edges {
        if !node_by_id.contains_key(edge.source.as_str()) {
            dangling_set.insert(edge.source.as_str());
        }
        if !node_by_id.contains_key(edge.target.as_str()) {
            dangling_set.insert(edge.target.as_str());
        }
    }
    let dangling_ids: Vec<String> = dangling_set.into_iter().map(|s| s.to_string()).collect();
    let has_dangling = !dangling_ids.is_empty();

    let mut dangling_centers: HashMap<String, (f64, f64)> = HashMap::new();
    let band_y = layout.height + DANG_GAP + DANGLING_H / 2.0;
    let mut cursor = MARGIN;
    for id in &dangling_ids {
        dangling_centers.insert(id.clone(), (cursor + DANGLING_W / 2.0, band_y));
        cursor += DANGLING_W + 24.0;
    }
    let canvas_w = layout.width.max(cursor + MARGIN - 24.0);
    let canvas_h = if has_dangling {
        layout.height + DANG_GAP + DANGLING_H + MARGIN
    } else {
        layout.height
    };

    let mut svg = String::new();
    svg.push_str(&format!(
        "<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 {:.0} {:.0}' preserveAspectRatio='xMidYMid meet' class='mw-diagram-svg' role='img' aria-label='Model diagram'>",
        canvas_w, canvas_h
    ));
    svg.push_str("<g class='mw-transform'>");

    let mut edges: Vec<&GraphEdge> = graph.edges.iter().collect();
    edges.sort_by(|a, b| {
        (&a.source, &a.target, &a.kind, &a.label).cmp(&(&b.source, &b.target, &b.kind, &b.label))
    });
    let resolve = |id: &str| -> Option<Anchor<'_>> {
        if let Some(b) = box_by_id.get(id) {
            Some(Anchor::Node(b))
        } else if let Some((x, y)) = dangling_centers.get(id) {
            Some(Anchor::Dangling(*x, *y))
        } else {
            None
        }
    };
    let mut pair_total: HashMap<(&str, &str), usize> = HashMap::new();
    for edge in &edges {
        if resolve(&edge.source).is_some() && resolve(&edge.target).is_some() {
            *pair_total
                .entry((edge.source.as_str(), edge.target.as_str()))
                .or_insert(0) += 1;
        }
    }
    let mut pair_seen: HashMap<(&str, &str), usize> = HashMap::new();
    for edge in &edges {
        let (Some(src), Some(tgt)) = (resolve(&edge.source), resolve(&edge.target)) else {
            continue;
        };
        let key = (edge.source.as_str(), edge.target.as_str());
        let total = pair_total[&key];
        let seen = pair_seen.entry(key).or_insert(0);
        let offset = if total > 1 {
            (*seen as f64 - (total as f64 - 1.0) / 2.0) * SPREAD
        } else {
            0.0
        };
        *seen += 1;
        push_edge(&mut svg, edge, &src, &tgt, offset);
    }

    for node_box in &layout.nodes {
        let node = node_by_id
            .get(node_box.id.as_str())
            .expect("placed node exists in the graph");
        push_node(&mut svg, node, node_box);
    }

    for id in &dangling_ids {
        let (cx, cy) = dangling_centers[id];
        push_dangling(&mut svg, id, cx, cy);
    }

    svg.push_str("</g></svg>");
    svg
}

fn edge_group(kind: &str) -> &'static str {
    match kind {
        "part" | "contains" => "containment",
        "dependency" => "dependency",
        "include" | "triggers" | "transition" => "flow",
        _ => "neutral",
    }
}

/// The point on an anchor's boundary facing toward - the box border for a node, the top edge for
/// a dangling marker - so edges run border-to-border rather than centre-to-centre.
fn anchor_point(anchor: &Anchor<'_>, toward: (f64, f64)) -> (f64, f64) {
    match anchor {
        Anchor::Node(b) => border_point(b, toward),
        Anchor::Dangling(x, y) => (*x, *y - DANGLING_H / 2.0),
    }
}

fn border_point(b: &NodeBox, toward: (f64, f64)) -> (f64, f64) {
    let cx = b.center_x();
    let cy = b.center_y();
    let dx = toward.0 - cx;
    let dy = toward.1 - cy;
    if dx.abs() < 1e-6 && dy.abs() < 1e-6 {
        return (cx, cy - b.height / 2.0);
    }
    let hw = b.width / 2.0;
    let hh = b.height / 2.0;
    let sx = if dx.abs() < 1e-6 {
        f64::INFINITY
    } else {
        hw / dx.abs()
    };
    let sy = if dy.abs() < 1e-6 {
        f64::INFINITY
    } else {
        hh / dy.abs()
    };
    let s = sx.min(sy);
    (cx + dx * s, cy + dy * s)
}

fn push_edge(svg: &mut String, edge: &GraphEdge, src: &Anchor<'_>, tgt: &Anchor<'_>, offset: f64) {
    let group = edge_group(&edge.kind);
    let (sx, sy) = anchor_point(src, tgt.center());
    let (tx, ty) = anchor_point(tgt, src.center());

    let dx = tx - sx;
    let dy = ty - sy;
    let length = (dx * dx + dy * dy).sqrt().max(1e-6);
    let (ox, oy) = if offset != 0.0 {
        (-dy / length * offset, dx / length * offset)
    } else {
        (0.0, 0.0)
    };
    let (x1, y1) = (sx + ox, sy + oy);
    let (x2, y2) = (tx + ox, ty + oy);

    let (label_x, label_y, head_x, head_y, head_dx, head_dy, line) = if edge.source == edge.target
        && matches!(src, Anchor::Node(_))
        && (sx - tx).abs() < 0.5
        && (sy - ty).abs() < 0.5
    {
        let (cx, cy) = src.center();
        let top = cy - 24.0;
        let path = format!(
            "<path class='mw-edge {group}' d='M {:.1} {:.1} C {:.1} {:.1} {:.1} {:.1} {:.1} {:.1}' fill='none'/>",
            cx, top, cx - 30.0, top - 24.0, cx + 30.0, top - 24.0, cx, top
        );
        (cx, top - 28.0, cx, top, 0.0, 1.0, path)
    } else {
        let l = format!(
            "<line class='mw-edge {group}' x1='{:.1}' y1='{:.1}' x2='{:.1}' y2='{:.1}'/>",
            x1, y1, x2, y2
        );
        ((x1 + x2) / 2.0, (y1 + y2) / 2.0 - 5.0, x2, y2, dx, dy, l)
    };

    svg.push_str(&format!(
        "<g class='edge' data-mw-source='{}' data-mw-target='{}' data-mw-kind='{}'>{line}",
        xml_escape(&edge.source),
        xml_escape(&edge.target),
        xml_escape(&edge.kind)
    ));
    push_arrowhead(svg, head_x, head_y, head_dx, head_dy, group);
    if !edge.label.is_empty() {
        svg.push_str(&format!(
            "<text class='edge-label' x='{:.1}' y='{:.1}' text-anchor='middle'>{}</text>",
            label_x,
            label_y,
            xml_escape(&edge.label)
        ));
    }
    svg.push_str("</g>");
}

/// Draw an explicit arrowhead triangle at the tip, oriented along the direction of travel. The
/// arrow is a plain polygon (filled through the stylesheet's design tokens), never a marker
/// reference, so the SVG still contains no url() and no external reference.
fn push_arrowhead(svg: &mut String, tip_x: f64, tip_y: f64, dir_x: f64, dir_y: f64, group: &str) {
    let len = (dir_x * dir_x + dir_y * dir_y).sqrt().max(1e-6);
    let ux = dir_x / len;
    let uy = dir_y / len;
    let px = -uy;
    let py = ux;
    let base_x = tip_x - ux * 10.0;
    let base_y = tip_y - uy * 10.0;
    let half = 5.0;
    let lx = base_x + px * half;
    let ly = base_y + py * half;
    let rx = base_x - px * half;
    let ry = base_y - py * half;
    svg.push_str(&format!(
        "<polygon class='arrow-{group}' points='{:.1},{:.1} {:.1},{:.1} {:.1},{:.1}'/>",
        tip_x, tip_y, lx, ly, rx, ry
    ));
}

fn push_node(svg: &mut String, node: &GraphNode, b: &NodeBox) {
    let display = if node.name.is_empty() {
        &node.id
    } else {
        &node.name
    };
    let truncated = graph_layout::truncate_label(display, b.width);
    let cx = b.center_x();
    let title = format!("{display} ({})", node.id);
    // Requirements keep their historical amber fill (an existing, pinned value the server test
    // asserts); every other kind sits on the white surface, and the KIND is carried by the
    // token-coloured stroke and kind label below, never by a new stray hex.
    let fill = if node.kind == "requirement" {
        "#fff3cd"
    } else {
        "#ffffff"
    };
    svg.push_str(&format!(
        "<g class='node' data-mw-id='{}' data-mw-kind='{}' data-mw-name='{}' tabindex='0'><title>{}</title><rect class='node-rect' fill='{}' x='{:.1}' y='{:.1}' width='{:.1}' height='{:.1}' rx='6'/><text class='node-name' x='{:.1}' y='{:.1}' text-anchor='middle'>{}</text><text class='node-kind' x='{:.1}' y='{:.1}' text-anchor='middle'>{}</text></g>",
        xml_escape(&node.id),
        xml_escape(&node.kind),
        xml_escape(display),
        xml_escape(&title),
        fill,
        b.x,
        b.y,
        b.width,
        b.height,
        cx,
        b.y + 18.0,
        xml_escape(&truncated),
        cx,
        b.y + 34.0,
        xml_escape(kind_label(&node.kind))
    ));
}

fn push_dangling(svg: &mut String, id: &str, cx: f64, cy: f64) {
    svg.push_str(&format!(
        "<g class='dangling'><polygon class='dangling-shape' points='{:.1},{:.1} {:.1},{:.1} {:.1},{:.1} {:.1},{:.1}'/><text class='dangling-label' x='{:.1}' y='{:.1}' text-anchor='middle'>unresolved</text><text class='dangling-id' x='{:.1}' y='{:.1}' text-anchor='middle'>{}</text></g>",
        cx,
        cy - 14.0,
        cx + 14.0,
        cy,
        cx,
        cy + 14.0,
        cx - 14.0,
        cy,
        cx,
        cy + 30.0,
        cx,
        cy + 44.0,
        xml_escape(id)
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
