// SPDX-License-Identifier: AGPL-3.0-or-later
//! The proposals page: every recorded proposal for a project, newest first, each showing the
//! proposing agent, the request, the decision (open / accepted by whom / refused by whom) and
//! the accepted items. A read-only view, guarded by the SAME Read and scope checks as the
//! JSON list endpoint.

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use maud::{html, Markup};

use crate::api::{map_store_error, ApiState};
use crate::auth::{identity as resolve_identity, Identity, Permission};
use crate::error::ApiError;
use crate::store::{ProposalDecision, ProposalRecord};
use crate::ui::layout;

/// GET /ui/projects/:project/proposals - the project's proposals, newest first, with the
/// decision attached (open, or accepted/refused by the human who made it).
pub async fn proposals_page(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(project): Path<String>,
) -> Response {
    let mechanism = state.auth.mechanism();
    let identity = match resolve_identity(&state, &headers).await {
        Ok(identity) => identity,
        Err(error) => return layout::sign_in_page(mechanism, &error.message),
    };
    match render_proposals_page(&state, &identity, &project) {
        Ok(page) => layout::html_response(StatusCode::OK, page),
        Err(error) => layout::error_page(
            error.status,
            Some(&identity.subject),
            mechanism,
            &error.message,
        ),
    }
}

fn render_proposals_page(
    state: &ApiState,
    identity: &Identity,
    project: &str,
) -> Result<Markup, ApiError> {
    // The SAME identity, Read-permission and project-scope decisions as the JSON handler.
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
        return Err(ApiError::not_found(format!("project {}", project)));
    }
    let records = state
        .store_for(identity)
        .list_proposals(project)
        .map_err(map_store_error)?;
    let mut nav = layout::Nav::load(state, identity, Some(project))?;
    nav.section = Some("proposals");
    Ok(proposals_page_markup(
        identity,
        state.auth.mechanism(),
        project,
        &records,
        &nav,
    ))
}

fn proposals_page_markup(
    identity: &Identity,
    mechanism: &str,
    project: &str,
    records: &[ProposalRecord],
    nav: &layout::Nav,
) -> Markup {
    let title = format!("modelwrite — {} — proposals", project);
    let body = html! {
        h1 { "Proposals" }
        @if records.is_empty() {
            p { "No proposals yet." }
        } @else {
            ul class="proposals" {
                @for record in records {
                    li class="proposal" {
                        h2 { (record.agent) }
                        p { (record.task_goal) }
                        p class="decision" { (decision_text(record)) }
                        @if !record.accepted_items.is_empty() {
                            p class="meta" { "Accepted items:" }
                            ul class="accepted-items" {
                                @for item in &record.accepted_items { li { code { (item) } } }
                            }
                        }
                        p class="meta" { "proposal " code { (short_hash(&record.id)) } }
                    }
                }
            }
        }
    };
    layout::shell(
        &title,
        nav,
        Some(&identity.subject),
        identity.may(Permission::Administer),
        mechanism,
        body,
    )
}

/// The decision a human made, or "open" while the proposal is undecided.
fn decision_text(record: &ProposalRecord) -> String {
    match record.decision {
        None => "open".to_string(),
        Some(ProposalDecision::Accepted) => format!("accepted by {}", record.decided_by),
        Some(ProposalDecision::Refused) => format!("refused by {}", record.decided_by),
    }
}

fn short_hash(hash: &str) -> &str {
    hash.get(..8).unwrap_or(hash)
}
