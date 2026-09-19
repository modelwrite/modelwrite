// SPDX-License-Identifier: AGPL-3.0-or-later
//! The shared page shell: the header, the navigation rail, the inline stylesheet, and the
//! sign-in and error renderings.
//!
//! There is deliberately no build step and no asset pipeline. The stylesheet is inlined in
//! the page, and every dynamic value is spliced through maud, which escapes it by
//! construction. A value that reaches a page from a model, a commit message, a branch name
//! or an error therefore cannot execute as markup: the only unescaped content is the
//! developer-written stylesheet, wrapped in `maud::PreEscaped` on purpose.

use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};
use maud::{html, Markup, PreEscaped, DOCTYPE};

const STYLE: &str = r#"
:root { color-scheme: light dark; }
* { box-sizing: border-box; }
body { margin: 0; font-family: system-ui, -apple-system, "Segoe UI", sans-serif; line-height: 1.5; }
.site-header { display: flex; align-items: baseline; gap: 1rem; padding: 0.75rem 1.25rem; border-bottom: 1px solid #ccc; }
.site-header .brand { font-weight: 700; color: #145ea8; text-decoration: none; }
.site-header .project { font-weight: 600; }
.site-header .who { margin-left: auto; color: #666; font-size: 0.9rem; }
.layout { display: flex; min-height: calc(100vh - 2.9rem); }
.rail { flex: 0 0 13rem; border-right: 1px solid #ccc; padding: 1rem; }
.rail ul { list-style: none; margin: 0; padding: 0; }
.rail li { margin-bottom: 0.25rem; }
.rail a { display: block; padding: 0.35rem 0.5rem; color: #145ea8; text-decoration: none; border-radius: 4px; }
.rail a:hover { background: #eef4fa; }
main { flex: 1 1 auto; padding: 1.5rem; max-width: 56rem; }
ul.projects, ul.branches { list-style: none; margin: 0; padding: 0; }
ul.projects li, ul.branches li { border: 1px solid #ccc; border-radius: 4px; padding: 0.75rem 1rem; margin-bottom: 0.5rem; background: #fff; }
a.project-name { font-weight: 600; color: #145ea8; text-decoration: none; }
.branch-name { font-weight: 600; }
code.tip { font-family: ui-monospace, monospace; font-size: 0.85rem; color: #666; margin-left: 0.5rem; }
.message { display: block; margin-top: 0.25rem; }
.meta { display: block; color: #666; font-size: 0.85rem; }
.sign-in, .error { max-width: 40rem; }
h1 { margin-top: 0; }
.model-section { margin-bottom: 2.25rem; }
.model-section h2 { border-bottom: 1px solid #ccc; padding-bottom: 0.25rem; }
.structure-tree, .structure-tree ul, ul.signals, ul.interfaces, ul.allocations, ul.states, ul.transitions, ul.activity-nodes, ul.unresolved-edges { list-style: none; margin: 0; padding: 0; }
.structure-tree ul { margin-left: 1.25rem; padding-left: 0.75rem; border-left: 1px solid #ddd; }
.element-name, .state-name, .activity-name { font-weight: 600; }
.element-kind, .node-type { color: #666; font-size: 0.85rem; margin-left: 0.4rem; }
.element-stereotypes { color: #8a6d1a; font-size: 0.8rem; margin-left: 0.4rem; }
.element-documentation { color: #444; margin: 0.15rem 0 0; font-size: 0.9rem; }
table.requirements, table.traceability { border-collapse: collapse; width: 100%; margin-top: 0.5rem; }
table.requirements th, table.requirements td, table.traceability th, table.traceability td { border: 1px solid #ccc; padding: 0.4rem 0.6rem; text-align: left; vertical-align: top; }
table.requirements th, table.traceability th { background: #f4f4f4; }
.req-id, .req-num { font-family: ui-monospace, monospace; font-size: 0.85rem; }
.uncovered { background: #f8d7da; color: #842029; font-weight: 600; padding: 0.05rem 0.4rem; border-radius: 4px; white-space: nowrap; }
.covered { background: #d1e7dd; color: #0f5132; font-weight: 600; padding: 0.05rem 0.4rem; border-radius: 4px; white-space: nowrap; }
.broken { color: #842029; font-weight: 600; }
.unresolved-edge { border-left: 3px solid #842029; padding-left: 0.5rem; margin-bottom: 0.25rem; }
.activity { border: 1px solid #ccc; border-radius: 4px; padding: 0.75rem 1rem; margin-bottom: 0.75rem; }
.activity-name { margin-top: 0; }
.diff-add { color: #0f5132; }
.diff-remove { color: #842029; }
.diff-change { color: #8a6d1a; }
.diff-list { list-style: none; margin: 0; padding: 0; }
.diff-list li { font-family: ui-monospace, monospace; font-size: 0.85rem; margin-bottom: 0.15rem; }
.conflict { border: 1px solid #ccc; border-radius: 4px; padding: 0.75rem 1rem; margin-bottom: 1rem; }
.conflict h2 { margin-top: 0; }
.conflict-columns { display: flex; gap: 1rem; }
.conflict-side { flex: 1 1 0; min-width: 0; }
.conflict-side h3 { margin-top: 0; }
.conflict-value { font-family: ui-monospace, monospace; font-size: 0.8rem; white-space: pre-wrap; overflow-wrap: anywhere; background: #f4f4f4; padding: 0.5rem; border-radius: 4px; }
.merge-form label, .compare-form label { display: block; font-weight: 600; margin-bottom: 0.15rem; }
.merge-form input, .compare-form input { font-family: ui-monospace, monospace; padding: 0.3rem 0.5rem; }
.element-edit { margin-left: 0.5rem; font-size: 0.8rem; color: #145ea8; text-decoration: none; }
.edit-form label { display: block; font-weight: 600; margin-bottom: 0.15rem; }
.edit-form input, .edit-form textarea { font-family: ui-monospace, monospace; padding: 0.3rem 0.5rem; width: 100%; max-width: 40rem; }
.edit-form textarea { min-height: 4rem; }
.edit-form fieldset { border: 1px solid #ccc; border-radius: 4px; margin: 0.75rem 0; padding: 0.5rem 0.75rem; }
.attribute-row { display: flex; gap: 0.5rem; margin-bottom: 0.25rem; }
.attribute-row input { flex: 1 1 0; min-width: 0; }
.form-errors, .lock-banner { border: 1px solid #ccc; border-radius: 4px; padding: 0.75rem 1rem; margin-bottom: 1rem; }
.form-errors { background: #f8d7da; color: #842029; }
.lock-banner { background: #fff3cd; color: #664d03; }
.import-form label { display: block; font-weight: 600; margin-bottom: 0.15rem; }
.import-form input, .import-form select, .import-form textarea { font-family: ui-monospace, monospace; padding: 0.3rem 0.5rem; }
.import-form textarea { min-height: 10rem; width: 100%; max-width: 40rem; }
.loss-list { list-style: none; margin: 0; padding: 0; }
.loss-list li { border-left: 3px solid #842029; padding-left: 0.5rem; margin-bottom: 0.35rem; }
.loss-subject { font-family: ui-monospace, monospace; font-weight: 600; }
.loss-note { color: #666; font-size: 0.85rem; }

.loss-accept { list-style: none; margin: 0; padding: 0; }
.loss-accept li { margin-bottom: 0.25rem; }
/* The JS-added IDE layout: containment tree | content | properties. Absent with JS off. */
main.mw-model-ide { max-width: none; }
.mw-workbench { display: flex; gap: 1.25rem; align-items: flex-start; }
.mw-tree-panel, .mw-props-panel { position: sticky; top: 1rem; max-height: calc(100vh - 3.5rem); overflow: auto; border: 1px solid #ccc; border-radius: 4px; background: #fff; }
.mw-tree-panel { flex: 0 0 16rem; padding: 0.5rem; }
.mw-props-panel { flex: 0 0 19rem; padding: 0.75rem 1rem; }
.mw-content { flex: 1 1 auto; min-width: 0; }
.mw-tree-panel .mw-search { width: 100%; padding: 0.35rem 0.5rem; font-family: ui-monospace, monospace; margin-bottom: 0.5rem; box-sizing: border-box; }
.mw-tree { list-style: none; margin: 0; padding: 0; }
.mw-tree ul { list-style: none; margin: 0 0 0 0.75rem; padding: 0 0 0 0.6rem; border-left: 1px solid #ddd; }
.mw-tree li { margin: 0.1rem 0; }
.mw-tree .mw-node { display: flex; align-items: center; gap: 0.3rem; padding: 0.15rem 0.3rem; border-radius: 4px; cursor: pointer; }
.mw-tree .mw-node:hover { background: #eef4fa; }
.mw-tree .mw-node.mw-selected { background: #145ea8; color: #fff; }
.mw-tree .mw-toggle { cursor: pointer; user-select: none; flex: 0 0 1rem; text-align: center; color: #666; }
.mw-tree .mw-group-label { font-weight: 600; color: #145ea8; padding: 0.2rem 0.3rem; cursor: pointer; }
.mw-tree .mw-kind-badge { color: #999; font-size: 0.75rem; margin-left: auto; }
.mw-node.mw-selected .mw-kind-badge { color: #dbe7f5; }
.mw-props h3 { margin: 0 0 0.5rem; }
.mw-props dl { margin: 0; }
.mw-props dt { font-weight: 600; margin-top: 0.6rem; font-size: 0.8rem; color: #666; }
.mw-props dd { margin: 0.1rem 0 0; overflow-wrap: anywhere; }
.mw-props .mw-empty { color: #999; font-style: italic; }
.mw-highlight { outline: 2px solid #145ea8; outline-offset: 2px; background: #f3f8fd; }
.mw-attr { font-family: ui-monospace, monospace; font-size: 0.85rem; }
.mw-badge-covered, .mw-badge-uncovered, .mw-badge-unknown { font-weight: 600; padding: 0.05rem 0.4rem; border-radius: 4px; white-space: nowrap; }
.mw-badge-covered { background: #d1e7dd; color: #0f5132; }
.mw-badge-uncovered { background: #f8d7da; color: #842029; }
.mw-badge-unknown { background: #eee; color: #555; }
g.mw-selected-node rect { stroke: #145ea8; stroke-width: 3px; }
"#;

pub fn html_response(status: StatusCode, markup: Markup) -> Response {
    (status, Html(markup.into_string())).into_response()
}

pub fn shell(
    title: &str,
    project: Option<&str>,
    subject: Option<&str>,
    mechanism: &str,
    body: Markup,
) -> Markup {
    html! {
        (DOCTYPE)
        html lang="en" {
            head {
                meta charset="utf-8";
                meta name="viewport" content="width=device-width, initial-scale=1";
                title { (title) }
                style { (PreEscaped(STYLE)) }
            }
            body {
                header class="site-header" {
                    a class="brand" href="/ui" { "modelwrite" }
                    @if let Some(project) = project {
                        span class="project" { (project) }
                    }
                    span class="who" {
                        @if let Some(subject) = subject {
                            (subject) " via " (mechanism)
                        } @else {
                            "not signed in"
                        }
                    }
                }
                div class="layout" {
                    nav class="rail" aria-label="workbench" {
                        ul {
                            li { a href="/ui" { "Projects" } }
                        }
                    }
                    main { (body) }
                }
            }
        }
    }
}

pub fn sign_in_page(mechanism: &str, message: &str) -> Response {
    let body = html! {
        section class="sign-in" {
            h1 { "Sign in required" }
            p { "This workbench is protected by " (mechanism) " authentication." }
            p { (message) }
            p { "Present your credentials with the request (for example an " code { "Authorization: Bearer" } " header) and reload." }
        }
    };
    html_response(
        StatusCode::UNAUTHORIZED,
        shell("sign in — modelwrite", None, None, mechanism, body),
    )
}

pub fn error_page(
    status: StatusCode,
    subject: Option<&str>,
    mechanism: &str,
    message: &str,
) -> Response {
    let body = html! {
        section class="error" {
            h1 { (status.as_u16()) " " (status.canonical_reason().unwrap_or("error")) }
            p { (message) }
        }
    };
    html_response(
        status,
        shell("error — modelwrite", None, subject, mechanism, body),
    )
}
