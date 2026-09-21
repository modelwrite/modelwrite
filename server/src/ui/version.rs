// SPDX-License-Identifier: AGPL-3.0-or-later
//! The version-control pages: creating a new version and making a version current.
//!
//! A version IS a branch, so these pages are clients of the existing branch and merge
//! endpoints, never a second implementation. Creating a version posts to the SAME
//! branch-create path (through [crate::api::create_branch_core]); making a version current
//! runs the ENGINE's gate FIRST - the same [gate::run] the compare page uses - and then posts
//! to the SAME merge path the JSON handler and the merge form share. The audit entries and the
//! merge commit are therefore exactly what the API would have recorded.

use std::collections::HashSet;

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use maud::{html, Markup};
use serde::Deserialize;

use crate::api::{load_model, map_store_error, ApiState};
use crate::auth::{identity as resolve_identity, Identity, Permission};
use crate::error::ApiError;
use crate::ui::layout;

/// The query parameters of the make-current page: the branch being promoted.
#[derive(Deserialize)]
pub struct MakeCurrentQuery {
    pub candidate: Option<String>,
}

/// GET /ui/projects/:project/version/new - the new-version form. Offered only to a writer,
/// in the same permission order the branch-create JSON handler applies.
pub async fn new_version_page(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(project): Path<String>,
) -> Response {
    let mechanism = state.auth.mechanism();
    let identity = match resolve_identity(&state, &headers).await {
        Ok(identity) => identity,
        Err(error) => return layout::sign_in_page(mechanism, &error.message),
    };
    match render_new_version(&state, &identity, &project) {
        Ok(page) => layout::html_response(StatusCode::OK, page),
        Err(error) => layout::error_page(
            error.status,
            Some(&identity.subject),
            mechanism,
            &error.message,
        ),
    }
}

fn render_new_version(
    state: &ApiState,
    identity: &Identity,
    project: &str,
) -> Result<Markup, ApiError> {
    // The SAME identity, Write-permission and project-scope decisions as the branch-create
    // JSON handler, in the SAME order.
    if !identity.may(Permission::Write) {
        return Err(ApiError::forbidden("write permission required"));
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
    let mut nav = layout::Nav::load(state, identity, Some(project))?;
    nav.section = Some("changes");
    Ok(new_version_markup(
        identity,
        state.auth.mechanism(),
        project,
        &nav,
    ))
}

/// The new-version form. A version starts from a commit: the choices are every branch's tip
/// plus every recent commit, deduplicated by hash. The form posts to the EXISTING branch
/// endpoint, so the audit entry is the branch-create entry every caller writes.
fn new_version_markup(
    identity: &Identity,
    mechanism: &str,
    project: &str,
    nav: &layout::Nav,
) -> Markup {
    let mut choices: Vec<(String, String)> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for (name, tip) in &nav.branches {
        if seen.insert(tip.clone()) {
            choices.push((tip.clone(), format!("{name} @ {}", short_hash(tip))));
        }
    }
    for commit in &nav.recent_commits {
        if seen.insert(commit.hash.clone()) {
            let label = if commit.message.is_empty() {
                format!("commit {}", short_hash(&commit.hash))
            } else {
                format!("commit {} · {}", short_hash(&commit.hash), commit.message)
            };
            choices.push((commit.hash.clone(), label));
        }
    }
    let body = html! {
        h1 { "New version" }
        p class="meta" {
            "A version is a branch; its identity is its tip commit. Name the version and choose the commit it starts from."
        }
        @if choices.is_empty() {
            p { "This project has no commits yet, so there is nothing to branch from." }
        } @else {
            form method="post"
                 action={ "/ui/projects/" (crate::ui::urlencode(project)) "/branch" }
                 class="create-form" {
                p {
                    label for="version-name" { "Version name" }
                    input type="text" id="version-name" name="name" placeholder="e.g. v2-triangle-plates" required;
                }
                p {
                    label for="version-from" { "Branch from" }
                    select id="version-from" name="from" {
                        @for (hash, label) in &choices {
                            option value=(hash.as_str()) { (label.as_str()) }
                        }
                    }
                }
                button type="submit" { "Create version" }
            }
        }
    };
    let title = format!("modelwrite — {project} — new version");
    layout::shell(
        &title,
        nav,
        Some(&identity.subject),
        identity.may(Permission::Administer),
        mechanism,
        body,
    )
}

/// GET /ui/projects/:project/version/make-current?candidate= - the gate verdict for merging
/// candidate into the main line, shown BEFORE the merge form, so the engineer decides with
/// the evidence in front of them.
pub async fn make_current_page(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(project): Path<String>,
    Query(query): Query<MakeCurrentQuery>,
) -> Response {
    let mechanism = state.auth.mechanism();
    let identity = match resolve_identity(&state, &headers).await {
        Ok(identity) => identity,
        Err(error) => return layout::sign_in_page(mechanism, &error.message),
    };
    match render_make_current(&state, &identity, &project, &query) {
        Ok(page) => layout::html_response(StatusCode::OK, page),
        Err(error) => layout::error_page(
            error.status,
            Some(&identity.subject),
            mechanism,
            &error.message,
        ),
    }
}

fn render_make_current(
    state: &ApiState,
    identity: &Identity,
    project: &str,
    query: &MakeCurrentQuery,
) -> Result<Markup, ApiError> {
    // The SAME identity, Write-permission and project-scope decisions as the merge JSON
    // handler, in the SAME order: this page ends in a merge, so it is a write path.
    if !identity.may(Permission::Write) {
        return Err(ApiError::forbidden("write permission required"));
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
    let candidate = query
        .candidate
        .as_deref()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ApiError::bad_request("the candidate branch is required"))?;
    let main_tip = state
        .store_for(identity)
        .branch_tip(project, "main")
        .map_err(map_store_error)?
        .ok_or_else(|| ApiError::not_found("branch main has no commits"))?;
    let candidate_tip = state
        .store_for(identity)
        .branch_tip(project, candidate)
        .map_err(map_store_error)?
        .ok_or_else(|| ApiError::not_found(format!("branch {candidate} has no commits")))?;
    let main_model = load_model(state.store_for(identity).as_ref(), project, &main_tip)
        .map_err(map_store_error)?;
    let candidate_model = load_model(state.store_for(identity).as_ref(), project, &candidate_tip)
        .map_err(map_store_error)?;

    // The gate verdict is the ENGINE's, computed the same way the compare page computes it.
    // The page records nothing here; the merge itself records the audit entry and commit.
    let outcome = gate::run(&main_model, &candidate_model, false);
    let isolated = isolated_nodes(&outcome.evidence);

    let mut nav = layout::Nav::load(state, identity, Some(project))?;
    nav.section = Some("changes");
    nav.branch = Some(candidate.to_string());
    Ok(make_current_markup(
        identity,
        state.auth.mechanism(),
        project,
        candidate,
        &main_tip,
        &candidate_tip,
        &outcome,
        &isolated,
        &nav,
    ))
}

#[allow(clippy::too_many_arguments)]
fn make_current_markup(
    identity: &Identity,
    mechanism: &str,
    project: &str,
    candidate: &str,
    main_tip: &str,
    candidate_tip: &str,
    outcome: &gate::GateOutcome,
    isolated: &[String],
    nav: &layout::Nav,
) -> Markup {
    let body = html! {
        h1 { "Make current" }
        p class="meta" {
            "Merge " strong { (candidate) } " (" code { (short_hash(candidate_tip)) } ")"
            " into the main line (" code { (short_hash(main_tip)) } ")."
        }
        // The gate verdict comes FIRST: the engineer decides with the evidence in front of
        // them, not after the merge. A failed gate is information, not a refusal - the merge
        // form below is still offered, because the decision is the engineer's.
        section class="model-section" id="gate" {
            h2 { "Gate" }
            p {
                "Verdict: "
                @if outcome.passed {
                    span class="covered" { "passed" }
                } @else {
                    span class="uncovered" { "failed" }
                }
            }
            @if !outcome.failures.is_empty() {
                h3 { "Failures" }
                ul class="failures" {
                    @for failure in &outcome.failures { li { (failure.as_str()) } }
                }
            }
            @if !isolated.is_empty() {
                h3 { "Isolated nodes" }
                ul class="isolated" {
                    @for node in isolated { li { code { (node.as_str()) } } }
                }
            }
            p class="meta" {
                a href={ "/ui/projects/" (crate::ui::urlencode(project)) "/compare?from=main&to=" (crate::ui::urlencode(candidate)) } {
                    "See the full section-scoped diff"
                }
            }
        }
        section class="model-section" id="merge" {
            h2 { "Promote" }
            p { "The gate is shown above. Merge only if the evidence is acceptable." }
            (make_current_form(project, candidate))
        }
    };
    let title = format!("modelwrite — {project} — make current");
    layout::shell(
        &title,
        nav,
        Some(&identity.subject),
        identity.may(Permission::Administer),
        mechanism,
        body,
    )
}

/// The merge form, posting to the EXISTING merge endpoint. The target is the main line and
/// the source is the candidate, both fixed, so the action cannot be redirected to the wrong
/// pair; the message is pre-filled with the promotion's own wording.
fn make_current_form(project: &str, candidate: &str) -> Markup {
    let message = format!("Make {candidate} the current version");
    html! {
        form method="post"
             action={ "/ui/projects/" (crate::ui::urlencode(project)) "/merge" }
             class="merge-form" {
            p {
                label for="branch" { "Merge into branch (the main line)" }
                input type="text" id="branch" name="branch" value="main" readonly;
            }
            p {
                label for="other" { "from branch" }
                input type="text" id="other" name="other" value=(candidate) readonly;
            }
            p {
                label for="message" { "message" }
                input type="text" id="message" name="message" value=(message);
            }
            p {
                label for="holder" { "holder (optional)" }
                input type="text" id="holder" name="holder";
            }
            button type="submit" { "Merge and make current" }
        }
    }
}

/// The isolated graph nodes the gate reported, read straight out of the evidence's
/// integration.isolated field rather than recomputed.
fn isolated_nodes(evidence: &serde_json::Value) -> Vec<String> {
    evidence
        .get("integration")
        .and_then(|integration| integration.get("isolated"))
        .and_then(|isolated| isolated.as_array())
        .map(|nodes| {
            nodes
                .iter()
                .filter_map(|node| node.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

fn short_hash(hash: &str) -> &str {
    hash.get(..8).unwrap_or(hash)
}
