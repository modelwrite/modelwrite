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
use graph::routing;
use graph::symbol;
use okf::types::{Graph, GraphEdge, GraphNode, OkfRoot};

use crate::api::{load_model, map_store_error, ApiState};
use crate::auth::{identity as resolve_identity, Identity, Permission};
use crate::error::ApiError;
use crate::store::Commit;
use crate::ui::layout;
use crate::ui::model::{resolve_hash, view_branch, ModelQuery};

// ---------------------------------------------------------------------------
// Geometry constants local to the renderer. Node sizes come from the engine
// layout; these only cover the dangling-marker lane (edge routing is in the engine).
// ---------------------------------------------------------------------------

const MARGIN: f64 = 28.0;
const DANGLING_W: f64 = 180.0;
const DANGLING_H: f64 = 48.0;
const DANG_GAP: f64 = 44.0;

/// The three diagram types. The choice is kept in the URL view query like every other view.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum DiagramView {
    Structure,
    Process,
    Control,
}

impl DiagramView {
    /// The URL value of the view, carried in shareable links and in the export provenance.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            DiagramView::Structure => "structure",
            DiagramView::Process => "process",
            DiagramView::Control => "control",
        }
    }

    /// The human label of the view, for the presentation caption.
    pub(crate) fn label(self) -> &'static str {
        match self {
            DiagramView::Structure => "Structure",
            DiagramView::Process => "Process",
            DiagramView::Control => "Control",
        }
    }
}

/// The view a request resolves to: the named view when the model declares it, otherwise the
/// structure view. This is the ONE fallback the page, the SVG export and the presentation view
/// share, so the three can never disagree about which diagram a URL shows.
pub(crate) fn resolve_view(graph: &Graph, requested: Option<&str>) -> DiagramView {
    let has_process = graph_layout::process_layout(graph).is_some();
    let has_control = graph_layout::control_layout(graph).is_some();
    if requested == Some("control") && has_control {
        DiagramView::Control
    } else if requested == Some("process") && has_process {
        DiagramView::Process
    } else {
        DiagramView::Structure
    }
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
        .store_for(identity)
        .project(project)
        .map_err(map_store_error)?
        .is_none()
    {
        return Err(ApiError::not_found(format!("project {project}")));
    }
    let hash = resolve_hash(state, identity, project, query)?;
    let commit = state
        .store_for(identity)
        .commit(project, &hash)
        .map_err(map_store_error)?
        .ok_or_else(|| ApiError::not_found(format!("commit {hash}")))?;
    let root =
        load_model(state.store_for(identity).as_ref(), project, &hash).map_err(map_store_error)?;
    let mut nav = layout::Nav::load(state, identity, Some(project))?;
    nav.section = Some("diagram");
    nav.branch = Some(view_branch(query, &commit));
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
        let has_process = graph_layout::process_layout(graph).is_some();
        let has_control = graph_layout::control_layout(graph).is_some();
        let view = resolve_view(graph, query.view.as_deref());
        let svg = diagram_svg(graph, view, &SvgOptions::NONE)
            .expect("the structure view always has a layout for a graph");
        let kinds = distinct_kinds(graph);
        let unmappable = symbol::unmappable_findings(graph);
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
            (diagram_toolbar(project, commit, view, has_process, has_control, &kinds))
            div class="diagram-viewport" {
                (PreEscaped(svg))
            }
            @if !unmappable.is_empty() {
                div class="symbol-report" role="note" aria-label="Unmapped symbols" {
                    h2 { "Unmapped symbols" }
                    p { "These elements declare a symbol this build does not draw. They are shown with their kind glyph and label, never silently boxed." }
                    ul {
                        @for finding in &unmappable {
                            li { code { (finding.node_id) } " declares " code { (finding.declared) } }
                        }
                    }
                }
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
    has_control: bool,
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
    let control_href = format!(
        "{base}?commit={}&view=control",
        crate::ui::urlencode(&commit.hash)
    );
    // V1 visual outputs: the same view, at the same commit, as a download and as a chrome-free
    // presentation page. Server-rendered links, so both work with JavaScript disabled.
    let view_name = view.as_str();
    let export_href = format!(
        "{base}.svg?commit={}&view={view_name}",
        crate::ui::urlencode(&commit.hash)
    );
    let present_href = format!(
        "/ui/projects/{}/present?commit={}&view={view_name}",
        crate::ui::urlencode(project),
        crate::ui::urlencode(&commit.hash)
    );
    html! {
        div class="diagram-toolbar" {
            span class="view-toggle" role="group" aria-label="Diagram type" {
                a.view-option.current[view == DiagramView::Structure] href=(structure_href) { "Structure" }
                @if has_process {
                    a.view-option.current[view == DiagramView::Process] href=(process_href) { "Process" }
                }
                @if has_control {
                    a.view-option.current[view == DiagramView::Control] href=(control_href) { "Control" }
                }
            }
            span class="diagram-outputs" {
                a class="diagram-output present" href=(present_href) { "Present" }
                a class="diagram-output download" href=(export_href) { "Download SVG" }
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

/// Options that turn the page's inline SVG into a standalone artefact. The page passes
/// [SvgOptions::NONE], so the page's bytes are untouched; the export adds provenance, an inlined
/// stylesheet and a caption band. The caption is the artefact's FACE: the revision is readable in
/// the picture itself, not only in metadata.
#[derive(Clone, Copy)]
pub(crate) struct SvgOptions<'a> {
    /// Markup inserted right after the root svg start tag: provenance and inlined stylesheet.
    pub preface: &'a str,
    /// A caption drawn in a band beneath the drawing, stating the project and revision.
    pub caption: Option<&'a str>,
    /// Emit width/height attributes, so a downloaded file opens at a sensible size.
    pub sized: bool,
}

impl SvgOptions<'_> {
    /// The page's options: nothing added, and not sized (the page sizes the SVG through CSS).
    pub(crate) const NONE: SvgOptions<'static> = SvgOptions {
        preface: "",
        caption: None,
        sized: false,
    };
}

/// The height of the caption band the export draws beneath the drawing.
const CAPTION_H: f64 = 34.0;

/// The SVG of one view, rendered by the SAME renderer the page uses. Returns None when the model
/// does not declare that view (the process and control views are optional).
pub(crate) fn diagram_svg(
    graph: &Graph,
    view: DiagramView,
    options: &SvgOptions<'_>,
) -> Option<String> {
    let layout = match view {
        DiagramView::Structure => graph_layout::structure_layout(graph),
        DiagramView::Process => graph_layout::process_layout(graph)?,
        DiagramView::Control => graph_layout::control_layout(graph)?,
    };
    Some(render_graph_svg(graph, &layout, options))
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

fn render_graph_svg(graph: &Graph, layout: &DiagramLayout, options: &SvgOptions<'_>) -> String {
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
    // The drawing's own height; the caption band is added beneath it only for an export.
    let drawing_h = if has_dangling {
        layout.height + DANG_GAP + DANGLING_H + MARGIN
    } else {
        layout.height
    };
    let band = if options.caption.is_some() {
        CAPTION_H
    } else {
        0.0
    };
    let canvas_h = drawing_h + band;

    let mut svg = String::new();
    if options.sized {
        svg.push_str(&format!(
            "<svg xmlns='http://www.w3.org/2000/svg' width='{:.0}' height='{:.0}' viewBox='0 0 {:.0} {:.0}' preserveAspectRatio='xMidYMid meet' class='mw-diagram-svg' role='img' aria-label='Model diagram'>",
            canvas_w, canvas_h, canvas_w, canvas_h
        ));
    } else {
        svg.push_str(&format!(
            "<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 {:.0} {:.0}' preserveAspectRatio='xMidYMid meet' class='mw-diagram-svg' role='img' aria-label='Model diagram'>",
            canvas_w, canvas_h
        ));
    }
    svg.push_str(options.preface);
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
    // Route every node-to-node edge together (not one at a time) so anchors spread across a
    // side, fan-out edges share a trunk, dependency edges take their own lane, and crossings are
    // marked with a jump. The draw position doubles as the deterministic draw order.
    let mut requests: Vec<routing::EdgeRequest> = Vec::new();
    let mut route_for: HashMap<usize, usize> = HashMap::new();
    for (pos, edge) in edges.iter().enumerate() {
        let node_to_node = matches!(
            (resolve(&edge.source), resolve(&edge.target)),
            (Some(Anchor::Node(_)), Some(Anchor::Node(_)))
        );
        if !node_to_node || edge.source == edge.target {
            continue;
        }
        requests.push(routing::EdgeRequest {
            source: edge.source.clone(),
            target: edge.target.clone(),
            lane: routing::Lane::from_group(edge_group(&edge.kind)),
            index: pos,
        });
        route_for.insert(pos, requests.len() - 1);
    }
    let routed = routing::route_graph(&layout.nodes, &requests);

    for (pos, edge) in edges.iter().enumerate() {
        let (Some(src), Some(tgt)) = (resolve(&edge.source), resolve(&edge.target)) else {
            continue;
        };
        let route = route_for.get(&pos).map(|&ri| &routed[ri]);
        push_edge(&mut svg, edge, &src, &tgt, route);
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

    svg.push_str("</g>");
    if let Some(caption) = options.caption {
        svg.push_str(&format!(
            "<rect class='mw-caption-band' x='0' y='{:.0}' width='{:.0}' height='{:.0}'/>",
            drawing_h, canvas_w, CAPTION_H
        ));
        svg.push_str(&format!(
            "<text class='mw-caption' x='{:.1}' y='{:.1}' text-anchor='start'>{}</text>",
            MARGIN,
            drawing_h + 22.0,
            xml_escape(caption)
        ));
    }
    svg.push_str("</svg>");
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

fn push_edge(
    svg: &mut String,
    edge: &GraphEdge,
    src: &Anchor<'_>,
    tgt: &Anchor<'_>,
    route: Option<&routing::RoutedEdge>,
) {
    let group = edge_group(&edge.kind);
    // Dependency (Satisfy/…) edges are a secondary relationship: dashed so they read as a layer
    // beneath the flow, never confused with it.
    let dash = if group == "dependency" {
        " stroke-dasharray='5 3'"
    } else {
        ""
    };

    svg.push_str(&format!(
        "<g class='edge' data-mw-source='{}' data-mw-target='{}' data-mw-kind='{}'>",
        xml_escape(&edge.source),
        xml_escape(&edge.target),
        xml_escape(&edge.kind)
    ));

    // A self-loop, an orthogonal route between two nodes, or a straight line to a dangling marker.
    let (head_x, head_y, head_dx, head_dy, label_x, label_y) = match (src, tgt) {
        (Anchor::Node(sb), Anchor::Node(tb)) if edge.source == edge.target => {
            let cx = sb.center_x();
            let cy = sb.center_y();
            let top = cy - 24.0;
            svg.push_str(&format!(
                "<path class='mw-edge {group}' d='M {:.1} {:.1} C {:.1} {:.1} {:.1} {:.1} {:.1} {:.1}' fill='none'/>",
                cx, top, cx - 30.0, top - 24.0, cx + 30.0, top - 24.0, cx, top
            ));
            (cx, top, 0.0, 1.0, cx, top - 28.0)
        }
        (Anchor::Node(_sb), Anchor::Node(_tb)) => {
            let r = route.expect("node-to-node edge is routed");
            push_polyline(svg, &r.points, &r.jumps, group, dash);
            let last = r.points[r.points.len() - 1];
            let prev = r.points[r.points.len() - 2];
            (
                last.0,
                last.1,
                last.0 - prev.0,
                last.1 - prev.1,
                (r.points[0].0 + last.0) / 2.0,
                (r.points[0].1 + last.1) / 2.0 - 5.0,
            )
        }
        _ => {
            let (sx, sy) = anchor_point(src, tgt.center());
            let (tx, ty) = anchor_point(tgt, src.center());
            let (dx, dy) = (tx - sx, ty - sy);
            svg.push_str(&format!(
                "<line class='mw-edge {group}' x1='{:.1}' y1='{:.1}' x2='{:.1}' y2='{:.1}'{dash}/>",
                sx, sy, tx, ty
            ));
            (tx, ty, dx, dy, (sx + tx) / 2.0, (sy + ty) / 2.0 - 5.0)
        }
    };

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

/// Render an orthogonal route as a polyline, or as a path when it carries crossing jumps (no
/// fill; the stroke comes from the stylesheet).
fn push_polyline(
    svg: &mut String,
    pts: &[(f64, f64)],
    jumps: &[routing::Jump],
    group: &str,
    dash: &str,
) {
    if jumps.is_empty() {
        let points: Vec<String> = pts
            .iter()
            .map(|(x, y)| format!("{:.1},{:.1}", x, y))
            .collect();
        svg.push_str(&format!(
            "<polyline class='mw-edge {group}' points='{}' fill='none'{dash}/>",
            points.join(" ")
        ));
    } else {
        let d = polyline_path(pts, jumps);
        svg.push_str(&format!(
            "<path class='mw-edge {group}' d='{d}' fill='none'{dash}/>"
        ));
    }
}

/// Build an SVG path from an orthogonal polyline, inserting a small arc (a jump) where the edge
/// crosses another, so a crossing can never be mistaken for a junction.
fn polyline_path(pts: &[(f64, f64)], jumps: &[routing::Jump]) -> String {
    const RADIUS: f64 = 4.0;
    const HEIGHT: f64 = 8.0;
    let mut d = String::new();
    if let Some(&(x0, y0)) = pts.first() {
        d.push_str(&format!("M {:.1} {:.1}", x0, y0));
    }
    for i in 0..pts.len().saturating_sub(1) {
        let a = pts[i];
        let b = pts[i + 1];
        let mut seg: Vec<&routing::Jump> = jumps.iter().filter(|j| j.seg == i).collect();
        if (a.1 - b.1).abs() < 1e-6 {
            let dir = if b.0 >= a.0 { 1.0 } else { -1.0 };
            seg.sort_by(|x, y| {
                (dir * x.x)
                    .partial_cmp(&(dir * y.x))
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            for j in seg {
                let before = j.x - dir * RADIUS;
                let after = j.x + dir * RADIUS;
                d.push_str(&format!(" L {:.1} {:.1}", before, j.y));
                d.push_str(&format!(
                    " Q {:.1} {:.1} {:.1} {:.1}",
                    j.x,
                    j.y - HEIGHT,
                    after,
                    j.y
                ));
            }
            d.push_str(&format!(" L {:.1} {:.1}", b.0, b.1));
        } else {
            let dir = if b.1 >= a.1 { 1.0 } else { -1.0 };
            seg.sort_by(|x, y| {
                (dir * x.y)
                    .partial_cmp(&(dir * y.y))
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            for j in seg {
                let before = j.y - dir * RADIUS;
                let after = j.y + dir * RADIUS;
                d.push_str(&format!(" L {:.1} {:.1}", j.x, before));
                d.push_str(&format!(
                    " Q {:.1} {:.1} {:.1} {:.1}",
                    j.x - HEIGHT,
                    j.y,
                    j.x,
                    after
                ));
            }
            d.push_str(&format!(" L {:.1} {:.1}", b.0, b.1));
        }
    }
    d
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
    let title = format!("{display} ({})", node.id);
    // Requirements keep their historical amber fill (an existing, pinned value the server test
    // asserts); every other kind sits on the white surface, and the KIND is carried by the
    // token-coloured stroke and kind label below, never by a new stray hex.
    let fill = if node.kind == "requirement" {
        "#fff3cd"
    } else {
        "#ffffff"
    };

    // The symbol boundary decides the glyph: a declared 2525 symbol, or the kind glyph (neutral
    // default for an unknown kind). An unmappable declaration still renders - the kind glyph and
    // the label stay - but it is flagged so the report below can name it, never silently boxed.
    let resolved = symbol::symbol(&node.kind, &node.stereotypes);

    svg.push_str("<g class='node' data-mw-id='");
    svg.push_str(&xml_escape(&node.id));
    svg.push_str("' data-mw-kind='");
    svg.push_str(&xml_escape(&node.kind));
    svg.push_str("' data-mw-name='");
    svg.push_str(&xml_escape(display));
    if let symbol::Symbol::Unmappable { declared } = &resolved {
        svg.push_str("' data-mw-unmappable='");
        svg.push_str(&xml_escape(declared));
    }
    svg.push_str("' tabindex='0'>");
    svg.push_str(&format!(
        "<title>{}</title><rect class='node-rect' fill='{}' x='{:.1}' y='{:.1}' width='{:.1}' height='{:.1}' rx='6'/>",
        xml_escape(&title),
        fill,
        b.x,
        b.y,
        b.width,
        b.height
    ));

    // The glyph sits on the left inside the box; the name and kind line shift right of it and
    // read left-aligned. The text label is never replaced by the glyph.
    let text_x = match &resolved {
        symbol::Symbol::MilStd2525 {
            affiliation,
            dimension,
        } => {
            push_2525(svg, *affiliation, *dimension, b.x + 6.0, b.y + 6.0);
            b.x + 46.0
        }
        symbol::Symbol::Kind(glyph) => {
            push_kind_glyph(svg, *glyph, b.x + 7.0, b.y + 15.0);
            b.x + 30.0
        }
        symbol::Symbol::Unmappable { .. } => {
            push_kind_glyph(svg, symbol::kind_glyph(&node.kind), b.x + 7.0, b.y + 15.0);
            b.x + 30.0
        }
    };

    svg.push_str(&format!(
        "<text class='node-name' x='{:.1}' y='{:.1}' text-anchor='start'>{}</text><text class='node-kind' x='{:.1}' y='{:.1}' text-anchor='start'>{}</text></g>",
        text_x,
        b.y + 18.0,
        xml_escape(&truncated),
        text_x,
        b.y + 34.0,
        xml_escape(kind_label(&node.kind))
    ));
}

/// Draw a monochrome kind glyph: its primary path plus an optional accent, in currentColor so
/// the token palette colours it, scaled from the authored viewBox down to the rendered size.
fn push_kind_glyph(svg: &mut String, glyph: symbol::Glyph, x: f64, y: f64) {
    let scale = 18.0 / glyph.view as f64;
    svg.push_str(&format!(
        "<g class='node-glyph' transform='translate({:.2} {:.2}) scale({:.3})'>",
        x, y, scale
    ));
    svg.push_str(&format!("<path d='{}' fill='currentColor'/>", glyph.path));
    if let Some(accent) = glyph.accent {
        svg.push_str(&format!("<path d='{}' fill='currentColor'/>", accent));
    }
    svg.push_str("</g>");
}

/// Draw a declared MIL-STD-2525D symbol: the affiliation x dimension frame, filled with the
/// standard colour and stroked, plus the generic platform glyph - or a "?" for an unknown
/// affiliation, which by the standard carries no entity.
fn push_2525(
    svg: &mut String,
    affiliation: symbol::Affiliation,
    dimension: symbol::Dimension,
    x: f64,
    y: f64,
) {
    let frame = symbol::frame(affiliation, dimension);
    let scale = 36.0 / symbol::FRAME_VIEW as f64;
    let dash = if frame.dash {
        " stroke-dasharray='5 3'"
    } else {
        ""
    };
    svg.push_str(&format!(
        "<g class='node-glyph mw-2525' transform='translate({:.2} {:.2}) scale({:.3})'><path class='mw-2525-frame' d='{}' fill='{}' stroke='#1f2937' stroke-width='1.5' vector-effect='non-scaling-stroke'{} />",
        x, y, scale, frame.path, frame.fill, dash
    ));
    if affiliation == symbol::Affiliation::Unknown {
        svg.push_str(
            "<text class='mw-2525-unknown' x='50' y='66' text-anchor='middle' font-size='40'>?</text>",
        );
    } else {
        svg.push_str(&format!(
            "<path class='mw-2525-glyph' d='{}' fill='#1f2937'/>",
            symbol::platform_glyph(dimension)
        ));
    }
    svg.push_str("</g>");
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
pub(crate) fn xml_escape(value: &str) -> String {
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
