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
