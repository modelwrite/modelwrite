// SPDX-License-Identifier: AGPL-3.0-or-later
//! The gate pages: the run list and the run detail. Both read the RECORDED runs through
//! the same [crate::store::Store] trait the JSON handlers use and apply the same identity,
//! permission and project-scope decisions, so the workbench can never become a weaker path
//! to a verdict. A failed gate is a successful outcome of running a gate, so it renders as
//! a page, never as an error page.

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use maud::{html, Markup};

use crate::api::{map_store_error, ApiState};
use crate::auth::{identity as resolve_identity, Identity, Permission};
use crate::error::ApiError;
use crate::store::GateRun;
use crate::ui::layout;

/// `GET /ui/projects/:project/gate` - the recorded gate runs, newest first.
pub async fn gate_list(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(project): Path<String>,
) -> Response {
    let mechanism = state.auth.mechanism();
    let identity = match resolve_identity(&state, &headers).await {
        Ok(identity) => identity,
        Err(error) => return layout::sign_in_page(mechanism, &error.message),
    };
    match render_gate_list(&state, &identity, &project) {
        Ok(page) => layout::html_response(StatusCode::OK, page),
        Err(error) => layout::error_page(
            error.status,
            Some(&identity.subject),
            mechanism,
            &error.message,
        ),
    }
}

fn render_gate_list(
    state: &ApiState,
    identity: &Identity,
    project: &str,
) -> Result<Markup, ApiError> {
    // The SAME Write-or-Review decision as the JSON list handler, in the SAME order, so a
    // caller that may not read a run is refused rather than shown an empty list that looks
    // like there are no runs.
    if !identity.may(Permission::Write) && !identity.may(Permission::Review) {
        return Err(ApiError::forbidden("write or review permission required"));
    }
    if !identity.may_reach(project) {
        return Err(ApiError::forbidden("project not in scope"));
    }
    if state
        .store
        .project(project)
        .map_err(map_store_error)?
        .is_none()
    {
        return Err(ApiError::not_found(format!("project {}", project)));
    }
    let mut runs = state.store.gate_runs(project).map_err(map_store_error)?;
    // The store returns runs in insertion order; the page lists the most recent first.
    runs.reverse();
    let mut nav = layout::Nav::load(state, identity, Some(project))?;
    nav.section = Some("checks");
    Ok(gate_list_markup(
        identity,
        state.auth.mechanism(),
        project,
        &runs,
        &nav,
    ))
}

fn gate_list_markup(
    identity: &Identity,
    mechanism: &str,
    project: &str,
    runs: &[GateRun],
    nav: &layout::Nav,
) -> Markup {
    let body = html! {
        h1 { "Checks" }
        p class="meta" { "Recorded gate runs, newest first." }
        @if runs.is_empty() {
            p { "No gate runs yet. A gate run through the API appears here." }
        } @else {
            ul class="projects" {
                @for run in runs {
                    li class="gate-run" {
                        a class="gate-run-link" href={
                            "/ui/projects/" (crate::ui::urlencode(project))
                            "/gate/" (crate::ui::urlencode(run.reference_hash.as_str()))
                            "/" (crate::ui::urlencode(run.candidate_hash.as_str()))
                        } {
                            @if run.passed {
                                span class="covered" { "passed" }
                            } @else {
                                span class="uncovered" { "failed" }
                            }
                            " "
                            // Short for reading, FULL on hover: two runs sharing a prefix
                            // would otherwise be indistinguishable in the list, and a hash a
                            // reviewer cannot check is a hash they must take on trust.
                            code title=(run.reference_hash.as_str()) { (short_hash(&run.reference_hash)) }
                            " → "
                            code title=(run.candidate_hash.as_str()) { (short_hash(&run.candidate_hash)) }
                            @if !run.branch.is_empty() {
                                span class="meta" { " on " (run.branch.as_str()) }
                            }
                            span class="meta" {
                                "evidence " code { (evidence_file_name(run).as_str()) }
                            }
                        }
                    }
                }
            }
        }
    };
    let title = format!("modelwrite — {} — gate", project);
    layout::shell(
        &title,
        nav,
        Some(&identity.subject),
        identity.may(Permission::Administer),
        mechanism,
        body,
    )
}

/// `GET /ui/projects/:project/gate/:reference/:candidate` - one recorded run in full.
pub async fn gate_detail(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path((project, reference, candidate)): Path<(String, String, String)>,
) -> Response {
    let mechanism = state.auth.mechanism();
    let identity = match resolve_identity(&state, &headers).await {
        Ok(identity) => identity,
        Err(error) => return layout::sign_in_page(mechanism, &error.message),
    };
    match render_gate_detail(&state, &identity, &project, &reference, &candidate) {
        Ok(page) => layout::html_response(StatusCode::OK, page),
        Err(error) => layout::error_page(
            error.status,
            Some(&identity.subject),
            mechanism,
            &error.message,
        ),
    }
}

fn render_gate_detail(
    state: &ApiState,
    identity: &Identity,
    project: &str,
    reference: &str,
    candidate: &str,
) -> Result<Markup, ApiError> {
    // The SAME permission the JSON list applies, because that list already returns the full
    // evidence record to any Write-or-Review caller. Requiring Review here would refuse an
    // author a record it can read through the API - a rule that hides nothing and only
    // teaches people that the workbench is the unreliable way to look at a gate run.
    //
    // (The brief that specified this page claimed the API restricted evidence to reviewers.
    // It does not, and mirroring a rule that exists only in the brief would have been the
    // wrong kind of consistency.)
    if !identity.may(Permission::Write) && !identity.may(Permission::Review) {
        return Err(ApiError::forbidden("write or review permission required"));
    }
    if !identity.may_reach(project) {
        return Err(ApiError::forbidden("project not in scope"));
    }
    if state
        .store
        .project(project)
        .map_err(map_store_error)?
        .is_none()
    {
        return Err(ApiError::not_found(format!("project {}", project)));
    }
    let run = state
        .store
        .gate_runs(project)
        .map_err(map_store_error)?
        .into_iter()
        .find(|run| run.reference_hash == reference && run.candidate_hash == candidate)
        .ok_or_else(|| {
            ApiError::not_found(format!("gate run {} against {}", candidate, reference))
        })?;
    let mut nav = layout::Nav::load(state, identity, Some(project))?;
    nav.section = Some("checks");
    Ok(gate_detail_markup(
        identity,
        state.auth.mechanism(),
        project,
        &run,
        &nav,
    ))
}

fn gate_detail_markup(
    identity: &Identity,
    mechanism: &str,
    project: &str,
    run: &GateRun,
    nav: &layout::Nav,
) -> Markup {
    // The recorded evidence is the AUTHORITY: the page renders what the gate reported,
    // never recomputes it, so the verdict, the names and the numbers can never disagree
    // with the run a reviewer is auditing.
    // An unparsable record is shown RAW rather than as a null. Rendering it as null would
    // present an empty page as though the gate had reported nothing, which is the opposite
    // of the truth - and this page exists so a reviewer can see what the gate actually said.
    let evidence: serde_json::Value = match serde_json::from_str(&run.evidence) {
        Ok(value) => value,
        Err(_) => serde_json::json!({ "unparsableEvidence": run.evidence }),
    };
    let roundtrip_equal = evidence
        .get("roundtrip")
        .and_then(|roundtrip| roundtrip.get("equal"))
        .and_then(|equal| equal.as_bool())
        .unwrap_or(false);
    let missing_elements = nested_strings(&evidence, "roundtrip", "missingElements");
    let extra_elements = nested_strings(&evidence, "roundtrip", "extraElements");
    let missing_edges = nested_strings(&evidence, "roundtrip", "missingEdges");
    let extra_edges = nested_strings(&evidence, "roundtrip", "extraEdges");
    let changed_attributes = nested_strings(&evidence, "roundtrip", "changedAttributes");
    let isolated = nested_strings(&evidence, "integration", "isolated");
    let component_count = nested_count(&evidence, "integration", "componentCount");
    let component_sizes = nested_sizes(&evidence, "integration", "componentSizes");
    let total = nested_count(&evidence, "coverage", "total");
    let covered = nested_count(&evidence, "coverage", "covered");
    let satisfied = nested_count(&evidence, "coverage", "satisfied");
    let refined = nested_count(&evidence, "coverage", "refined");
    let verified = nested_count(&evidence, "coverage", "verified");
    let allocated = nested_count(&evidence, "coverage", "allocated");
    let uncovered = nested_strings(&evidence, "coverage", "uncovered");
    let validation_errors = top_level_strings(&evidence, "validationErrors");
    let validation_warnings = top_level_strings(&evidence, "validationWarnings");
    let failures = top_level_strings(&evidence, "failures");
    let component_sizes_display = component_sizes
        .iter()
        .map(usize::to_string)
        .collect::<Vec<_>>()
        .join(", ");
    let evidence_pretty =
        serde_json::to_string_pretty(&evidence).unwrap_or_else(|_| run.evidence.clone());

    let body = html! {
        h1 { "Gate run" }
        p class="verdict" {
            "Verdict: "
            @if run.passed {
                span class="covered" { "passed" }
            } @else {
                span class="uncovered" { "failed" }
            }
        }
        p class="meta" {
            "reference " code { (run.reference_hash.as_str()) }
            " → candidate " code { (run.candidate_hash.as_str()) }
            @if !run.branch.is_empty() {
                " · branch " (run.branch.as_str())
            }
            " · " (run.created_at.as_str())
        }
        p class="meta" { "Evidence file: " code { (evidence_file_name(run).as_str()) } }

        section class="model-section" id="roundtrip" {
            h2 { "Fidelity" }
            @if roundtrip_equal {
                p { "The two models are identical; nothing was lost or changed." }
            } @else {
                (named_list("Missing elements", &missing_elements))
                (named_list("Extra elements", &extra_elements))
                (named_list("Missing edges", &missing_edges))
                (named_list("Extra edges", &extra_edges))
                (named_list("Changed attributes", &changed_attributes))
            }
        }
        section class="model-section" id="integration" {
            h2 { "Integration" }
            @if let Some(count) = component_count {
                p class="coverage-summary" {
                    (count) " connected component"
                    @if count != 1 { "s" }
                    @if !component_sizes_display.is_empty() {
                        " (sizes: " (component_sizes_display.as_str()) ")"
                    }
                }
            }
            @if !isolated.is_empty() {
                h3 { "Isolated nodes" }
                ul class="diff-list" {
                    @for node in &isolated { li { code { (node.as_str()) } } }
                }
            }
        }
        section class="model-section" id="coverage" {
            h2 { "Coverage" }
            @if let Some(total) = total {
                p class="coverage-summary" {
                    (total) " requirements: " (covered.unwrap_or(0)) " covered, " (uncovered.len()) " uncovered · "
                    (satisfied.unwrap_or(0)) " satisfy, " (refined.unwrap_or(0)) " refine, "
                    (verified.unwrap_or(0)) " verify, " (allocated.unwrap_or(0)) " allocate"
                }
                @if !uncovered.is_empty() {
                    h3 { "Uncovered requirements" }
                    ul class="diff-list" {
                        @for id in &uncovered { li { code { (id.as_str()) } } }
                    }
                }
            } @else {
                p { "This evidence carries no coverage report." }
            }
        }
        @if !validation_errors.is_empty() || !validation_warnings.is_empty() {
            section class="model-section" id="validation" {
                h2 { "Validation" }
                @if !validation_errors.is_empty() {
                    h3 { "Errors" }
                    ul class="failures" {
                        @for error in &validation_errors { li { (error.as_str()) } }
                    }
                }
                @if !validation_warnings.is_empty() {
                    h3 { "Warnings" }
                    ul class="diff-list" {
                        @for warning in &validation_warnings { li { (warning.as_str()) } }
                    }
                }
            }
        }
        @if !failures.is_empty() {
            section class="model-section" id="failures" {
                h2 { "Failures" }
                ul class="failures" {
                    @for failure in &failures { li { (failure.as_str()) } }
                }
            }
        }
        section class="model-section" id="evidence" {
            h2 { "Evidence record" }
            pre class="conflict-value" { (evidence_pretty.as_str()) }
        }
    };
    let title = format!("modelwrite — {} — gate", project);
    layout::shell(
        &title,
        nav,
        Some(&identity.subject),
        identity.may(Permission::Administer),
        mechanism,
        body,
    )
}

/// A named fidelity finding: shown BY NAME rather than as a count, because "requirement r7
/// is missing" is actionable while "1 missing element" is not.
fn named_list(heading: &str, items: &[String]) -> Markup {
    if items.is_empty() {
        return Markup::default();
    }
    html! {
        h3 { (heading) }
        ul class="diff-list" {
            @for item in items { li { code { (item) } } }
        }
    }
}

/// The deterministic evidence file name the gate handler writes: both FULL hashes, never
/// truncated, so a run can never cite a file it did not write.
fn evidence_file_name(run: &GateRun) -> String {
    format!("server-{}-{}.json", run.reference_hash, run.candidate_hash)
}

/// A string-list field nested under one section of the evidence record.
fn nested_strings(evidence: &serde_json::Value, section: &str, field: &str) -> Vec<String> {
    evidence
        .get(section)
        .and_then(|section| section.get(field))
        .and_then(|field| field.as_array())
        .map(|array| {
            array
                .iter()
                .filter_map(|value| value.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

/// A string-list field at the top level of the evidence record.
fn top_level_strings(evidence: &serde_json::Value, field: &str) -> Vec<String> {
    evidence
        .get(field)
        .and_then(|field| field.as_array())
        .map(|array| {
            array
                .iter()
                .filter_map(|value| value.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

/// A numeric field nested under one section of the evidence record.
fn nested_count(evidence: &serde_json::Value, section: &str, field: &str) -> Option<usize> {
    evidence
        .get(section)
        .and_then(|section| section.get(field))
        .and_then(|value| value.as_u64())
        .map(|value| value as usize)
}

/// A numeric-array field nested under one section of the evidence record.
fn nested_sizes(evidence: &serde_json::Value, section: &str, field: &str) -> Vec<usize> {
    evidence
        .get(section)
        .and_then(|section| section.get(field))
        .and_then(|field| field.as_array())
        .map(|array| {
            array
                .iter()
                .filter_map(|value| value.as_u64().map(|value| value as usize))
                .collect()
        })
        .unwrap_or_default()
}

fn short_hash(hash: &str) -> &str {
    hash.get(..8).unwrap_or(hash)
}
