// SPDX-License-Identifier: AGPL-3.0-or-later
//! V1 visual outputs: the standalone, downloadable SVG of a diagram.
//!
//! The export is generated FROM THE COMMIT, not from the screen. It resolves the requested
//! revision through the SAME [crate::ui::model::load_view] path every workbench page uses, then
//! draws it with the SAME deterministic renderer the diagram page uses. What the export adds is
//! what a file needs to stand on its own:
//!
//! * PROVENANCE - project, full commit id and renderer version, in an XML comment AND a metadata
//!   element, plus a visible caption band, so a picture in a report can always be traced to the
//!   revision it shows.
//! * SELF-CONTAINMENT - the stylesheet the page reads from the workbench is inlined here with
//!   literal values, and no font, image or stylesheet is fetched. The file opens correctly with
//!   no network.
//! * DETERMINISM - nothing time-dependent is written, so the same revision exports
//!   byte-identically every time. (That is also why a wall-clock generated-at is deliberately
//!   absent: the commit id is the traceability anchor, and a timestamp would break byte-identity.)
//!
//! The HTTP handler reads through the same identity, permission and project-scope decisions as
//! every other page, so a download can never become a weaker path to the data.

use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::Response;

use okf::types::Graph;

use crate::api::ApiState;
use crate::auth::{identity as resolve_identity, Identity};
use crate::error::ApiError;
use crate::store::Commit;
use crate::ui::diagram::{diagram_svg, resolve_view, xml_escape, DiagramView, SvgOptions};
use crate::ui::layout;
use crate::ui::model::{load_view, view_branch, LoadedView, ModelQuery};

/// The renderer's name, written into every export so an artefact says which code drew it.
pub const RENDERER_NAME: &str = "mw-diagram-svg";

/// The renderer's version: the crate version of the code that composes the SVG.
pub const RENDERER_VERSION: &str = env!("CARGO_PKG_VERSION");

/// The identity of a drawing: the revision it came from and which view of it.
pub(crate) struct DiagramSource<'a> {
    pub project: &'a str,
    pub commit: &'a Commit,
    /// The branch the view is scoped to (a commit may be reached by more than one branch).
    pub branch: &'a str,
    pub view: DiagramView,
}

/// The inlined stylesheet. The page reads these rules from the workbench stylesheet; a downloaded
/// SVG has no stylesheet of its own, so every token it needs is resolved to its literal value
/// here. The values mirror the :root tokens in ui/layout.rs. There is no @font-face, no @import
/// and no url(): nothing is fetched, so the file opens offline.
const EXPORT_STYLE: &str = r#"
svg { background: #ffffff; color-scheme: light; }
svg g.node { color: #8b949e; }
svg g.node .node-rect { stroke: #8b949e; stroke-width: 1.5px; }
svg g.node .node-name { font-family: system-ui, -apple-system, "Segoe UI", Roboto, Helvetica, Arial, sans-serif; font-size: 13px; fill: #1f2328; pointer-events: none; }
svg g.node .node-kind { font-family: system-ui, -apple-system, "Segoe UI", Roboto, Helvetica, Arial, sans-serif; font-size: 11px; fill: #59636e; text-transform: uppercase; letter-spacing: 0.04em; pointer-events: none; }
svg g.node .node-glyph { pointer-events: none; }
svg g.node .mw-2525-unknown { font-family: system-ui, -apple-system, "Segoe UI", Roboto, Helvetica, Arial, sans-serif; font-weight: 700; fill: #1f2937; pointer-events: none; }
svg g.node[data-mw-kind="block"] { color: #2563eb; }
svg g.node[data-mw-kind="block"] .node-rect { stroke: #2563eb; }
svg g.node[data-mw-kind="block"] .node-kind { fill: #2563eb; }
svg g.node[data-mw-kind="actor"] { color: #8250df; }
svg g.node[data-mw-kind="actor"] .node-rect { stroke: #8250df; }
svg g.node[data-mw-kind="actor"] .node-kind { fill: #8250df; }
svg g.node[data-mw-kind="usecase"] { color: #0550ae; }
svg g.node[data-mw-kind="usecase"] .node-rect { stroke: #0550ae; }
svg g.node[data-mw-kind="usecase"] .node-kind { fill: #0550ae; }
svg g.node[data-mw-kind="requirement"] { color: #9a6700; }
svg g.node[data-mw-kind="requirement"] .node-rect { stroke: #9a6700; }
svg g.node[data-mw-kind="requirement"] .node-kind { fill: #9a6700; }
svg g.node[data-mw-kind="signal"] { color: #1a7f37; }
svg g.node[data-mw-kind="signal"] .node-rect { stroke: #1a7f37; }
svg g.node[data-mw-kind="signal"] .node-kind { fill: #1a7f37; }
svg g.node[data-mw-kind="interface"] { color: #0a7ea4; }
svg g.node[data-mw-kind="interface"] .node-rect { stroke: #0a7ea4; }
svg g.node[data-mw-kind="interface"] .node-kind { fill: #0a7ea4; }
svg g.node[data-mw-kind="activity"] { color: #bc4c00; }
svg g.node[data-mw-kind="activity"] .node-rect { stroke: #bc4c00; }
svg g.node[data-mw-kind="activity"] .node-kind { fill: #bc4c00; }
svg g.node[data-mw-kind="state"] { color: #bf3989; }
svg g.node[data-mw-kind="state"] .node-rect { stroke: #bf3989; }
svg g.node[data-mw-kind="state"] .node-kind { fill: #bf3989; }
svg g.node[data-mw-kind="stateMachine"] { color: #cf222e; }
svg g.node[data-mw-kind="stateMachine"] .node-rect { stroke: #cf222e; }
svg g.node[data-mw-kind="stateMachine"] .node-kind { fill: #cf222e; }
svg .mw-edge { fill: none; stroke: #8b949e; stroke-width: 1.3px; }
svg .mw-edge.containment { stroke: #59636e; }
svg .mw-edge.dependency { stroke: #2563eb; }
svg .mw-edge.flow { stroke: #bc4c00; }
svg .arrow-accent { fill: #2563eb; }
svg .arrow-containment { fill: #59636e; }
svg .arrow-flow { fill: #bc4c00; }
svg .arrow-neutral { fill: #8b949e; }
svg g.edge .edge-label { display: block; font-family: system-ui, -apple-system, "Segoe UI", Roboto, Helvetica, Arial, sans-serif; font-size: 11px; fill: #59636e; pointer-events: none; }
svg g.dangling .dangling-shape { fill: #ffebe9; stroke: #cf222e; stroke-width: 1.5px; }
svg g.dangling .dangling-label { font-family: system-ui, -apple-system, "Segoe UI", Roboto, Helvetica, Arial, sans-serif; font-size: 11px; fill: #cf222e; font-weight: 600; pointer-events: none; }
svg g.dangling .dangling-id { font-family: ui-monospace, "SFMono-Regular", "Cascadia Code", Consolas, "Liberation Mono", Menlo, monospace; font-size: 10px; fill: #cf222e; pointer-events: none; }
svg .mw-caption-band { fill: #f6f8fa; }
svg .mw-caption { font-family: system-ui, -apple-system, "Segoe UI", Roboto, Helvetica, Arial, sans-serif; font-size: 12px; fill: #59636e; }
"#;

/// The canonical provenance line, the one a reader can quote or grep.
fn provenance_line(source: &DiagramSource<'_>) -> String {
    format!(
        "project={} commit={} branch={} diagram={} renderer={}/{}",
        source.project,
        source.commit.hash,
        source.branch,
        source.view.as_str(),
        RENDERER_NAME,
        RENDERER_VERSION
    )
}

/// An XML comment body cannot contain a double hyphen or a newline. The values are escaped so a
/// hostile project or branch name cannot make the exported file invalid XML.
fn comment_safe(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut previous = '\0';
    for c in value.chars() {
        let c = if c == '\n' || c == '\r' { ' ' } else { c };
        if c == '-' && previous == '-' {
            out.push(' ');
        }
        out.push(c);
        previous = c;
    }
    out
}

/// The visible caption drawn beneath the drawing: project, full revision, view and renderer.
fn caption(source: &DiagramSource<'_>) -> String {
    format!(
        "{} · commit {} · {} · {}/{}",
        source.project,
        source.commit.hash,
        source.view.label(),
        RENDERER_NAME,
        RENDERER_VERSION
    )
}

/// The inline SVG for one revision: the page's own drawing, plus the metadata and inlined style
/// that make it stand alone, plus the caption band that states its revision on its face.
pub(crate) fn inline_diagram(source: &DiagramSource<'_>, graph: &Graph) -> Option<String> {
    let preface = format!(
        "<metadata><mw:provenance xmlns:mw=\"https://modelwrite.org/ns/provenance\" project=\"{}\" commit=\"{}\" branch=\"{}\" diagram=\"{}\" renderer=\"{}\" rendererVersion=\"{}\"/></metadata><style>{}</style>",
        xml_escape(source.project),
        xml_escape(&source.commit.hash),
        xml_escape(source.branch),
        source.view.as_str(),
        RENDERER_NAME,
        RENDERER_VERSION,
        EXPORT_STYLE
    );
    let caption = caption(source);
    diagram_svg(
        graph,
        source.view,
        &SvgOptions {
            preface: &preface,
            caption: Some(&caption),
            sized: true,
        },
    )
}

/// A standalone XML document: the XML declaration and the provenance comment, then the inline
/// SVG. This is the byte sequence a caller downloads.
pub(crate) fn standalone_document(source: &DiagramSource<'_>, graph: &Graph) -> Option<String> {
    let inline = inline_diagram(source, graph)?;
    Some(format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!-- modelwrite-provenance: {} -->\n{}\n",
        comment_safe(&provenance_line(source)),
        inline
    ))
}

/// A filesystem-safe path segment (the project may contain characters a header cannot carry).
fn safe_segment(value: &str) -> String {
    value
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Resolve the revision and compose the document and its download filename.
fn render_export(
    state: &ApiState,
    identity: &Identity,
    project: &str,
    query: &ModelQuery,
) -> Result<(String, String), ApiError> {
    let LoadedView::Model { commit, root } = load_view(state, identity, project, query)? else {
        return Err(ApiError::not_found(format!(
            "project {project} has no committed model"
        )));
    };
    let graph = root
        .graph
        .as_ref()
        .ok_or_else(|| ApiError::not_found("this model has no graph section to draw"))?;
    let branch = view_branch(query, &commit);
    let view = resolve_view(graph, query.view.as_deref());
    let source = DiagramSource {
        project,
        commit: &commit,
        branch: &branch,
        view,
    };
    let document = standalone_document(&source, graph).ok_or_else(|| {
        ApiError::not_found(format!("this model declares no {} diagram", view.as_str()))
    })?;
    let filename = format!(
        "modelwrite-{}-{}-{}.svg",
        safe_segment(project),
        commit.hash,
        view.as_str()
    );
    Ok((document, filename))
}

/// GET /ui/projects/:project/diagram.svg?branch=&commit=&view= - the diagram as a standalone,
/// downloadable SVG generated from the commit.
pub async fn diagram_svg_export(
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
    match render_export(&state, &identity, &project, &query) {
        Ok((document, filename)) => Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "image/svg+xml; charset=utf-8")
            .header(
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{filename}\""),
            )
            .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
            .header(header::CACHE_CONTROL, "no-cache")
            .body(Body::from(document))
            .expect("static svg response construction cannot fail"),
        Err(error) => layout::error_page(
            error.status,
            Some(&identity.subject),
            mechanism,
            &error.message,
        ),
    }
}
