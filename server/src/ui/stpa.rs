// SPDX-License-Identifier: AGPL-3.0-or-later
//! The STPA completeness screen: the ONE place the five checks land.
//!
//! The page holds the honest boundary up front: the analysis is AUTHORED, the check is
//! COMPUTED. Modelwrite does not perform STPA and will never claim to. Every finding below is
//! a pure function of the authored document (via [graph::stpa::stpa_report]), every one carries
//! its basis - what it was computed over and what the model did not carry - and the trend across
//! baselines is computed directly over the branch's commits, because there is no cached trend
//! path to reuse.
//!
//! The page reads through the SAME identity, Read-permission and project-scope decisions as the
//! JSON handlers, in the SAME order, so the workbench can never be a weaker path to the data.

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use maud::{html, Markup};

use graph::stpa::{stpa_report, StpaReport};
use okf::types::OkfRoot;

use crate::api::{load_model, ApiState};
use crate::auth::{identity as resolve_identity, Identity, Permission};
use crate::error::ApiError;
use crate::ui::layout;
use crate::ui::model::{load_view, view_branch, LoadedView, ModelQuery};

/// GET /ui/projects/:project/stpa?branch=&commit= - the STPA completeness screen.
pub async fn stpa_page(
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
    match render_stpa_page(&state, &identity, &project, &query) {
        Ok(page) => layout::html_response(StatusCode::OK, page),
        Err(error) => layout::error_page(
            error.status,
            Some(&identity.subject),
            mechanism,
            &error.message,
        ),
    }
}

fn render_stpa_page(
    state: &ApiState,
    identity: &Identity,
    project: &str,
    query: &ModelQuery,
) -> Result<Markup, ApiError> {
    let nav = layout::Nav::load(state, identity, Some(project))?;
    match load_view(state, identity, project, query)? {
        LoadedView::Empty => {
            let body = html! {
                h1 { "STPA completeness" }
                p class="empty-state" {
                    "This project has no model yet — there is no STPA analysis to check."
                }
            };
            Ok(layout::shell(
                &format!("modelwrite — {project} — STPA"),
                &nav,
                Some(&identity.subject),
                identity.may(Permission::Administer),
                state.auth.mechanism(),
                body,
            ))
        }
        LoadedView::Model { commit, root } => {
            let mut nav = nav;
            nav.section = Some("stpa");
            nav.branch = Some(view_branch(query, &commit));
            nav.commit = Some(commit.hash.clone());
            let branch = nav.branch.clone().unwrap_or_else(|| commit.branch.clone());
            let trend = trend_points(state, identity, project, &branch);
            Ok(stpa_markup(
                identity,
                state.auth.mechanism(),
                project,
                &branch,
                &commit.hash,
                &root,
                &trend,
                &nav,
            ))
        }
    }
}

/// One point on the trend line: the completeness counts at one commit of the branch.
struct TrendPoint {
    hash: String,
    message: String,
    unanalysed: usize,
    feedback_gaps: usize,
    hazards_without: usize,
    constraints_without: usize,
    ucas_without: usize,
    total: usize,
}

/// Check 5: the same counts across the branch's commits, computed directly (there is no cached
/// trend path in the analytics crate). Commits are read oldest-first, the store's own order.
fn trend_points(
    state: &ApiState,
    identity: &Identity,
    project: &str,
    branch: &str,
) -> Vec<TrendPoint> {
    let store = state.store_for(identity);
    let commits = match store.commits_on(project, branch) {
        Ok(commits) => commits,
        Err(_) => return Vec::new(),
    };
    let mut points = Vec::new();
    for commit in &commits {
        let Ok(root) = load_model(store.as_ref(), project, &commit.hash) else {
            continue;
        };
        let report = stpa_report(&root);
        points.push(TrendPoint {
            hash: commit.hash.clone(),
            message: commit.message.clone(),
            unanalysed: report.unanalysed_actions.len(),
            feedback_gaps: report.feedback_gaps.len(),
            hazards_without: report.hazards_without_constraints.len(),
            constraints_without: report.constraints_without_elements.len(),
            ucas_without: report.ucas_without_scenarios.len(),
            total: report.total_findings,
        });
    }
    points
}

/// Whether the model declares any STPA vocabulary at all.
fn has_stpa_content(report: &StpaReport) -> bool {
    report.controllers > 0
        || report.controlled_processes > 0
        || report.control_actions > 0
        || report.feedback > 0
        || report.hazards > 0
        || report.constraints > 0
        || report.ucas > 0
        || report.loss_scenarios > 0
}

/// The honest boundary, stated once and loudly: the analysis is authored, the check is computed.
fn honest_boundary() -> Markup {
    html! {
        p class="meta" {
            strong { "The analysis is AUTHORED; the check is COMPUTED." }
            " Modelwrite does not perform STPA, does not infer hazards from a design, and will "
            "never claim to. It holds your control structure and analysis as a versioned model, "
            "checks completeness and internal consistency, names everything missing, and reports "
            "each finding's basis."
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn stpa_markup(
    identity: &Identity,
    mechanism: &str,
    project: &str,
    branch: &str,
    commit_hash: &str,
    root: &OkfRoot,
    trend: &[TrendPoint],
    nav: &layout::Nav,
) -> Markup {
    let report = stpa_report(root);

    let body = html! {
        h1 { "STPA completeness" }
        p class="meta" {
            "branch " (branch) " · commit " code { (&commit_hash[..commit_hash.len().min(8)]) }
        }
        (honest_boundary())

        // The vocabulary the check was computed over, so a reader knows what the counts mean.
        section class="model-section" id="vocabulary" {
            h2 { "The control structure" }
            p class="meta" {
                (report.controllers) " controller" @if report.controllers != 1 { "s" }
                " · " (report.controlled_processes) " controlled process" @if report.controlled_processes != 1 { "es" }
                " · " (report.control_actions) " control action" @if report.control_actions != 1 { "s" }
                " · " (report.feedback) " feedback signal" @if report.feedback != 1 { "s" }
                " · " (report.hazards) " hazard" @if report.hazards != 1 { "s" }
                " · " (report.constraints) " constraint" @if report.constraints != 1 { "s" }
                " · " (report.ucas) " UCA" @if report.ucas != 1 { "s" }
                " · " (report.loss_scenarios) " loss scenario" @if report.loss_scenarios != 1 { "s" }
            }
            p class="meta" { (report.basis) }
        }

        @if !has_stpa_content(&report) {
            p class="empty-state" {
                "This model declares no STPA vocabulary, so there is nothing to check. See the "
                "domain pack at " code { "sample/stpa/stpa-vocabulary.json" } " for the convention."
            }
        } @else if report.total_findings == 0 {
            p class="check-passed" { "Complete: no missing UCA types, no open control loops, every hazard constrained, every constraint reaching the design, every UCA explained." }
        } @else {
            p class="check-failed" { (report.total_findings) " finding" @if report.total_findings != 1 { "s" } " found." }
        }

        (unanalysed_section(&report))
        (feedback_section(&report))
        (hazard_constraint_section(&report))
        (uca_scenario_section(&report))
        (trend_section(trend))
        p class="meta" {
            a href={ "/ui/projects/" (crate::ui::urlencode(project)) "/diagram?branch=" (crate::ui::urlencode(branch)) "&view=control" } { "See the control-structure view" }
            " · "
            a href={ "/ui/projects/" (crate::ui::urlencode(project)) "/health?branch=" (crate::ui::urlencode(branch)) } { "See model health" }
        }
    };
    let title = format!("modelwrite — {project} — STPA");
    layout::shell(
        &title,
        nav,
        Some(&identity.subject),
        identity.may(Permission::Administer),
        mechanism,
        body,
    )
}

/// Check 1: every ControlAction must carry a UCA for each of the four types.
fn unanalysed_section(report: &StpaReport) -> Markup {
    html! {
        section class="model-section" id="unanalysed" {
            h2 { "Unanalysed control actions (" (report.unanalysed_actions.len()) ")" }
            p class="meta" {
                "Every ControlAction must carry an UnsafeControlAction for each of the four types: "
                "not-provided, provided, wrong-timing-or-order, stopped-too-soon-or-applied-too-long."
            }
            @if report.unanalysed_actions.is_empty() {
                p { "None — every control action is analysed across the four types." }
            } @else {
                ul class="gaps" {
                    @for action in &report.unanalysed_actions {
                        li {
                            code { (action.action_id) } " — " (action.action_name)
                            " · missing " strong { (action.missing_types.join(", ")) }
                            " · carried " (action.covered_types.join(", "))
                            p class="meta" { (action.basis) }
                        }
                    }
                }
            }
        }
    }
}

/// Check 2: a Controller with a ControlAction and no feedback path.
fn feedback_section(report: &StpaReport) -> Markup {
    html! {
        section class="model-section" id="feedback" {
            h2 { "Control loops with no feedback (" (report.feedback_gaps.len()) ")" }
            p class="meta" {
                "A Controller with a ControlAction and no Feedback path is the classic STPA defect - "
                "structurally the same shape as the orphan/isolation the health view reports."
            }
            @if report.feedback_gaps.is_empty() {
                p { "None — every controller with a control action has a feedback path." }
            } @else {
                ul class="gaps" {
                    @for gap in &report.feedback_gaps {
                        li {
                            code { (gap.controller_id) } " — " (gap.controller_name)
                            " issues " (gap.control_actions.join(", "))
                            " but receives no feedback"
                            p class="meta" { (gap.basis) }
                        }
                    }
                }
            }
        }
    }
}

/// Check 3: hazards with no constraint, and constraints reaching no element.
fn hazard_constraint_section(report: &StpaReport) -> Markup {
    html! {
        section class="model-section" id="hazards-constraints" {
            h2 {
                "Hazards with no constraint (" (report.hazards_without_constraints.len()) ") "
                "· constraints reaching no element (" (report.constraints_without_elements.len()) ")"
            }
            p class="meta" {
                "Every hazard the analysis names must be mitigated by a constraint; every constraint "
                "must reach the design - otherwise it is an aspiration."
            }
            @if report.hazards_without_constraints.is_empty() {
                p { "None — every hazard is mitigated by a SystemConstraint." }
            } @else {
                ul class="gaps" {
                    @for hazard in &report.hazards_without_constraints {
                        li {
                            code { (hazard.hazard_id) } " — " (hazard.hazard_name)
                            p class="meta" { (hazard.basis) }
                        }
                    }
                }
            }
            @if report.constraints_without_elements.is_empty() {
                p { "None — every SystemConstraint reaches the design." }
            } @else {
                ul class="gaps" {
                    @for constraint in &report.constraints_without_elements {
                        li {
                            code { (constraint.constraint_id) } " — " (constraint.constraint_name)
                            p class="meta" { (constraint.basis) }
                        }
                    }
                }
            }
        }
    }
}

/// Check 4: UCAs with no LossScenario.
fn uca_scenario_section(report: &StpaReport) -> Markup {
    html! {
        section class="model-section" id="uca-scenarios" {
            h2 { "UCAs with no loss scenario (" (report.ucas_without_scenarios.len()) ")" }
            p class="meta" {
                "A claim that a UCA could happen, without a causal scenario, is an assertion — a "
                "UCA must be explained by a LossScenario."
            }
            @if report.ucas_without_scenarios.is_empty() {
                p { "None — every UCA is explained by a loss scenario." }
            } @else {
                ul class="gaps" {
                    @for uca in &report.ucas_without_scenarios {
                        li {
                            code { (uca.uca_id) } " — " (uca.uca_name)
                            p class="meta" { (uca.basis) }
                        }
                    }
                }
            }
        }
    }
}

/// Check 5: the trend across the branch's baselines.
fn trend_section(trend: &[TrendPoint]) -> Markup {
    html! {
        section class="model-section" id="trend" {
            h2 { "Trend across baselines (" (trend.len()) ")" }
            p class="meta" {
                "The same counts across the commits of the branch, oldest first. Computed directly "
                "over each commit's model - there is no cached trend path to reuse."
            }
            @if trend.is_empty() {
                p { "No commits to trend." }
            } @else {
                table class="traceability" {
                    thead {
                        tr {
                            th { "commit" }
                            th { "unanalysed" }
                            th { "no feedback" }
                            th { "hazards" }
                            th { "constraints" }
                            th { "UCAs" }
                            th { "total" }
                        }
                    }
                    tbody {
                        @for point in trend {
                            tr {
                                td {
                                    code { (&point.hash[..point.hash.len().min(8)]) }
                                    @if !point.message.is_empty() { " " (point.message) }
                                }
                                td { (point.unanalysed) }
                                td { (point.feedback_gaps) }
                                td { (point.hazards_without) }
                                td { (point.constraints_without) }
                                td { (point.ucas_without) }
                                td { strong { (point.total) } }
                            }
                        }
                    }
                }
            }
        }
    }
}
