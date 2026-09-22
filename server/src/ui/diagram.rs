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
        let (svg, unplaced) = diagram_svg_with_unplaced(graph, view, &SvgOptions::NONE)
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
            (diagram_export_scopes(project, commit, view, graph))
            div class="diagram-viewport" {
                (PreEscaped(svg))
            }
            @if !unplaced.is_empty() {
                div class="symbol-report" role="note" aria-label="Unplaceable relationship names" {
                    h2 { "Relationship names that could not be placed" }
                    p {
                        "There was no clear space on this drawing for " (unplaced.len())
                        " relationship name(s). They are named here rather than dropped in "
                        "silence, and each relationship is still listed on the page of the "
                        "element it belongs to."
                    }
                    ul {
                        @for name in &unplaced {
                            li { code { (name) } }
                        }
                    }
                }
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

/// The export scopes, ON THE PAGE. The whole model is complete and, at slide scale, unreadable,
/// so what to draw belongs to the reader - and each option is SIZED here, in the same numbers the
/// exported file will have, because "export by kind" is only a choice if a person can see which
/// kind produces a picture they can read.
fn diagram_export_scopes(
    project: &str,
    commit: &Commit,
    view: DiagramView,
    graph: &Graph,
) -> Markup {
    let options = scope_options(graph, view);
    let base = format!(
        "/ui/projects/{}/diagram.svg?commit={}&view={}",
        crate::ui::urlencode(project),
        crate::ui::urlencode(&commit.hash),
        view.as_str()
    );
    let full_px = options
        .first()
        .map(|option| option.label_px)
        .unwrap_or_default();
    let elements = elements_in_picker_order(graph);
    let hops: Vec<(usize, String)> = (1..=MAX_HOPS)
        .map(|radius| {
            let label = if radius == 1 {
                "1 relationship".to_string()
            } else {
                format!("{radius} relationships")
            };
            (radius, label)
        })
        .collect();
    html! {
        section class="diagram-exports" aria-label="Export a drawing" {
            h2 { "Export a drawing" }
            p class="diagram-exports-note" {
                "The full drawing is complete and correct, and on a 1920×1080 slide its labels land "
                "at " (format!("{full_px:.1}")) " px — nobody can read that. Narrow the drawing to "
                "what the report or the review is about. Narrowing usually buys label size, but not "
                "always: the canvas height follows how DEEP the structure is, not how many boxes it "
                "has, so every option below is measured on the real layout and the ones that are "
                "still unreadable say so. Every export also names its scope and its counts on its "
                "face, so a filtered picture can never be mistaken for the whole model."
            }
            ul class="export-scopes" {
                @for option in &options {
                    li class="export-scope" {
                        a class="export-scope-link" href={ (base) "&" (option.scoped.scope.query()) } {
                            (scope_title(option))
                        }
                        span class="export-scope-detail" { " — " (scope_detail(option)) }
                    }
                }
            }
            form class="export-neighbourhood" method="get"
                 action={ "/ui/projects/" (crate::ui::urlencode(project)) "/diagram.svg" } {
                h3 { "One element and its neighbourhood" }
                p {
                    "Draws the element you pick and everything within the radius you pick, in either "
                    "direction along every relationship. This is the \"show me this part of the model\" "
                    "export a review uses."
                }
                input type="hidden" name="commit" value=(commit.hash);
                input type="hidden" name="view" value=(view.as_str());
                input type="hidden" name="scope" value="neighbourhood";
                label { "Element "
                    select name="element" {
                        @for node in &elements {
                            option value=(node.id) {
                                (display_name(node)) " — " (kind_label(&node.kind))
                            }
                        }
                    }
                }
                label { "within "
                    select name="hops" {
                        @for (radius, label) in &hops {
                            option value=(radius) selected[*radius == DEFAULT_HOPS] { (label) }
                        }
                    }
                }
                button type="submit" { "Download the neighbourhood" }
            }
        }
    }
}

/// The name of a node as a person reads it: its name, or its id when it has none.
fn display_name(node: &GraphNode) -> &str {
    if node.name.is_empty() {
        &node.id
    } else {
        &node.name
    }
}

/// What a scope option draws, as the link a person clicks.
fn scope_title(option: &ScopeOption) -> String {
    let scoped = &option.scoped;
    if scoped.is_full() {
        "the whole model (complete)".to_string()
    } else {
        scoped.label()
    }
}

/// What a scope option gives you: the counts, and the label size it lands at on a slide. The
/// measure is stated as a number because that is the whole point of offering the option.
fn scope_detail(option: &ScopeOption) -> String {
    let readability = if option.legible {
        "readable"
    } else {
        "still too small to read"
    };
    format!(
        "{} · labels {:.1} px at slide scale — {readability}",
        option.scoped.counts(),
        option.label_px
    )
}

// ---------------------------------------------------------------------------
// Scoping: how much of the model a drawing shows.
//
// The full drawing is complete and correct - and for a model of any size it is also unreadable
// at slide scale. Ninety-nine labelled boxes on one canvas land their labels at single-digit
// pixels, which is the same as exporting nothing. A scope is a filter on the LAYOUT INPUT: the
// same renderer, the same determinism, fewer elements, a smaller canvas, larger labels. Every
// scope also carries its counts, because a filtered picture that did not say so could be
// mistaken for the whole model.
// ---------------------------------------------------------------------------

/// The frame an exported picture is most often read in: one 1920x1080 slide.
pub(crate) const SLIDE_W: f64 = 1920.0;
pub(crate) const SLIDE_H: f64 = 1080.0;
/// The node label size in user units, as the ONE stylesheet declares it
/// (`svg g.node .node-name { font-size: 13px }` in ui/layout.rs). A test pins the two together,
/// so this measure cannot silently drift from the type the pages actually set.
pub(crate) const NODE_LABEL_PX: f64 = 13.0;
/// A label below this lands too small to read on the slide it was put on.
pub(crate) const LEGIBLE_LABEL_PX: f64 = 11.0;
/// The default and maximum neighbourhood radius, in relationships.
pub(crate) const DEFAULT_HOPS: usize = 1;
pub(crate) const MAX_HOPS: usize = 3;
/// How many containment branches the diagram page offers as one-click exports.
const CONTAINMENT_LINKS: usize = 6;

/// The size a node label lands at when a drawing of this canvas is fitted into one slide. This is
/// the honest measure of "can a person read it": it is the label's own size scaled by exactly the
/// factor the whole drawing is scaled by, so it is computable without rendering anything.
pub(crate) fn slide_label_px(canvas_w: f64, canvas_h: f64) -> f64 {
    if canvas_w <= 0.0 || canvas_h <= 0.0 {
        return 0.0;
    }
    NODE_LABEL_PX * (SLIDE_W / canvas_w).min(SLIDE_H / canvas_h)
}

/// Whether a drawing of this canvas keeps its labels readable on a slide.
pub(crate) fn legible_at_slide_scale(canvas_w: f64, canvas_h: f64) -> bool {
    slide_label_px(canvas_w, canvas_h) >= LEGIBLE_LABEL_PX
}

/// A scope that names something the model does not define is a REQUEST error, not a drawing
/// error: the two are kept apart so the handler can answer 404 or 400 correctly.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ScopeError {
    /// The scope centres on an element this model does not have.
    UnknownElement(String),
    /// The scope asks for a kind this model has no elements of.
    NoSuchKind(String),
}

impl ScopeError {
    pub(crate) fn message(&self) -> String {
        match self {
            ScopeError::UnknownElement(id) => format!("this model has no element {id}"),
            ScopeError::NoSuchKind(kind) => format!("this model has no {kind} elements"),
        }
    }
}

/// The part of a model a drawing shows.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum DiagramScope {
    /// Every element and every relationship: the only COMPLETE scope.
    Full,
    /// Only these element kinds, and the relationships among them. The kind list is kept sorted
    /// and deduplicated, so two addresses that mean the same scope export the same bytes.
    Kinds(Vec<String>),
    /// One element and everything within `hops` relationships of it, in either direction.
    Neighbourhood { element: String, hops: usize },
    /// One container and everything it contains, through part/contains edges.
    Containment { element: String },
}

/// The element a scope is centred on, resolved when the scope is applied so the caption can name
/// what the reader is looking at rather than only which id was asked for.
#[derive(Debug, Clone)]
pub(crate) struct Focus {
    pub name: String,
    pub id: String,
}

impl Focus {
    /// The element as a person reads it: its name, with its id when the two differ.
    fn label(&self) -> String {
        if self.name == self.id {
            self.id.clone()
        } else {
            format!("{} ({})", self.name, self.id)
        }
    }
}

/// The drawing a scope resolves to: the sub-graph to draw, and what it is a subset of. The counts
/// travel with the drawing because they are what makes a filtered picture honest.
#[derive(Debug, Clone)]
pub(crate) struct ScopedGraph {
    pub graph: Graph,
    pub scope: DiagramScope,
    /// The element the scope is centred on, for the element-centred scopes.
    pub focus: Option<Focus>,
    pub elements: usize,
    pub relationships: usize,
    pub total_elements: usize,
    pub total_relationships: usize,
}

impl ScopedGraph {
    /// The whole model as a scope: what the diagram page and the presentation view draw.
    pub(crate) fn full(graph: &Graph) -> ScopedGraph {
        DiagramScope::Full
            .apply(graph)
            .expect("the full scope always applies")
    }

    /// The human phrase for a caption: what the picture is of.
    pub(crate) fn label(&self) -> String {
        match &self.scope {
            DiagramScope::Full => "the whole model".to_string(),
            DiagramScope::Kinds(kinds) => format!(
                "only {} elements",
                kinds
                    .iter()
                    .map(|kind| kind_label(kind))
                    .collect::<Vec<_>>()
                    .join(" + ")
            ),
            DiagramScope::Neighbourhood { hops, .. } => format!(
                "{} and everything within {hops} relationship{}",
                self.focus_label(),
                if *hops == 1 { "" } else { "s" }
            ),
            DiagramScope::Containment { .. } => {
                format!("{} and its containment descendants", self.focus_label())
            }
        }
    }

    /// The centred element, or the id the scope was asked for when it could not be named.
    fn focus_label(&self) -> String {
        match &self.focus {
            Some(focus) => focus.label(),
            None => match &self.scope {
                DiagramScope::Neighbourhood { element, .. }
                | DiagramScope::Containment { element } => element.clone(),
                _ => String::new(),
            },
        }
    }

    /// The counts as the caption states them: "42/99 elements, 60/165 relationships".
    pub(crate) fn counts(&self) -> String {
        format!(
            "{}/{} elements, {}/{} relationships",
            self.elements, self.total_elements, self.relationships, self.total_relationships
        )
    }

    /// Whether this drawing is the whole model. Only this one may be called complete.
    pub(crate) fn is_full(&self) -> bool {
        self.scope == DiagramScope::Full
    }
}

impl DiagramScope {
    /// The canonical token, written into the provenance line, the metadata element and the file
    /// name. One spelling, so a scope can be named, quoted and grepped.
    pub(crate) fn as_str(&self) -> String {
        match self {
            DiagramScope::Full => "full".to_string(),
            DiagramScope::Kinds(kinds) => format!("kinds:{}", kinds.join(",")),
            DiagramScope::Neighbourhood { element, hops } => {
                format!("neighbourhood:{element}:{hops}")
            }
            DiagramScope::Containment { element } => format!("containment:{element}"),
        }
    }

    /// The query fragment that addresses this scope, so a link and the filter it reaches can
    /// never disagree about what was asked for.
    pub(crate) fn query(&self) -> String {
        match self {
            DiagramScope::Full => "scope=full".to_string(),
            DiagramScope::Kinds(kinds) => format!("scope=kinds&kinds={}", kinds.join(",")),
            DiagramScope::Neighbourhood { element, hops } => format!(
                "scope=neighbourhood&element={}&hops={hops}",
                crate::ui::urlencode(element)
            ),
            DiagramScope::Containment { element } => format!(
                "scope=containment&element={}",
                crate::ui::urlencode(element)
            ),
        }
    }

    /// Resolve this scope against a model: the sub-graph to draw, plus the counts that keep a
    /// filtered picture honest. Deterministic by construction - the subsets are filtered out of
    /// the model's own node and edge order, so the same scope always yields the same bytes.
    pub(crate) fn apply(&self, graph: &Graph) -> Result<ScopedGraph, ScopeError> {
        let total_elements = graph.nodes.len();
        let total_relationships = graph.edges.len();
        let named = |id: &str| -> Result<Focus, ScopeError> {
            let node = graph
                .nodes
                .iter()
                .find(|node| node.id == id)
                .ok_or_else(|| ScopeError::UnknownElement(id.to_string()))?;
            let name = if node.name.is_empty() {
                node.id.clone()
            } else {
                node.name.clone()
            };
            Ok(Focus {
                name,
                id: node.id.clone(),
            })
        };

        // The WHOLE MODEL is drawn exactly as the model is: no filtering at all, so nothing -
        // not even an edge whose endpoint the model does not define - can be lost to the scope
        // machinery. Completeness is a tested property of the full export and stays one.
        let mut focus: Option<Focus> = None;
        let keep: Option<BTreeSet<&str>> = match self {
            DiagramScope::Full => None,
            DiagramScope::Kinds(kinds) => {
                let present: BTreeSet<&str> = graph.nodes.iter().map(|n| n.kind.as_str()).collect();
                for kind in kinds {
                    if !present.contains(kind.as_str()) {
                        return Err(ScopeError::NoSuchKind(kind.clone()));
                    }
                }
                let wanted: BTreeSet<&str> = kinds.iter().map(String::as_str).collect();
                Some(
                    graph
                        .nodes
                        .iter()
                        .filter(|node| wanted.contains(node.kind.as_str()))
                        .map(|node| node.id.as_str())
                        .collect(),
                )
            }
            DiagramScope::Neighbourhood { element, hops } => {
                let adjacency = adjacency(graph);
                let mut seen: BTreeSet<&str> = BTreeSet::new();
                seen.insert(element.as_str());
                let mut frontier: Vec<&str> = vec![element.as_str()];
                for _ in 0..(*hops).min(MAX_HOPS) {
                    let mut next: Vec<&str> = Vec::new();
                    for id in &frontier {
                        for other in adjacency.get(id).map(Vec::as_slice).unwrap_or(&[]) {
                            if seen.insert(other) {
                                next.push(other);
                            }
                        }
                    }
                    if next.is_empty() {
                        break;
                    }
                    frontier = next;
                }
                focus = Some(named(element)?);
                Some(seen)
            }
            DiagramScope::Containment { element } => {
                let children = containment_children(graph);
                let mut seen: BTreeSet<&str> = BTreeSet::new();
                seen.insert(element.as_str());
                let mut frontier: Vec<&str> = vec![element.as_str()];
                while let Some(id) = frontier.pop() {
                    for child in children.get(id).map(Vec::as_slice).unwrap_or(&[]) {
                        if seen.insert(child) {
                            frontier.push(child);
                        }
                    }
                }
                focus = Some(named(element)?);
                Some(seen)
            }
        };

        // The scope a drawing is of, or the whole model untouched.
        let (nodes, edges): (Vec<GraphNode>, Vec<GraphEdge>) = match &keep {
            None => (graph.nodes.clone(), graph.edges.clone()),
            Some(keep) => (
                graph
                    .nodes
                    .iter()
                    .filter(|node| keep.contains(node.id.as_str()))
                    .cloned()
                    .collect(),
                // Edges survive only when BOTH endpoints do. An edge with one endpoint filtered out
                // would otherwise be drawn as an unresolved marker, which would be an artefact of
                // the scope rather than a fact about the model.
                graph
                    .edges
                    .iter()
                    .filter(|edge| {
                        keep.contains(edge.source.as_str()) && keep.contains(edge.target.as_str())
                    })
                    .cloned()
                    .collect(),
            ),
        };
        Ok(ScopedGraph {
            elements: nodes.len(),
            relationships: edges.len(),
            graph: Graph { nodes, edges },
            scope: self.clone(),
            focus,
            total_elements,
            total_relationships,
        })
    }
}

/// Undirected adjacency over every relationship, restricted to endpoints the model defines.
fn adjacency(graph: &Graph) -> BTreeMap<&str, Vec<&str>> {
    let ids: BTreeSet<&str> = graph.nodes.iter().map(|node| node.id.as_str()).collect();
    let mut adjacency: BTreeMap<&str, Vec<&str>> = graph
        .nodes
        .iter()
        .map(|node| (node.id.as_str(), Vec::new()))
        .collect();
    for edge in &graph.edges {
        let (source, target) = (edge.source.as_str(), edge.target.as_str());
        if source == target || !ids.contains(source) || !ids.contains(target) {
            continue;
        }
        adjacency.entry(source).or_default().push(target);
        adjacency.entry(target).or_default().push(source);
    }
    adjacency
}

/// The containment children of each element: part/contains edges point container -> contained.
fn containment_children(graph: &Graph) -> BTreeMap<&str, Vec<&str>> {
    let ids: BTreeSet<&str> = graph.nodes.iter().map(|node| node.id.as_str()).collect();
    let mut children: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for edge in &graph.edges {
        if !matches!(edge.kind.as_str(), "part" | "contains") {
            continue;
        }
        let (source, target) = (edge.source.as_str(), edge.target.as_str());
        if !ids.contains(source) || !ids.contains(target) {
            continue;
        }
        children.entry(source).or_default().push(target);
    }
    children
}

/// The containers that hold something, most-populated branch first: the branches a reviewer can
/// export in one click. Ties break on id, so the list is stable from load to load.
fn containment_roots(graph: &Graph) -> Vec<(String, usize)> {
    let children = containment_children(graph);
    let mut roots: Vec<(String, usize)> = children
        .keys()
        .map(|root| {
            let mut seen: BTreeSet<&str> = BTreeSet::new();
            let mut frontier: Vec<&str> = vec![root];
            while let Some(id) = frontier.pop() {
                for child in children.get(id).map(Vec::as_slice).unwrap_or(&[]) {
                    if child != root && seen.insert(child) {
                        frontier.push(child);
                    }
                }
            }
            ((*root).to_string(), seen.len())
        })
        .collect();
    roots.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    roots
}

/// A scope offered on the diagram page, SIZED: what it draws and how large its labels land on a
/// slide. The page offers only scopes the view can actually draw.
pub(crate) struct ScopeOption {
    pub scoped: ScopedGraph,
    /// The node label size this drawing lands at when fitted into one slide.
    pub label_px: f64,
    /// Whether that size is readable.
    pub legible: bool,
}

/// Every scope the diagram page offers for one drawing: the whole model first, then each kind,
/// then the kinds a review usually asks for together, then the most-populated containment
/// branches. Each is sized through the REAL layout, so the numbers on the page are the numbers
/// the exported file will have.
pub(crate) fn scope_options(graph: &Graph, view: DiagramView) -> Vec<ScopeOption> {
    let mut scopes: Vec<DiagramScope> = vec![DiagramScope::Full];
    if view == DiagramView::Structure {
        for (kind, _) in distinct_kinds(graph) {
            scopes.push(DiagramScope::Kinds(vec![kind]));
        }
        let present = |kind: &str| graph.nodes.iter().any(|node| node.kind == kind);
        if present("block") && present("requirement") {
            scopes.push(DiagramScope::Kinds(vec![
                "block".to_string(),
                "requirement".to_string(),
            ]));
        }
        for (root, _) in containment_roots(graph).into_iter().take(CONTAINMENT_LINKS) {
            scopes.push(DiagramScope::Containment { element: root });
        }
    }
    scopes
        .into_iter()
        .filter_map(|scope| {
            let scoped = scope.apply(graph).ok()?;
            let layout = layout_for(&scoped.graph, view)?;
            let (width, height) = canvas_extent(&scoped.graph, &layout, true);
            Some(ScopeOption {
                label_px: slide_label_px(width, height),
                legible: legible_at_slide_scale(width, height),
                scoped,
            })
        })
        .collect()
}

/// Every element of a drawing, ordered the way the picker lists them: the filter bar's kind
/// order first, then name, then id, so the list is stable and groups like with like.
pub(crate) fn elements_in_picker_order(graph: &Graph) -> Vec<&GraphNode> {
    let mut nodes: Vec<&GraphNode> = graph.nodes.iter().collect();
    nodes.sort_by(|a, b| {
        kind_sort_key(&a.kind)
            .cmp(&kind_sort_key(&b.kind))
            .then_with(|| a.name.cmp(&b.name))
            .then_with(|| a.id.cmp(&b.id))
    });
    nodes
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

/// The layout of one view, when the model declares it. The structure view always exists for a
/// graph; the process and control views are optional, and a scope that leaves the model nothing
/// for one of them to draw is a scope that view cannot offer.
pub(crate) fn layout_for(graph: &Graph, view: DiagramView) -> Option<DiagramLayout> {
    match view {
        DiagramView::Structure => Some(graph_layout::structure_layout(graph)),
        DiagramView::Process => graph_layout::process_layout(graph),
        DiagramView::Control => graph_layout::control_layout(graph),
    }
}

/// The SVG of one view, rendered by the SAME renderer the page uses. Returns None when the model
/// does not declare that view (the process and control views are optional).
pub(crate) fn diagram_svg(
    graph: &Graph,
    view: DiagramView,
    options: &SvgOptions<'_>,
) -> Option<String> {
    let layout = layout_for(graph, view)?;
    Some(render_graph_svg(graph, &layout, options).0)
}

/// The same drawing, plus the names of any relationship labels the placement could not find a
/// readable home for. The page draws a note for each of them: a relationship name that is not on
/// the drawing is a relationship nobody can check, so it is never dropped without being said.
pub(crate) fn diagram_svg_with_unplaced(
    graph: &Graph,
    view: DiagramView,
    options: &SvgOptions<'_>,
) -> Option<(String, Vec<String>)> {
    let layout = layout_for(graph, view)?;
    Some(render_graph_svg(graph, &layout, options))
}

/// The canvas a drawing occupies: the layout extent, a lane for dangling-endpoint markers when an
/// edge names a node the model does not define, and - for an export - the caption band. Split out
/// of [render_graph_svg] so a caller can SIZE a drawing before drawing it: legibility is a
/// property of the canvas, and a scope is chosen on that measure.
struct Canvas {
    width: f64,
    height: f64,
    /// The drawing's own height, without the caption band.
    drawing_h: f64,
    /// The dangling markers: their id and centre, in the order they are drawn.
    dangling: Vec<(String, (f64, f64))>,
}

fn canvas_for(graph: &Graph, layout: &DiagramLayout, caption: bool) -> Canvas {
    let node_ids: BTreeSet<&str> = graph.nodes.iter().map(|node| node.id.as_str()).collect();
    let mut dangling_ids: BTreeSet<&str> = BTreeSet::new();
    for edge in &graph.edges {
        for endpoint in [edge.source.as_str(), edge.target.as_str()] {
            if !node_ids.contains(endpoint) {
                dangling_ids.insert(endpoint);
            }
        }
    }

    let band_y = layout.height + DANG_GAP + DANGLING_H / 2.0;
    let mut cursor = MARGIN;
    let mut dangling: Vec<(String, (f64, f64))> = Vec::new();
    for id in dangling_ids {
        dangling.push((id.to_string(), (cursor + DANGLING_W / 2.0, band_y)));
        cursor += DANGLING_W + 24.0;
    }
    let width = layout.width.max(cursor + MARGIN - 24.0);
    let drawing_h = if dangling.is_empty() {
        layout.height
    } else {
        layout.height + DANG_GAP + DANGLING_H + MARGIN
    };
    let band = if caption { CAPTION_H } else { 0.0 };
    Canvas {
        width,
        height: drawing_h + band,
        drawing_h,
        dangling,
    }
}

/// The canvas a drawing of this graph would occupy, without drawing it. This is what the scoping
/// options are measured on, and what the export is measured against when it says how readable it
/// is at slide scale.
pub(crate) fn canvas_extent(graph: &Graph, layout: &DiagramLayout, caption: bool) -> (f64, f64) {
    let canvas = canvas_for(graph, layout, caption);
    (canvas.width, canvas.height)
}

/// The canvas a drawing of this model, in this view, would occupy - or None when the view has
/// nothing to draw. The exported file states the label size this canvas lands at on a slide, so
/// the measurement is taken from the same arithmetic the renderer uses.
pub(crate) fn drawing_canvas(
    graph: &Graph,
    view: DiagramView,
    caption: bool,
) -> Option<(f64, f64)> {
    let layout = layout_for(graph, view)?;
    Some(canvas_extent(graph, &layout, caption))
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

fn render_graph_svg(
    graph: &Graph,
    layout: &DiagramLayout,
    options: &SvgOptions<'_>,
) -> (String, Vec<String>) {
    let node_by_id: HashMap<&str, &GraphNode> =
        graph.nodes.iter().map(|n| (n.id.as_str(), n)).collect();
    let box_by_id: HashMap<&str, &NodeBox> =
        layout.nodes.iter().map(|n| (n.id.as_str(), n)).collect();

    let canvas = canvas_for(graph, layout, options.caption.is_some());
    let canvas_w = canvas.width;
    let canvas_h = canvas.height;
    let drawing_h = canvas.drawing_h;
    let dangling_ids: Vec<String> = canvas.dangling.iter().map(|(id, _)| id.clone()).collect();
    let dangling_centers: HashMap<&str, (f64, f64)> = canvas
        .dangling
        .iter()
        .map(|(id, center)| (id.as_str(), *center))
        .collect();

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

    // The line each edge actually draws, so the label placer reasons about the drawing the reader
    // sees rather than about the endpoints. A node-to-node edge draws its route; a self-loop draws
    // its curve, sampled; an edge to a dangling marker draws a straight run to the marker.
    let mut drawn: Vec<Vec<(f64, f64)>> = Vec::with_capacity(edges.len());
    for (pos, edge) in edges.iter().enumerate() {
        let (Some(src), Some(tgt)) = (resolve(&edge.source), resolve(&edge.target)) else {
            drawn.push(Vec::new());
            continue;
        };
        let line = match (&src, &tgt) {
            (Anchor::Node(sb), Anchor::Node(_)) if edge.source == edge.target => {
                self_loop_points(sb)
            }
            (Anchor::Node(_), Anchor::Node(_)) => route_for
                .get(&pos)
                .map(|&ri| routed[ri].points.clone())
                .unwrap_or_default(),
            _ => vec![
                anchor_point(&src, tgt.center()),
                anchor_point(&tgt, src.center()),
            ],
        };
        drawn.push(line);
    }

    // Place every edge label against those lines BEFORE drawing any of them. A label goes beside
    // its own line where there is room, is led to it from a clear pocket where there is not, and is
    // placed so that it lands on no box, on no other label, and across no other line - the
    // containment border is one of those lines, and it used to cut the names in half. The
    // placement is a pure function of this geometry, so the export stays byte-identical.
    //
    // EVERY drawn line is handed over, including the unlabelled ones: an edge without a name is
    // still a line on the drawing, and a label dropped across it is struck through just as badly.
    // An empty name is a line the placer keeps clear of and never places anything on.
    let mut label_lines: Vec<Vec<(f64, f64)>> = Vec::new();
    let mut label_text: Vec<&str> = Vec::new();
    let mut slot_for: HashMap<usize, usize> = HashMap::new();
    for (pos, edge) in edges.iter().enumerate() {
        if drawn[pos].len() < 2 {
            continue;
        }
        slot_for.insert(pos, label_lines.len());
        label_lines.push(drawn[pos].clone());
        label_text.push(if edge.label.is_empty() {
            ""
        } else {
            edge.label.as_str()
        });
    }
    let placed = routing::place_labels(
        &layout.nodes,
        &label_lines,
        &label_text,
        (canvas_w, drawing_h),
    );
    let mut label_for: HashMap<usize, routing::LabelPlacement> = HashMap::new();
    // The names the placement could not place. There should be none: the placer leads a label out
    // to clear space rather than give up on it. When there is one the page says so, because a
    // relationship nobody can name is a relationship nobody can check.
    let mut unplaced: Vec<String> = Vec::new();
    for (pos, edge) in edges.iter().enumerate() {
        if edge.label.is_empty() {
            continue;
        }
        match slot_for.get(&pos).and_then(|&slot| placed[slot].as_ref()) {
            Some(placement) => {
                label_for.insert(pos, placement.clone());
            }
            None => unplaced.push(edge.label.clone()),
        }
    }

    for (pos, edge) in edges.iter().enumerate() {
        let (Some(src), Some(tgt)) = (resolve(&edge.source), resolve(&edge.target)) else {
            continue;
        };
        let route = route_for.get(&pos).map(|&ri| &routed[ri]);
        push_edge(&mut svg, edge, &src, &tgt, route, label_for.get(&pos));
    }

    for node_box in &layout.nodes {
        let node = node_by_id
            .get(node_box.id.as_str())
            .expect("placed node exists in the graph");
        push_node(&mut svg, node, node_box);
    }

    for id in &dangling_ids {
        let (cx, cy) = dangling_centers[id.as_str()];
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
    (svg, unplaced)
}

fn edge_group(kind: &str) -> &'static str {
    match kind {
        "part" | "contains" => "containment",
        "dependency" => "dependency",
        "include" | "triggers" | "transition" => "flow",
        _ => "neutral",
    }
}

/// The ARROWHEAD class for an edge group. The stylesheet names the accent arrow after the token it
/// wears (--accent) rather than after the relationship, so a dependency edge needs the mapping:
/// without it the polygon matched no rule at all and SVG's default fill - BLACK - painted every
/// dependency arrowhead, on the page and in the export. Measured, not guessed: the rendered
/// export painted rgb(0, 0, 0) for the arrow whose line was accent blue.
fn arrow_group(group: &str) -> &'static str {
    match group {
        "dependency" => "accent",
        "containment" => "containment",
        "flow" => "flow",
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
    label: Option<&routing::LabelPlacement>,
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
    let (head_x, head_y, head_dx, head_dy) = match (src, tgt) {
        (Anchor::Node(sb), Anchor::Node(_)) if edge.source == edge.target => {
            let cx = sb.center_x();
            let cy = sb.center_y();
            let top = cy - 24.0;
            svg.push_str(&format!(
                "<path class='mw-edge {group}' d='M {:.1} {:.1} C {:.1} {:.1} {:.1} {:.1} {:.1} {:.1}' fill='none'/>",
                cx, top, cx - 30.0, top - 24.0, cx + 30.0, top - 24.0, cx, top
            ));
            (cx, top, 0.0, 1.0)
        }
        (Anchor::Node(_sb), Anchor::Node(_tb)) => {
            let r = route.expect("node-to-node edge is routed");
            push_polyline(svg, &r.points, &r.jumps, group, dash);
            let last = r.points[r.points.len() - 1];
            let prev = r.points[r.points.len() - 2];
            (last.0, last.1, last.0 - prev.0, last.1 - prev.1)
        }
        _ => {
            let (sx, sy) = anchor_point(src, tgt.center());
            let (tx, ty) = anchor_point(tgt, src.center());
            let (dx, dy) = (tx - sx, ty - sy);
            svg.push_str(&format!(
                "<line class='mw-edge {group}' x1='{:.1}' y1='{:.1}' x2='{:.1}' y2='{:.1}'{dash}/>",
                sx, sy, tx, ty
            ));
            (tx, ty, dx, dy)
        }
    };

    push_arrowhead(svg, head_x, head_y, head_dx, head_dy, arrow_group(group));
    if let Some(placement) = label {
        push_edge_label(svg, placement, &edge.label);
    }
    svg.push_str("</g>");
}

/// The point on a text baseline that puts the ink at the centre of the box the placement chose:
/// the browser sets 11 px text with an ascent of 12 and a descent of 3, so the baseline sits
/// (12 - 3) / 2 below the centre of the ink. Measured in the browser, not assumed.
const EDGE_LABEL_BASELINE: f64 = 4.5;

/// The ink an edge label's leader is drawn in: the same --text-2 the label rule uses, so a leader
/// reads as part of the label rather than as another edge. The class carries no stylesheet rule
/// (the stylesheet is not this module's to change), so the stroke is inline - which also means the
/// leader can never fall back to SVG's default of no stroke and vanish.
const EDGE_LEADER_INK: &str = "#49535c";

/// Draw a placed edge label: its text centred on the box the placement chose, and the leader that
/// leads to its line when the label had to leave the line to stay readable. A label that sits
/// beside its line gets no leader - the line is already next to it.
fn push_edge_label(svg: &mut String, placement: &routing::LabelPlacement, text: &str) {
    if placement.leader.len() >= 2 {
        let points: Vec<String> = placement
            .leader
            .iter()
            .map(|(x, y)| format!("{:.1},{:.1}", x, y))
            .collect();
        svg.push_str(&format!(
            "<polyline class='edge-leader' points='{}' fill='none' stroke='{EDGE_LEADER_INK}' stroke-width='1'/>",
            points.join(" ")
        ));
    }
    svg.push_str(&format!(
        "<text class='edge-label' x='{:.1}' y='{:.1}' text-anchor='middle'>{}</text>",
        placement.x,
        placement.y + EDGE_LABEL_BASELINE,
        xml_escape(text)
    ));
}

/// The curve the renderer draws for a self-loop, sampled into a polyline: the label placer needs
/// the line the reader sees, not the control points that produce it.
fn self_loop_points(b: &NodeBox) -> Vec<(f64, f64)> {
    let cx = b.center_x();
    let top = b.center_y() - 24.0;
    let (p0, p1, p2, p3) = (
        (cx, top),
        (cx - 30.0, top - 24.0),
        (cx + 30.0, top - 24.0),
        (cx, top),
    );
    (0..=8)
        .map(|i| {
            let t = i as f64 / 8.0;
            let u = 1.0 - t;
            let x = u * u * u * p0.0
                + 3.0 * u * u * t * p1.0
                + 3.0 * u * t * t * p2.0
                + t * t * t * p3.0;
            let y = u * u * u * p0.1
                + 3.0 * u * u * t * p1.1
                + 3.0 * u * t * t * p2.1
                + t * t * t * p3.1;
            (x, y)
        })
        .collect()
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
