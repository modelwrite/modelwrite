// SPDX-License-Identifier: AGPL-3.0-or-later
//! V1 visual outputs: the standalone, downloadable SVG of a diagram.
//!
//! The export is generated FROM THE COMMIT, not from the screen. It resolves the requested
//! revision through the SAME [crate::ui::model::load_view] path every workbench page uses, then
//! draws it with the SAME deterministic renderer the diagram page uses. What the export adds is
//! what a file needs to stand on its own:
//!
//! * PROVENANCE - project, full commit id, view, SCOPE and renderer version, in an XML comment
//!   AND a metadata element, plus a visible caption band, so a picture in a report can always be
//!   traced to the revision it shows and can never be mistaken for the whole model.
//! * ONE PALETTE - the export owns no colours of its own. Its rules are written in the SAME
//!   custom properties the pages use and are resolved, at export time, from the literals declared
//!   in the ONE stylesheet (ui/layout.rs). An artefact that carried its own literals is exactly
//!   how the download came to wear the old brand after the pages were rebranded.
//! * SELF-CONTAINMENT - the resolved stylesheet is inlined here, and no font, image or stylesheet
//!   is fetched. The file opens correctly with no network.
//! * SCOPE - the whole model is complete and, at slide scale, unreadable. The export can draw
//!   one kind, one element with its neighbourhood, or one containment branch instead, and every
//!   artefact states on its face which scope it is and how much of the model that is.
//! * DETERMINISM - nothing time-dependent is written, so the same revision and the same SCOPE
//!   export byte-identically every time. (That is also why a wall-clock generated-at is
//!   deliberately absent: the commit id is the traceability anchor, and a timestamp would break
//!   byte-identity.)
//!
//! The HTTP handler reads through the same identity, permission and project-scope decisions as
//! every other page, so a download can never become a weaker path to the data.

use std::collections::BTreeMap;

use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::Response;
use serde::Deserialize;

use okf::types::Graph;

use crate::api::ApiState;
use crate::auth::{identity as resolve_identity, Identity};
use crate::error::ApiError;
use crate::store::Commit;
use crate::ui::diagram::{
    diagram_svg, drawing_canvas, legible_at_slide_scale, resolve_view, slide_label_px, xml_escape,
    DiagramScope, DiagramView, ScopeError, ScopedGraph, SvgOptions, DEFAULT_HOPS, MAX_HOPS,
};
use crate::ui::layout;
use crate::ui::model::{load_view, view_branch, LoadedView, ModelQuery};

/// The renderer's name, written into every export so an artefact says which code drew it.
pub const RENDERER_NAME: &str = "mw-diagram-svg";

/// The renderer's version: the crate version of the code that composes the SVG.
pub const RENDERER_VERSION: &str = env!("CARGO_PKG_VERSION");

/// The identity of a drawing: the revision it came from, which view of it, and HOW MUCH of it.
/// The scope is part of the artefact's identity, not a rendering detail: a filtered picture that
/// did not say so could be mistaken for the whole model.
pub(crate) struct DiagramSource<'a> {
    pub project: &'a str,
    pub commit: &'a Commit,
    /// The branch the view is scoped to (a commit may be reached by more than one branch).
    pub branch: &'a str,
    pub view: DiagramView,
    /// The drawing this artefact shows, with the counts that say what it is a subset of.
    pub scoped: &'a ScopedGraph,
}

// ---------------------------------------------------------------------------
// The palette: one source, resolved from the one stylesheet.
//
// A downloaded SVG carries its stylesheet inside itself, so the export has to write its rules
// out. What it must NOT do is write out a PALETTE: an artefact with its own colour literals is
// exactly how the export came to wear the old blue brand long after the pages were rebranded.
// The rules below are therefore written in the SAME custom properties the pages use, and
// [export_style] resolves them to literals read out of the one stylesheet in ui/layout.rs.
// Change a colour there and the export changes with it; the tests below fail loudly if a token
// the export needs stops being declared.
// ---------------------------------------------------------------------------

/// The ROOT token block of the ONE stylesheet the whole product wears (ui/layout.rs's STYLE).
/// Every colour, font and radius in the workbench pages is declared there, which makes that block
/// the only place in the product a palette is written down.
pub(crate) fn shared_root_block() -> &'static str {
    let style = layout::STYLE;
    let start = style
        .find(":root")
        .expect("the workbench stylesheet declares a :root token block");
    let open = style[start..]
        .find('{')
        .expect("the :root token block opens with a brace")
        + start;
    let close = style[open..]
        .find('}')
        .expect("the :root token block closes with a brace")
        + open;
    &style[start..=close]
}

/// Strip comment blocks. The token block is commented, and a comment that mentions a colour must
/// never be read as a declaration.
fn strip_comments(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    let mut rest = source;
    while let Some(start) = rest.find("/*") {
        out.push_str(&rest[..start]);
        match rest[start..].find("*/") {
            Some(end) => rest = &rest[start + end + 2..],
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

/// Every custom property declared in a token block, as name to literal value. Pure and total: a
/// declaration that cannot be read is skipped rather than guessed at.
fn parse_root_tokens(block: &str) -> BTreeMap<String, String> {
    let body = strip_comments(block);
    let mut tokens = BTreeMap::new();
    for segment in body.split(';') {
        // The block opens with the :root selector, and a segment can therefore carry that opener -
        // or a plain declaration such as color-scheme - before the custom property itself. A
        // property is found by its OWN name rather than by the position of the first colon: an
        // earlier version of this parser only worked because color-scheme happened to come first,
        // and would have silently dropped whatever property was written at the top of the block.
        let Some(at) = segment.find("--") else {
            continue;
        };
        let Some((name, value)) = segment[at..].split_once(':') else {
            continue;
        };
        let name = name.trim();
        let value = value.split_whitespace().collect::<Vec<_>>().join(" ");
        if !name.is_empty() && !value.is_empty() {
            tokens.insert(name.to_string(), value);
        }
    }
    tokens
}

/// The shared design tokens, read from the one stylesheet.
pub(crate) fn shared_tokens() -> BTreeMap<String, String> {
    parse_root_tokens(shared_root_block())
}

/// Resolve every var() reference against the shared tokens. The exported file therefore contains
/// no var() at all: it is a plain self-contained SVG that renders identically in any viewer, and
/// it still cannot carry a palette of its own. A token that is not declared is left exactly as
/// written, and the tests assert that none survive.
fn resolve_tokens(source: &str, tokens: &BTreeMap<String, String>) -> String {
    let mut out = String::with_capacity(source.len());
    let mut rest = source;
    while let Some(at) = rest.find("var(") {
        out.push_str(&rest[..at]);
        let after = &rest[at + 4..];
        let Some(close) = after.find(')') else {
            out.push_str("var(");
            rest = after;
            continue;
        };
        let name = after[..close].trim();
        match tokens.get(name) {
            Some(value) => out.push_str(value),
            None => {
                out.push_str("var(");
                out.push_str(&after[..close]);
                out.push(')');
            }
        }
        rest = &after[close + 1..];
    }
    out.push_str(rest);
    out
}

/// The export's own rules: the SVG subset a downloaded file needs. Every value is a SHARED design
/// token and there is deliberately not one colour literal here - see [export_style]. The rules
/// mirror the diagram section of the workbench stylesheet, with one deliberate difference: edge
/// labels are drawn, because a file that is read rather than hovered, and a picture on a slide,
/// cannot reveal them on demand.
const EXPORT_RULES: &str = r#"
svg { background: var(--surface); color-scheme: light; }
svg g.node { color: var(--text-3); }
svg g.node .node-rect { fill: var(--kind-neutral-bg); stroke: var(--text-3); stroke-width: 1.5px; rx: 2px; }
svg g.node .node-name { font-family: var(--font-ui); font-size: 13px; fill: var(--text); pointer-events: none; }
svg g.node .node-kind { font-family: var(--font-ui); font-size: 11px; fill: var(--text-2); text-transform: uppercase; letter-spacing: 0.04em; pointer-events: none; }
svg g.node .node-glyph { pointer-events: none; }
svg g.node .mw-2525-unknown { font-family: var(--font-ui); font-weight: 700; fill: var(--text); pointer-events: none; }
svg g.node[data-mw-kind="block"] { color: var(--kind-block); }
svg g.node[data-mw-kind="block"] .node-rect { fill: var(--kind-block-bg); stroke: var(--kind-block); }
svg g.node[data-mw-kind="block"] .node-kind { fill: var(--kind-block); }
svg g.node[data-mw-kind="actor"] { color: var(--kind-actor); }
svg g.node[data-mw-kind="actor"] .node-rect { fill: var(--kind-actor-bg); stroke: var(--kind-actor); }
svg g.node[data-mw-kind="actor"] .node-kind { fill: var(--kind-actor); }
svg g.node[data-mw-kind="usecase"] { color: var(--kind-usecase); }
svg g.node[data-mw-kind="usecase"] .node-rect { fill: var(--kind-usecase-bg); stroke: var(--kind-usecase); }
svg g.node[data-mw-kind="usecase"] .node-kind { fill: var(--kind-usecase); }
svg g.node[data-mw-kind="requirement"] { color: var(--kind-requirement); }
svg g.node[data-mw-kind="requirement"] .node-rect { fill: var(--kind-requirement-bg); stroke: var(--kind-requirement); }
svg g.node[data-mw-kind="requirement"] .node-kind { fill: var(--kind-requirement); }
svg g.node[data-mw-kind="signal"] { color: var(--kind-signal); }
svg g.node[data-mw-kind="signal"] .node-rect { fill: var(--kind-signal-bg); stroke: var(--kind-signal); }
svg g.node[data-mw-kind="signal"] .node-kind { fill: var(--kind-signal); }
svg g.node[data-mw-kind="interface"] { color: var(--kind-interface); }
svg g.node[data-mw-kind="interface"] .node-rect { fill: var(--kind-interface-bg); stroke: var(--kind-interface); }
svg g.node[data-mw-kind="interface"] .node-kind { fill: var(--kind-interface); }
svg g.node[data-mw-kind="activity"] { color: var(--kind-activity); }
svg g.node[data-mw-kind="activity"] .node-rect { fill: var(--kind-activity-bg); stroke: var(--kind-activity); }
svg g.node[data-mw-kind="activity"] .node-kind { fill: var(--kind-activity); }
svg g.node[data-mw-kind="state"] { color: var(--kind-state); }
svg g.node[data-mw-kind="state"] .node-rect { fill: var(--kind-state-bg); stroke: var(--kind-state); }
svg g.node[data-mw-kind="state"] .node-kind { fill: var(--kind-state); }
svg g.node[data-mw-kind="stateMachine"] { color: var(--kind-statemachine); }
svg g.node[data-mw-kind="stateMachine"] .node-rect { fill: var(--kind-statemachine-bg); stroke: var(--kind-statemachine); }
svg g.node[data-mw-kind="stateMachine"] .node-kind { fill: var(--kind-statemachine); }
svg .mw-edge { fill: none; stroke: var(--text-3); stroke-width: 1.3px; }
svg .mw-edge.containment { stroke: var(--text-2); }
svg .mw-edge.dependency { stroke: var(--accent); }
svg .mw-edge.flow { stroke: var(--kind-activity); }
svg .arrow-accent { fill: var(--accent); }
svg .arrow-containment { fill: var(--text-2); }
svg .arrow-flow { fill: var(--kind-activity); }
svg .arrow-neutral { fill: var(--text-3); }
svg g.edge .edge-label { display: block; font-family: var(--font-ui); font-size: 11px; fill: var(--text-2); pointer-events: none; }
svg g.dangling .dangling-shape { fill: var(--fail-bg); stroke: var(--fail); stroke-width: 1.5px; }
svg g.dangling .dangling-label { font-family: var(--font-ui); font-size: 11px; fill: var(--fail); font-weight: 600; pointer-events: none; }
svg g.dangling .dangling-id { font-family: var(--font-mono); font-size: 10px; fill: var(--fail); pointer-events: none; }
svg .mw-caption-band { fill: var(--surface-1); }
svg .mw-caption { font-family: var(--font-ui); font-size: 12px; fill: var(--text-2); }
"#;

/// The stylesheet an exported document carries: the export's rules with every shared token
/// resolved to the literal the one stylesheet declares.
pub(crate) fn export_style() -> String {
    resolve_tokens(EXPORT_RULES, &shared_tokens())
}

/// The canonical provenance line, the one a reader can quote or grep. It names the scope and the
/// counts, so a filtered picture carries its own evidence that it is not the whole model.
fn provenance_line(source: &DiagramSource<'_>) -> String {
    format!(
        "project={} commit={} branch={} diagram={} scope={} elements={}/{} relationships={}/{} renderer={}/{}",
        source.project,
        source.commit.hash,
        source.branch,
        source.view.as_str(),
        source.scoped.scope.as_str(),
        source.scoped.elements,
        source.scoped.total_elements,
        source.scoped.relationships,
        source.scoped.total_relationships,
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

/// The visible caption drawn beneath the drawing: project, full revision, view, SCOPE, and the
/// size the labels actually land at on a slide. A narrow drawing says so in words, and every
/// drawing states whether it is a slide picture at all, because a picture a reader cannot place -
/// or cannot read - is worse than no picture.
fn caption(source: &DiagramSource<'_>, canvas: Option<(f64, f64)>) -> String {
    let scoped = source.scoped;
    let scope = if scoped.is_full() {
        format!("the whole model ({})", scoped.counts())
    } else {
        format!(
            "{} - a subset of the model ({})",
            scoped.label(),
            scoped.counts()
        )
    };
    let readability = match canvas {
        Some((width, height)) => format!(
            " · labels {:.1} px on a 1920x1080 slide ({})",
            slide_label_px(width, height),
            if legible_at_slide_scale(width, height) {
                "readable"
            } else {
                "too small to read"
            }
        ),
        None => String::new(),
    };
    format!(
        "{} · commit {} · {} · scope: {}{readability} · {}/{}",
        source.project,
        source.commit.hash,
        source.view.label(),
        scope,
        RENDERER_NAME,
        RENDERER_VERSION
    )
}

/// The inline SVG for one revision: the page's own drawing, plus the metadata and inlined style
/// that make it stand alone, plus the caption band that states its revision and its scope.
pub(crate) fn inline_diagram(source: &DiagramSource<'_>, graph: &Graph) -> Option<String> {
    let scoped = source.scoped;
    let preface = format!(
        "<metadata><mw:provenance xmlns:mw=\"https://modelwrite.org/ns/provenance\" project=\"{}\" commit=\"{}\" branch=\"{}\" diagram=\"{}\" scope=\"{}\" elements=\"{}\" relationships=\"{}\" totalElements=\"{}\" totalRelationships=\"{}\" renderer=\"{}\" rendererVersion=\"{}\"/></metadata><style>{}</style>",
        xml_escape(source.project),
        xml_escape(&source.commit.hash),
        xml_escape(source.branch),
        source.view.as_str(),
        xml_escape(&scoped.scope.as_str()),
        scoped.elements,
        scoped.relationships,
        scoped.total_elements,
        scoped.total_relationships,
        RENDERER_NAME,
        RENDERER_VERSION,
        export_style()
    );
    // The caption states the readability of THIS drawing, measured on the canvas the renderer is
    // about to produce, so the artefact can never claim a legibility it does not have.
    let canvas = drawing_canvas(graph, source.view, true);
    let caption = caption(source, canvas);
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

/// The scope parameters of an export address, parsed apart from [ModelQuery] so the scoping option
/// lives with the export instead of widening the query every page shares.
#[derive(Deserialize, Default)]
pub struct ScopeQuery {
    /// full (the default), kinds, neighbourhood or containment.
    pub scope: Option<String>,
    /// The kinds of a kinds scope, comma separated.
    pub kinds: Option<String>,
    /// The element a neighbourhood or containment scope is centred on.
    pub element: Option<String>,
    /// The radius of a neighbourhood scope, in relationships.
    pub hops: Option<String>,
}

impl ScopeQuery {
    /// The scope this address asks for. An unrecognised scope, a missing kind list or a missing
    /// element is a bad request, and the message says what was asked for and what is valid.
    fn resolve(&self) -> Result<DiagramScope, String> {
        let mut kinds: Vec<String> = self
            .kinds
            .as_deref()
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|kind| !kind.is_empty())
            .map(str::to_string)
            .collect();
        // Sorted and deduplicated, so two addresses that mean the same scope - and must therefore
        // produce the same bytes - are one scope, not two.
        kinds.sort();
        kinds.dedup();
        let element = self
            .element
            .clone()
            .filter(|element| !element.trim().is_empty());
        match self.scope.as_deref() {
            None | Some("") | Some("full") => {
                if kinds.is_empty() && element.is_none() {
                    Ok(DiagramScope::Full)
                } else {
                    Err("scope=full draws the whole model and takes no kinds or element".to_string())
                }
            }
            Some("kinds") => {
                if kinds.is_empty() {
                    return Err(
                        "scope=kinds needs a kind list, e.g. &kinds=block,requirement".to_string()
                    );
                }
                Ok(DiagramScope::Kinds(kinds))
            }
            Some("neighbourhood") => {
                let element = element.ok_or_else(|| {
                    "scope=neighbourhood needs the element to centre on, e.g. &element=<id>"
                        .to_string()
                })?;
                let hops = match self.hops.as_deref() {
                    None | Some("") => DEFAULT_HOPS,
                    Some(raw) => raw.parse::<usize>().map_err(|_| {
                        format!("hops must be a whole number of relationships, got {raw}")
                    })?,
                };
                Ok(DiagramScope::Neighbourhood {
                    element,
                    hops: hops.min(MAX_HOPS),
                })
            }
            Some("containment") => {
                let element = element.ok_or_else(|| {
                    "scope=containment needs the container to draw, e.g. &element=<id>".to_string()
                })?;
                Ok(DiagramScope::Containment { element })
            }
            Some(other) => Err(format!(
                "unknown scope {other}: use scope=full, scope=kinds, scope=neighbourhood or scope=containment"
            )),
        }
    }
}

/// Resolve the revision, scope the model, and compose the document and its download filename.
fn render_export(
    state: &ApiState,
    identity: &Identity,
    project: &str,
    query: &ModelQuery,
    scope_query: &ScopeQuery,
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

    // The scope is resolved BEFORE the drawing and named IN it. A kind this model does not have is
    // a bad request; an element it does not have is a not-found, exactly like asking for a project
    // or a commit that does not exist.
    let scope = scope_query.resolve().map_err(ApiError::bad_request)?;
    let scoped = scope.apply(graph).map_err(|error| match error {
        ScopeError::UnknownElement(_) => ApiError::not_found(error.message()),
        ScopeError::NoSuchKind(_) => ApiError::bad_request(error.message()),
    })?;

    let source = DiagramSource {
        project,
        commit: &commit,
        branch: &branch,
        view,
        scoped: &scoped,
    };
    let document = standalone_document(&source, &scoped.graph).ok_or_else(|| {
        ApiError::not_found(format!("this model declares no {} diagram", view.as_str()))
    })?;
    let filename = format!(
        "modelwrite-{}-{}-{}-{}.svg",
        safe_segment(project),
        commit.hash,
        view.as_str(),
        safe_segment(&scope.as_str())
    );
    Ok((document, filename))
}

/// GET /ui/projects/:project/diagram.svg?branch=&commit=&view=&scope= - the diagram as a
/// standalone, downloadable SVG generated from the commit. With no scope it is the whole model;
/// with one it is a stated, counted subset that a slide can actually carry.
pub async fn diagram_svg_export(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(project): Path<String>,
    Query(query): Query<ModelQuery>,
    Query(scope): Query<ScopeQuery>,
) -> Response {
    let mechanism = state.auth.mechanism();
    let identity = match resolve_identity(&state, &headers).await {
        Ok(identity) => identity,
        Err(error) => return layout::sign_in_page(mechanism, &error.message),
    };
    match render_export(&state, &identity, &project, &query, &scope) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::diagram::NODE_LABEL_PX;

    /// The retired palette, from the export's side: the literals the export used to hardcode in a
    /// stylesheet of its own, which is why a downloaded artefact wore the old brand.
    const RETIRED: &[&str] = &[
        "#2563eb", "#1d4ed8", "#8b949e", "#59636e", "#1f2328", "#1f2937", "#8250df", "#0550ae",
        "#9a6700", "#1a7f37", "#0a7ea4", "#bc4c00", "#bf3989", "#cf222e", "#d1d9e0", "#f6f8fa",
        "#ffebe9", "#fff3cd",
    ];

    #[test]
    fn the_export_rules_carry_no_colour_literal_of_their_own() {
        // Not one hex value anywhere in the rules: every colour is a shared token, resolved from
        // the one stylesheet below. This is the assertion that would have caught the old blue
        // brand, and it fails the moment anyone pastes a literal back in.
        assert!(
            !EXPORT_RULES.contains('#'),
            "the export must not name a colour of its own:\n{EXPORT_RULES}"
        );
        assert!(
            EXPORT_RULES.contains("var(--accent)"),
            "the export paints with the shared tokens"
        );
    }

    #[test]
    fn every_token_the_export_uses_is_declared_by_the_one_stylesheet() {
        let tokens = shared_tokens();
        assert!(!tokens.is_empty(), "the one stylesheet declares tokens");
        let style = export_style();
        assert!(
            !style.contains("var("),
            "an unresolved token would render as nothing, got:\n{style}"
        );
        for retired in RETIRED {
            assert!(
                !style.contains(retired),
                "the export stylesheet still wears the retired colour {retired}"
            );
        }
        // The brand tokens are the instrument palette, and the export takes them by value from the
        // one stylesheet rather than restating them.
        assert_eq!(tokens.get("--accent").map(String::as_str), Some("#0b6e99"));
        assert_eq!(tokens.get("--chrome").map(String::as_str), Some("#101418"));
        for name in [
            "--accent",
            "--text",
            "--text-2",
            "--text-3",
            "--surface",
            "--surface-1",
            "--font-ui",
            "--font-mono",
            "--fail",
            "--fail-bg",
            "--kind-block",
            "--kind-block-bg",
            "--kind-requirement",
            "--kind-requirement-bg",
        ] {
            let value = tokens
                .get(name)
                .unwrap_or_else(|| panic!("{name} must be declared by the one stylesheet"));
            assert!(
                style.contains(value.as_str()),
                "the export must resolve {name} to {value}"
            );
        }
    }

    #[test]
    fn the_shared_token_block_is_the_stylesheets_own() {
        let block = shared_root_block();
        assert!(block.starts_with(":root"));
        assert!(block.ends_with('}'));
        assert!(block.contains("--accent"));
        assert!(
            layout::STYLE.contains(block),
            "the shared token block must be the stylesheet's own block, not a copy"
        );
    }

    #[test]
    fn a_token_comment_is_not_read_as_a_declaration() {
        // The token block is heavily commented, and its comments mention colours. A comment must
        // never become the palette.
        let tokens = parse_root_tokens(":root { /* --accent: #000000; */ --accent: #0b6e99; }");
        assert_eq!(tokens.get("--accent").map(String::as_str), Some("#0b6e99"));
        assert_eq!(
            tokens.len(),
            1,
            "a commented declaration is not a declaration"
        );
    }

    #[test]
    fn an_undeclared_token_is_left_exactly_as_written() {
        let tokens = parse_root_tokens(":root { --accent: #0b6e99; }");
        assert_eq!(
            resolve_tokens("a { color: var(--accent); }", &tokens),
            "a { color: #0b6e99; }"
        );
        assert!(
            resolve_tokens("a { color: var(--nope); }", &tokens).contains("var(--nope)"),
            "an undeclared token is left for the tests to find, never silently dropped"
        );
    }

    #[test]
    fn the_legibility_measure_is_the_type_the_one_stylesheet_sets() {
        // The scoping options are sized with NODE_LABEL_PX. If the stylesheet ever sets another
        // size, the page would state a number nobody can reproduce, so the two are pinned.
        let style = layout::STYLE;
        let at = style
            .find("svg g.node .node-name")
            .expect("the one stylesheet sizes the node name");
        let rule = &style[at..(at + 200).min(style.len())];
        assert!(
            rule.contains(&format!("font-size: {NODE_LABEL_PX:.0}px")),
            "the label size the export measures is not the size the stylesheet sets:\n{rule}"
        );
    }
}
