// SPDX-License-Identifier: AGPL-3.0-or-later
//! The browser workbench: a set of server-rendered HTML pages with no build step, no
//! bundler and no asset pipeline. Every page is plain semantic HTML a person can read in
//! view-source, with one small inline stylesheet.
//!
//! The pages are clients of the existing API surface, not a second implementation: they
//! read through the same [crate::store::Store] trait the JSON handlers use and apply the
//! same [crate::auth::Identity] resolution, [crate::auth::Permission::Read] check and
//! project-scope check, so the workbench can never become a weaker path to the data.
//!
//! The single templating dependency is maud, a compile-time HTML macro. Every dynamic
//! value - a project name, a commit message, a branch name, an author - is spliced through
//! maud, which HTML-escapes it by construction; the only way to emit markup is to wrap it
//! in [maud::PreEscaped], which is used solely for the developer-written stylesheet.

use axum::body::Body;
use axum::http::{header, StatusCode};
use axum::response::Response;

pub mod assist;
pub mod composition;
pub mod create;
pub mod diagram;
pub mod edit;
pub mod gate;
pub mod import;
pub mod layout;
pub mod model;
pub mod pages;
pub mod proposals;
pub mod review;
pub mod version;

/// Percent-encode a value for use in a URL PATH SEGMENT or QUERY STRING.
///
/// HTML escaping is a different job and does not cover this: a branch named `a&b` is
/// perfectly safe to put in a document but produces a query string that means something
/// else, so the link silently points at the wrong thing. Nothing here is a security fix -
/// maud already prevents markup injection - it is the difference between a link that works
/// and a link that lies.
pub fn urlencode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{:02X}", byte)),
        }
    }
    out
}

/// The workbench's single static asset: the vanilla-JavaScript enhancement that turns the
/// server-rendered model page into a three-pane IDE. There is deliberately no build step and
/// no asset pipeline - the file is embedded at compile time and served by this router, so an
/// air-gapped install gets it from the same binary as the page, and nothing is fetched at
/// page load.
pub async fn app_js() -> Response {
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/javascript; charset=utf-8")
        .header(header::CACHE_CONTROL, "no-cache")
        .body(Body::from(include_str!("app.js")))
        .expect("static response construction cannot fail")
}
