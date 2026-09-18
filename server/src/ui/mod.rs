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

pub mod layout;
pub mod pages;
