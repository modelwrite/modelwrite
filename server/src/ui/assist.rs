// SPDX-License-Identifier: AGPL-3.0-or-later
//! The assist panel: a natural-language request becomes a REVIEW ARTIFACT on screen, never a
//! commit. The panel posts to the SAME assist core the JSON handler uses
//! ([crate::assist::assist_core]) and accepts through the SAME acceptance core
//! ([crate::proposal_api::accept_proposal_core]), so the workbench is a client of the
//! existing API rather than a second sequence. Nothing here writes a model until a human
//! with Write clicks Accept; the panel itself only records a proposal and shows it.

use axum::extract::{Form, Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use maud::{html, Markup};
use serde::Deserialize;

use agent::{Confidence, Proposal, ProposalCheck, ProposedAction, ReviewArtifact};

use crate::api::{map_store_error, validate_name, verify_actor, ApiState};
use crate::assist::{
    assist_core, reasoner_status, AssistOutcome, LiveBackend, ReasonerStatus, LIVE_AGENT,
};
use crate::auth::{identity as resolve_identity, Identity, Permission};
use crate::error::ApiError;
use crate::proposal_api::accept_proposal_core;
use crate::store::{Commit, CommitProvenance};
use crate::ui::layout;

/// The panel rendered on the model page. It is offered only to a caller who may Write
/// (the caller decides that upstream): the request box and the Submit never appear for a
/// read-only viewer. The status names what a request WOULD do, so a deployment without a
/// reasoner is told plainly - and pointed at the scripted mode - rather than a bare error.
pub fn assist_panel_markup(project: &str, branch: &str, status: &ReasonerStatus) -> Markup {
    html! {
        section class="assist-panel" id="assist" {
            h2 { "Assist" }
            (reasoner_status_markup(status))
            @if matches!(status, ReasonerStatus::Live(_) | ReasonerStatus::Scripted) {
                (assist_form_markup(project, branch))
            }
        }
    }
}

/// The one-line status: which live backend (the local fleet or Anthropic), scripted, or
/// (plainly) not configured with a pointer at the deterministic scripted mode. The local
/// fleet is named first because it wins when both are configured.
fn reasoner_status_markup(status: &ReasonerStatus) -> Markup {
    match status {
        ReasonerStatus::Live(LiveBackend::OpenAi) => html! {
            p class="meta" { "Live reasoner configured: local OpenAI-compatible model (" (LIVE_AGENT) ")." }
        },
        ReasonerStatus::Live(LiveBackend::Anthropic) => html! {
            p class="meta" { "Live reasoner configured: Anthropic (" (LIVE_AGENT) ")." }
        },
        ReasonerStatus::Scripted => html! {
            p class="meta" { "Running the scripted reasoner (deterministic test mode)." }
        },
        ReasonerStatus::NotConfigured => html! {
            p class="meta" {
                "No live reasoner configured: set " code { "MW_ASSIST_BASE_URL" }
                " (with " code { "MW_ASSIST_MODEL" } ") or " code { "MW_ANTHROPIC_API_KEY" }
                ", or " code { "MW_ASSIST_REASONER=scripted" } " for the deterministic test mode."
            }
        },
        ReasonerStatus::UnknownMode(mode) => html! {
            p class="meta" {
                "The assist reasoner mode " code { (mode) }
                " is not recognised; expected " code { "scripted" } "."
            }
        },
    }
}

/// The request form. The branch is CARRIED, not chosen: the request is about the model the
/// caller is looking at, so it targets that branch rather than quietly running on main.
fn assist_form_markup(project: &str, branch: &str) -> Markup {
    html! {
        form method="post"
             action={ "/ui/projects/" (crate::ui::urlencode(project)) "/assist" }
             class="assist-form" {
            input type="hidden" name="branch" value=(branch);
            p {
                label for="assist-request" { "request" }
                textarea id="assist-request" name="request"
                         placeholder="add a heater block with a water inlet port and a requirement that it heats to 95C" { }
            }
            button type="submit" { "Submit" }
        }
    }
}

/// `GET /ui/projects/:project/assist` - the assist panel as its own section page. It targets
/// the default branch (the one the model page resolves to) and offers the panel to a writer; a
/// viewer sees the status but never the request box.
pub async fn assist_page(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(project): Path<String>,
) -> Response {
    let mechanism = state.auth.mechanism();
    let identity = match resolve_identity(&state, &headers).await {
        Ok(identity) => identity,
        Err(error) => return layout::sign_in_page(mechanism, &error.message),
    };
    match render_assist_page(&state, &identity, &project) {
        Ok(page) => layout::html_response(StatusCode::OK, page),
        Err(error) => layout::error_page(
            error.status,
            Some(&identity.subject),
            mechanism,
            &error.message,
        ),
    }
}

fn render_assist_page(
    state: &ApiState,
    identity: &Identity,
    project: &str,
) -> Result<Markup, ApiError> {
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
    let mut nav = layout::Nav::load(state, identity, Some(project))?;
    nav.section = Some("assist");
    nav.branch = Some("main".to_string());
    let has_model = state
        .store_for(identity)
        .branch_tip(project, "main")
        .map_err(map_store_error)?
        .is_some();
    Ok(assist_page_markup(
        identity,
        state.auth.mechanism(),
        project,
        has_model,
        &nav,
    ))
}

fn assist_page_markup(
    identity: &Identity,
    mechanism: &str,
    project: &str,
    has_model: bool,
    nav: &layout::Nav,
) -> Markup {
    let can_write = identity.may(Permission::Write);
    let body = html! {
        h1 { "Assist" }
        p { "Describe a change in words; the reasoner drafts a proposal for you to review before accepting." }
        @if !has_model {
            p class="empty-state" { "This project has no model yet — start one before asking for changes." }
        } @else if can_write {
            (assist_panel_markup(project, "main", &reasoner_status()))
        } @else {
            p { "Ask a writer to propose changes." }
        }
    };
    let title = format!("modelwrite — {} — assist", project);
    layout::shell(
        &title,
        nav,
        Some(&identity.subject),
        identity.may(Permission::Administer),
        mechanism,
        body,
    )
}

#[derive(Deserialize)]
pub struct AssistForm {
    pub request: String,
    pub branch: String,
}

/// POST /ui/projects/:project/assist - run the reasoner and render its review artifact.
/// The SAME Write and scope checks as the JSON handler, through the SAME core, so the page
/// and the endpoint cannot disagree about what a request did. The result records a proposal
/// and shows it; nothing is committed.
pub async fn assist_form(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(project): Path<String>,
    Form(form): Form<AssistForm>,
) -> Response {
    let mechanism = state.auth.mechanism();
    let identity = match resolve_identity(&state, &headers).await {
        Ok(identity) => identity,
        Err(error) => return layout::sign_in_page(mechanism, &error.message),
    };
    // Permissions first, in the same order as the JSON handler.
    if !identity.may(Permission::Write) {
        return layout::error_page(
            StatusCode::FORBIDDEN,
            Some(&identity.subject),
            mechanism,
            "write permission required",
        );
    }
    if !identity.may_reach(&project) {
        return layout::error_page(
            StatusCode::FORBIDDEN,
            Some(&identity.subject),
            mechanism,
            "project not in scope",
        );
    }
    let nav = match layout::Nav::load(&state, &identity, Some(&project)) {
        Ok(mut nav) => {
            nav.section = Some("assist");
            nav.branch = Some(form.branch.clone());
            nav
        }
        Err(error) => {
            return layout::error_page(
                error.status,
                Some(&identity.subject),
                mechanism,
                &error.message,
            );
        }
    };
    match assist_core(
        state.store_for(&identity).as_ref(),
        &project,
        &form.branch,
        &form.request,
        &identity.subject,
        mechanism,
        state.auth.authorizer().unwrap_or(""),
    )
    .await
    {
        Ok(outcome) => layout::html_response(
            StatusCode::OK,
            assist_result_page(&identity, mechanism, &project, &form.branch, &outcome, &nav),
        ),
        Err(error) => layout::error_page(
            error.status,
            Some(&identity.subject),
            mechanism,
            &error.message,
        ),
    }
}

/// The review-artifact page: who proposed it, the request, the proposed changes (each with
/// its action and rationale, low-confidence marked) and - for a writer - the Accept form.
fn assist_result_page(
    identity: &Identity,
    mechanism: &str,
    project: &str,
    branch: &str,
    outcome: &AssistOutcome,
    nav: &layout::Nav,
) -> Markup {
    let title = format!("modelwrite — {} — assist review", project);
    let body = html! {
        h1 { "Assist review" }
        p class="meta" {
            "proposed by " strong { (outcome.artifact.agent) }
            " · proposal " code { (short_hash(&outcome.id)) }
        }
        (review_artifact_markup(&outcome.artifact))
        @if identity.may(Permission::Write) {
            (accept_form_markup(project, &outcome.id, branch))
        }
        p {
            a href={ "/ui/projects/" (crate::ui::urlencode(project)) "/model?branch=" (crate::ui::urlencode(branch)) } {
                "Back to the model"
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

/// The review artifact, rendered as the human reads it everywhere: the request, the proposed
/// changes, the gate check of the candidate (so an isolated node or a coverage regression is
/// visible BEFORE acceptance), and any gaps the reasoner named. The candidate document is the
/// SAME document a later acceptance applies, but it is not re-rendered here - the changes are
/// what a human decides on.
pub fn review_artifact_markup(artifact: &ReviewArtifact) -> Markup {
    html! {
        p class="meta" { "Request: " (artifact.task.goal) }
        @if let Some(check) = &artifact.check {
            (check_markup(check))
        }
        section class="review-artifact" {
            h2 { "Proposed changes" }
            (proposal_list_markup(&artifact.proposals))
            @if !artifact.gaps.is_empty() {
                h3 { "Gaps" }
                ul class="gaps" {
                    @for gap in &artifact.gaps { li { (gap) } }
                }
            }
        }
    }
}

/// The validator-and-gate result over the candidate, surfaced to the human BEFORE they accept:
/// an isolated node, a disconnected component, an uncovered requirement or a validation error
/// is named here rather than left to a later gate run to discover.
fn check_markup(check: &ProposalCheck) -> Markup {
    let new_uncovered = check
        .uncovered_requirements
        .iter()
        .filter(|id| !check.prior_uncovered_requirements.contains(id))
        .count();
    html! {
        section class="proposal-check" {
            h2 { "Gate check of the candidate" }
            @if check.passed {
                p class="check-passed" {
                    "Passes: no validation errors, no isolated nodes, one connected component."
                }
            } @else {
                p class="check-failed" { "Would fail the gate." }
            }
            @if !check.validation_errors.is_empty() {
                h3 { "Validation errors" }
                ul class="check-errors" {
                    @for error in &check.validation_errors { li { (error) } }
                }
            }
            @if !check.isolated_nodes.is_empty() {
                h3 { "Isolated nodes" }
                p { "Each is invisible to coverage and traceability." }
                ul class="check-isolated" {
                    @for id in &check.isolated_nodes { li { code { (id) } } }
                }
            }
            @if check.component_count != 1 {
                p { "Connected components: " (check.component_count) }
            }
            p {
                "Uncovered requirements: " (check.uncovered_requirements.len())
                @if new_uncovered > 0 { " (" (new_uncovered) " new since the current model)" }
            }
        }
    }
}

/// One proposed change: its action, the subject it names, its rationale, and its confidence.
/// A low-confidence proposal is marked for attention, never buried among the sure ones.
pub fn proposal_list_markup(proposals: &[Proposal]) -> Markup {
    html! {
        @if proposals.is_empty() {
            p { "The reasoner proposed no changes." }
        } @else {
            ul class="proposal-list" {
                @for proposal in proposals {
                    li class="proposal" {
                        span class="proposal-action" { (action_label(&proposal.action)) }
                        @if !proposal.subject.is_empty() {
                            code class="proposal-subject" { (proposal.subject) }
                        }
                        @if !proposal.rationale.is_empty() {
                            p class="proposal-rationale" { (proposal.rationale) }
                        }
                        @if proposal.needs_attention() {
                            span class="low-confidence" { "low confidence" }
                        } @else {
                            span class="confidence" { (confidence_label(&proposal.confidence)) }
                        }
                    }
                }
            }
        }
    }
}

fn action_label(action: &ProposedAction) -> &'static str {
    match action {
        ProposedAction::EditElement => "EditElement",
        ProposedAction::DraftText => "DraftText",
        ProposedAction::AcceptLoss => "AcceptLoss",
        ProposedAction::Reject => "Reject",
        ProposedAction::NoAction => "NoAction",
    }
}

fn confidence_label(confidence: &Confidence) -> &'static str {
    match confidence {
        Confidence::High => "high confidence",
        Confidence::Medium => "medium confidence",
        Confidence::Low => "low confidence",
    }
}

/// The Accept form, offered only to a writer (the caller already checked). Accepting commits
/// the proposed candidate through the SAME acceptance core as the JSON route.
fn accept_form_markup(project: &str, id: &str, branch: &str) -> Markup {
    html! {
        section class="accept-proposal" {
            h2 { "Accept" }
            p { "Accepting commits the proposed changes and records you as the acceptor." }
            form method="post"
                 action={ "/ui/projects/" (crate::ui::urlencode(project)) "/proposals/" (crate::ui::urlencode(id)) "/accept" }
                 class="accept-form" {
                input type="hidden" name="branch" value=(branch);
                p {
                    label for="message" { "commit message" }
                    input type="text" id="message" name="message" required;
                }
                p {
                    label for="holder" { "holder (optional)" }
                    input type="text" id="holder" name="holder";
                }
                button type="submit" { "Accept" }
            }
        }
    }
}

#[derive(Deserialize)]
pub struct AcceptForm {
    pub branch: String,
    pub message: String,
    #[serde(default)]
    pub holder: String,
}

/// POST /ui/projects/:project/proposals/:id/accept - a human with Write accepts a proposal
/// by id, through the SAME acceptance core as the JSON route. The page then shows the commit
/// and the provenance naming both the proposing agent and the accepting human.
pub async fn accept_proposal_form(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path((project, id)): Path<(String, String)>,
    Form(form): Form<AcceptForm>,
) -> Response {
    let mechanism = state.auth.mechanism();
    let identity = match resolve_identity(&state, &headers).await {
        Ok(identity) => identity,
        Err(error) => return layout::sign_in_page(mechanism, &error.message),
    };
    if !identity.may(Permission::Write) {
        return layout::error_page(
            StatusCode::FORBIDDEN,
            Some(&identity.subject),
            mechanism,
            "write permission required",
        );
    }
    if !identity.may_reach(&project) {
        return layout::error_page(
            StatusCode::FORBIDDEN,
            Some(&identity.subject),
            mechanism,
            "project not in scope",
        );
    }
    let holder = if form.holder.is_empty() {
        None
    } else {
        Some(form.holder.as_str())
    };
    if let Err(error) = verify_actor(&state.auth, &identity, holder) {
        return layout::error_page(
            error.status,
            Some(&identity.subject),
            mechanism,
            &error.message,
        );
    }
    if let Err(error) = validate_name("branch name", &form.branch) {
        return layout::error_page(
            error.status,
            Some(&identity.subject),
            mechanism,
            &error.message,
        );
    }
    let nav = match layout::Nav::load(&state, &identity, Some(&project)) {
        Ok(mut nav) => {
            nav.section = Some("assist");
            nav.branch = Some(form.branch.clone());
            nav
        }
        Err(error) => {
            return layout::error_page(
                error.status,
                Some(&identity.subject),
                mechanism,
                &error.message,
            );
        }
    };
    // The author is the verified identity, never a field the browser supplies.
    let author = identity.subject.clone();
    match accept_proposal_core(
        state.store_for(&identity).as_ref(),
        &project,
        &id,
        &identity.subject,
        mechanism,
        state.auth.authorizer().unwrap_or(""),
        &author,
        &form.branch,
        &form.message,
        holder,
        &[],
    ) {
        Ok(commit) => layout::html_response(
            StatusCode::CREATED,
            accept_result_page(&identity, mechanism, &project, &commit, &nav),
        ),
        Err(error) => layout::error_page(
            error.status,
            Some(&identity.subject),
            mechanism,
            &error.message,
        ),
    }
}

fn accept_result_page(
    identity: &Identity,
    mechanism: &str,
    project: &str,
    commit: &Commit,
    nav: &layout::Nav,
) -> Markup {
    let parties = acceptance_parties(commit);
    let title = format!("modelwrite — {} — proposal accepted", project);
    let body = html! {
        h1 { "Proposal accepted" }
        p { "Commit " code { (short_hash(&commit.hash)) } " on " strong { (commit.branch) } }
        p { "author " strong { (commit.author) } }
        @if !commit.message.is_empty() { p { (commit.message) } }
        @if let Some((agent, accepted_by, accepted_items)) = parties {
            section class="provenance" {
                h2 { "Provenance" }
                p { "Proposed by " strong { (agent) } }
                p { "Accepted by " strong { (accepted_by) } }
                @if !accepted_items.is_empty() {
                    p { "Accepted items:" }
                    ul class="accepted-items" {
                        @for item in accepted_items { li { code { (item) } } }
                    }
                }
            }
        }
        p {
            a href={ "/ui/projects/" (crate::ui::urlencode(project)) "/model?commit=" (crate::ui::urlencode(commit.hash.as_str())) } {
                "View the accepted model"
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

/// The two parties an acceptance commit names, read from the commit's own provenance. A
/// commit that is not an acceptance (the store cannot produce one here, but a reader a year
/// later must not assume) yields None.
fn acceptance_parties(commit: &Commit) -> Option<(&str, &str, &[String])> {
    match &commit.provenance {
        CommitProvenance::Accepted {
            agent,
            accepted_by,
            accepted_items,
            ..
        } => Some((
            agent.as_str(),
            accepted_by.as_str(),
            accepted_items.as_slice(),
        )),
        _ => None,
    }
}

fn short_hash(hash: &str) -> &str {
    hash.get(..8).unwrap_or(hash)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_panel_says_so_when_no_reasoner_is_configured() {
        let markup =
            assist_panel_markup("coffee", "main", &ReasonerStatus::NotConfigured).into_string();
        assert!(markup.contains("No live reasoner configured"), "{}", markup);
        assert!(markup.contains("MW_ASSIST_REASONER=scripted"));
        assert!(
            !markup.contains(r#"name="request""#),
            "no form must be offered when no reasoner is configured"
        );
    }

    #[test]
    fn the_panel_offers_a_form_in_scripted_mode() {
        let markup = assist_panel_markup("coffee", "main", &ReasonerStatus::Scripted).into_string();
        assert!(markup.contains("scripted"), "{}", markup);
        assert!(markup.contains(r#"name="request""#));
        assert!(markup.contains("Submit"));
    }

    #[test]
    fn low_confidence_proposals_are_marked() {
        let proposals = vec![Proposal {
            subject: "heater".to_string(),
            action: ProposedAction::EditElement,
            rationale: "because".to_string(),
            confidence: Confidence::Low,
        }];
        let markup = proposal_list_markup(&proposals).into_string();
        assert!(markup.contains("low confidence"), "{}", markup);
        assert!(markup.contains("low-confidence"));
    }

    #[test]
    fn high_confidence_proposals_are_not_marked() {
        let proposals = vec![Proposal {
            subject: "heater".to_string(),
            action: ProposedAction::EditElement,
            rationale: "because".to_string(),
            confidence: Confidence::High,
        }];
        let markup = proposal_list_markup(&proposals).into_string();
        assert!(!markup.contains("low confidence"));
        assert!(markup.contains("high confidence"), "{}", markup);
    }
}
