// SPDX-License-Identifier: AGPL-3.0-or-later
//! V1 visual outputs: the full-screen presentation view of a diagram.
//!
//! The diagram at a readable scale with the workbench chrome gone - what goes on a projector.
//! The page is plain server-rendered HTML and embeds the SAME self-styled SVG the download route
//! serves, so the presentation view and the exported file can never disagree. It carries the
//! project/revision caption, and the view is in the URL, so a link is shareable.
//!
//! There is no script on the page: it works with JavaScript disabled. The SVG scales to the
//! viewport through CSS, and the browser's own zoom (not JavaScript) is how a presenter inspects
//! a large diagram in detail.

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use maud::{html, Markup, PreEscaped, DOCTYPE};

use crate::api::ApiState;
use crate::auth::{identity as resolve_identity, Identity};
use crate::error::ApiError;
use crate::ui::diagram::resolve_view;
use crate::ui::export::{inline_diagram, DiagramSource, RENDERER_NAME, RENDERER_VERSION};
use crate::ui::layout;
use crate::ui::model::{load_view, view_branch, LoadedView, ModelQuery};

/// The presentation page's own stylesheet: a full-viewport stage, the caption, and nothing else.
/// The diagram itself is styled by the stylesheet inlined in the SVG. No font is fetched.
const PRESENT_STYLE: &str = r#"
:root { color-scheme: light; }
html, body { margin: 0; height: 100%; }
body.present {
  display: flex; flex-direction: column; background: #ffffff; color: #1f2328;
  font-family: system-ui, -apple-system, "Segoe UI", Roboto, Helvetica, Arial, sans-serif;
}
.present-stage {
  flex: 1 1 auto; min-height: 0;
  display: flex; align-items: center; justify-content: center;
  overflow: auto; padding: 0.5rem;
}
.present-stage svg.mw-diagram-svg { width: 100%; height: 100%; display: block; }
.present-caption {
  flex: 0 0 auto; display: flex; align-items: baseline; gap: 0.5rem 1rem; flex-wrap: wrap;
  border-top: 1px solid #d1d9e0; background: #f6f8fa;
  padding: 0.5rem 0.9rem; font-size: 13px; color: #59636e;
}
.present-caption code { font-family: ui-monospace, "SFMono-Regular", "Cascadia Code", Consolas, "Liberation Mono", Menlo, monospace; color: #1f2328; }
.present-actions { margin-left: auto; display: inline-flex; gap: 1rem; }
.present-actions a { color: #1d4ed8; font-weight: 600; }
.present-actions a:hover { color: #2563eb; }
"#;

/// GET /ui/projects/:project/present?branch=&commit=&view= - the diagram, full screen, no chrome.
pub async fn present_page(
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
    match render_present(&state, &identity, &project, &query) {
        Ok(page) => layout::html_response(StatusCode::OK, page),
        Err(error) => layout::error_page(
            error.status,
            Some(&identity.subject),
            mechanism,
            &error.message,
        ),
    }
}

fn render_present(
    state: &ApiState,
    identity: &Identity,
    project: &str,
    query: &ModelQuery,
) -> Result<Markup, ApiError> {
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
    let diagram = inline_diagram(&source, graph).ok_or_else(|| {
        ApiError::not_found(format!("this model declares no {} diagram", view.as_str()))
    })?;

    // Every link carries the view and the commit, so the address is the whole state and the page
    // can be shared or bookmarked without JavaScript.
    let encoded = crate::ui::urlencode(project);
    let hash = crate::ui::urlencode(&commit.hash);
    let view_name = view.as_str();
    let download = format!("/ui/projects/{encoded}/diagram.svg?commit={hash}&view={view_name}");
    let workbench = format!("/ui/projects/{encoded}/diagram?commit={hash}&view={view_name}");
    let title = format!("modelwrite — {project} — present");

    Ok(html! {
        (DOCTYPE)
        html lang="en" {
            head {
                meta charset="utf-8";
                meta name="viewport" content="width=device-width, initial-scale=1";
                title { (title) }
                style { (PreEscaped(PRESENT_STYLE)) }
            }
            body class="present" {
                main class="present-stage" { (PreEscaped(diagram)) }
                footer class="present-caption" {
                    span class="present-id" {
                        (project) " · commit " code { (commit.hash) } " · " (view.label())
                        " · " (RENDERER_NAME) "/" (RENDERER_VERSION)
                    }
                    span class="present-actions" {
                        a class="download" href=(download) { "Download SVG" }
                        a class="workbench" href=(workbench) { "Back to the workbench diagram" }
                    }
                }
            }
        }
    })
}
