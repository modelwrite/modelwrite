// SPDX-License-Identifier: AGPL-3.0-or-later
//! The analyses pages: the LIBRARY (what has been run over this project), one RUN, and the
//! DIFF between two runs.
//!
//! The pages are clients of the frame in [crate::analyses] and of the store's own analysis-run
//! reads, never a second implementation: the library renders the RECORDED runs, the run page
//! renders the findings the run stored, and the diff compares two stored runs by finding
//! identity. A page that recomputed an analysis on view would be exactly the side effect the
//! design rules out, so nothing here measures anything - running is an explicit POST that
//! stores a record and lands the caller on it.
//!
//! Running an analysis READS the model: the commit is resolved, the document is loaded, the
//! engine measures it, and the result is stored. No write path to a model is reachable from
//! here, which is what "an analytics run never writes to a model" means in code.

use std::collections::HashMap;

use axum::extract::{Form, Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use maud::{html, Markup};
use serde::Deserialize;

use crate::analyses::{self, AnalysisRun, Finding, FindingsDiff, Severity};
use crate::api::{load_model, map_store_error, ApiState};
use crate::auth::{identity as resolve_identity, Identity, Permission};
use crate::error::ApiError;
use crate::store::{now_epoch, now_seconds, AuditEntry};
use crate::ui::layout;

/// The query of the run page: the optional other run to diff this one against.
#[derive(Deserialize)]
pub struct RunQuery {
    pub compare: Option<String>,
}

/// The query of the diff page: the two runs to compare.
#[derive(Deserialize)]
pub struct DiffQuery {
    pub from: Option<String>,
    pub to: Option<String>,
}

// ---------------------------------------------------------------------------
// The library.

/// GET /ui/projects/:project/analyses - the library: every analysis run recorded for this
/// project, newest first, each addressable and any two diffable.
pub async fn analyses_page(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(project): Path<String>,
) -> Response {
    let mechanism = state.auth.mechanism();
    let identity = match resolve_identity(&state, &headers).await {
        Ok(identity) => identity,
        Err(error) => return layout::sign_in_page(mechanism, &error.message),
    };
    match render_library(&state, &identity, &project, None) {
        Ok(page) => layout::html_response(StatusCode::OK, page),
        Err(error) => layout::error_page(
            error.status,
            Some(&identity.subject),
            mechanism,
            &error.message,
        ),
    }
}

fn render_library(
    state: &ApiState,
    identity: &Identity,
    project: &str,
    notice: Option<&str>,
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
    let mut runs = state
        .store_for(identity)
        .analysis_runs(project)
        .map_err(map_store_error)?;
    // The store returns runs in insertion order; the library reads newest first.
    runs.reverse();
    let branches = state
        .store_for(identity)
        .list_branches(project)
        .map_err(map_store_error)?;
    let can_write = identity.may(Permission::Write);
    let mut nav = layout::Nav::load(state, identity, Some(project))?;
    nav.section = Some("analyses");
    Ok(library_markup(
        identity,
        state.auth.mechanism(),
        project,
        &runs,
        &branches,
        can_write,
        notice,
        &nav,
    ))
}

#[allow(clippy::too_many_arguments)]
fn library_markup(
    identity: &Identity,
    mechanism: &str,
    project: &str,
    runs: &[AnalysisRun],
    branches: &[(String, String)],
    can_write: bool,
    notice: Option<&str>,
    nav: &layout::Nav,
) -> Markup {
    // The two runs the compare form offers by default: the OLDEST run (the library is newest
    // first) and the NEWEST run of the SAME definition, so pressing "Diff the two runs"
    // compares two readings of one analysis rather than a coverage run against a health run.
    let default_from = runs.len().saturating_sub(1);
    let default_to = runs
        .iter()
        .position(|run| {
            run.definition_id() == runs[default_from].definition_id()
                && run.id != runs[default_from].id
        })
        .unwrap_or(0);
    let body = html! {
        h1 { "Analyses" }
        p class="meta" {
            "An analysis is a record, not a page that recomputes: a named definition run over \
             one commit, stored with the engine version it was computed at. Running one reads \
             the model and never changes it."
        }
        @if let Some(notice) = notice {
            p class="form-errors" { (notice) }
        }

        section class="model-section" id="run" {
            h2 { "Run an analysis" }
            @if !can_write {
                p class="meta" { "Running an analysis records a run, so it needs write permission on this project." }
            } @else if branches.is_empty() {
                p class="empty-state" { "This project has no commits yet, so there is nothing to analyse." }
            } @else {
                form method="post" action={ "/ui/projects/" (crate::ui::urlencode(project)) "/analyses" } class="create-form" {
                    p {
                        label for="analysis-definition" { "Analysis" }
                        select id="analysis-definition" name="definition" {
                            @for definition in analyses::built_ins() {
                                option value=(definition.id.as_str()) {
                                    (definition.name.as_str()) " (v" (definition.version) ")"
                                }
                            }
                        }
                    }
                    p {
                        label for="analysis-branch" { "Version (branch)" }
                        select id="analysis-branch" name="branch" {
                            @for (name, tip) in branches {
                                option value=(name.as_str()) { (name.as_str()) " @ " (short_hash(tip)) }
                            }
                        }
                    }
                    button type="submit" { "Run and store the record" }
                }
            }
        }

        section class="model-section" id="library" {
            h2 { "Library (" (runs.len()) ")" }
            @if runs.is_empty() {
                p class="empty-state" { "No analysis has been run over this project yet. A run appears here the moment it is stored, and stays." }
            } @else {
                ul class="projects" {
                    @for run in runs {
                        li {
                            a class="analysis-run" href={
                                "/ui/projects/" (crate::ui::urlencode(project))
                                "/analyses/" (crate::ui::urlencode(run.id.as_str()))
                            } {
                                span class="project-name" { (run.definition.name.as_str()) }
                                span class="project-meta" {
                                    " v" (run.definition.version)
                                    " · commit " code title=(run.commit_hash.as_str()) { (short_hash(&run.commit_hash)) }
                                    " · " (if run.branch.is_empty() { "no branch" } else { run.branch.as_str() })
                                    " · engine " code { (run.engine_version.as_str()) }
                                    " · " code { (run.created_at.as_str()) }
                                }
                                p class="analysis-tallies" {
                                    @if run.measured {
                                        (severity_chip(Severity::Gap, run.count(Severity::Gap)))
                                        " "
                                        (severity_chip(Severity::Unknown, run.count(Severity::Unknown)))
                                        " "
                                        (severity_chip(Severity::Ok, run.count(Severity::Ok)))
                                    } @else {
                                        // A run the engine could not measure publishes NO gap
                                        // count: a zero beside "not measured" would be read as
                                        // "no gaps", which is the one thing it does not say.
                                        (severity_chip(Severity::Unknown, run.count(Severity::Unknown)))
                                        " "
                                        span class="mw-badge-unknown" { "not measured" }
                                    }
                                }
                                p class="project-meta" {
                                    "run " code title=(run.id.as_str()) { (short_hash(&run.id)) }
                                    " · evidence " code title=(run.evidence_hash.as_str()) { (short_hash(&run.evidence_hash)) }
                                    @if !run.run_by.is_empty() { " · by " (run.run_by.as_str()) }
                                }
                            }
                        }
                    }
                }
            }
        }

        @if runs.len() >= 2 {
            section class="model-section" id="compare" {
                h2 { "Compare two runs" }
                p class="meta" { "Two runs are compared by finding identity, so what appeared, what was resolved and what was measured differently is read rather than inferred." }
                form method="get"
                     action={ "/ui/projects/" (crate::ui::urlencode(project)) "/analyses/diff" }
                     class="create-form" {
                    p {
                        label for="diff-from" { "From (the earlier run)" }
                        select id="diff-from" name="from" {
                            // The library is newest first, so the OLDEST run is pre-selected:
                            // pressing the button then compares the two ends of the history
                            // rather than a run with itself. The caller can pick any pair.
                            @for (index, run) in runs.iter().enumerate() {
                                option selected[index == default_from] value=(run.id.as_str()) {
                                    (run_label(run))
                                }
                            }
                        }
                    }
                    p {
                        label for="diff-to" { "To (the later run)" }
                        select id="diff-to" name="to" {
                            @for (index, run) in runs.iter().enumerate() {
                                option selected[index == default_to] value=(run.id.as_str()) {
                                    (run_label(run))
                                }
                            }
                        }
                    }
                    button type="submit" { "Diff the two runs" }
                }
            }
        }
    };
    let title = format!("modelwrite — {project} — analyses");
    layout::shell(
        &title,
        nav,
        Some(&identity.subject),
        identity.may(Permission::Administer),
        mechanism,
        body,
    )
}

/// A one-line label for one run in a selector: definition, commit and when.
fn run_label(run: &AnalysisRun) -> String {
    format!(
        "{} v{} @ {} ({})",
        run.definition.name,
        run.definition.version,
        short_hash(&run.commit_hash),
        run.created_at
    )
}

// ---------------------------------------------------------------------------
// Running one.

/// POST /ui/projects/:project/analyses - run a definition over a commit and STORE the record,
/// then land the caller on it. The result is the record, never a computed-on-view page.
pub async fn run_analysis_form(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(project): Path<String>,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    let mechanism = state.auth.mechanism();
    let identity = match resolve_identity(&state, &headers).await {
        Ok(identity) => identity,
        Err(error) => return layout::sign_in_page(mechanism, &error.message),
    };
    match perform_run(&state, &identity, &project, &form) {
        Ok(id) => Redirect::to(&format!(
            "/ui/projects/{}/analyses/{}",
            crate::ui::urlencode(&project),
            crate::ui::urlencode(&id)
        ))
        .into_response(),
        // A refusal is INFORMATION, not a lost page: the caller keeps the library and reads
        // why - including the refusal of a definition this server cannot express, which is
        // refused with a reason rather than approximated.
        Err(error) => match render_library(&state, &identity, &project, Some(&error.message)) {
            Ok(page) => layout::html_response(error.status, page),
            Err(inner) => layout::error_page(
                inner.status,
                Some(&identity.subject),
                mechanism,
                &inner.message,
            ),
        },
    }
}

/// The whole run: permission, the definition, the commit, the measurement, and the record.
/// Nothing in here writes to a model - the only writes are the run record and its audit entry.
fn perform_run(
    state: &ApiState,
    identity: &Identity,
    project: &str,
    form: &HashMap<String, String>,
) -> Result<String, ApiError> {
    // Running records a run, so it is a write path and takes the Write decision FIRST, before
    // anything is measured.
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
        return Err(ApiError::not_found(format!("project {}", project)));
    }
    let definition_id = value(form, "definition");
    // A definition the vocabulary cannot express is REFUSED with the reason, never silently
    // approximated.
    let definition = analyses::resolve(&definition_id)
        .map_err(|refusal| ApiError::unprocessable(refusal.reason, Vec::new()))?;
    let branch = match value(form, "branch") {
        branch if branch.is_empty() => "main".to_string(),
        branch => branch,
    };
    // A run is against a COMMIT, never a branch: the tip is resolved once, and the record
    // carries the hash it actually examined.
    let commit_hash = state
        .store_for(identity)
        .branch_tip(project, &branch)
        .map_err(map_store_error)?
        .ok_or_else(|| ApiError::not_found(format!("branch {} has no commits", branch)))?;
    let root = load_model(state.store_for(identity).as_ref(), project, &commit_hash)
        .map_err(map_store_error)?;

    let measured = analyses::measure(&definition, &root)
        .map_err(|refusal| ApiError::unprocessable(refusal.reason, Vec::new()))?;
    let run = analyses::record(
        project,
        &branch,
        &commit_hash,
        &identity.subject,
        &now_epoch(),
        &definition,
        &measured,
    );
    let audit = AuditEntry {
        id: 0,
        project: project.to_string(),
        at: now_seconds(),
        actor: identity.subject.clone(),
        mechanism: state.auth.mechanism().to_string(),
        authorizer: state.auth.authorizer().unwrap_or("").to_string(),
        action: crate::audit::ANALYSIS_RUN.to_string(),
        subject: run.id.clone(),
        detail: format!(
            "{} v{} over commit {} on {}",
            definition.name, definition.version, commit_hash, branch
        ),
    };
    state
        .store_for(identity)
        .record_analysis_run(&run, Some(&audit))
        .map_err(map_store_error)?;
    Ok(run.id)
}

fn value(form: &HashMap<String, String>, key: &str) -> String {
    form.get(key)
        .map(|value| value.trim().to_string())
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// One run.

/// GET /ui/projects/:project/analyses/:id - one stored run, addressable: its definition AS
/// RUN (naming the engine computations it used), the findings it measured, and - when
/// ?compare= names another run - the diff against it.
pub async fn run_page(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path((project, id)): Path<(String, String)>,
    Query(query): Query<RunQuery>,
) -> Response {
    let mechanism = state.auth.mechanism();
    let identity = match resolve_identity(&state, &headers).await {
        Ok(identity) => identity,
        Err(error) => return layout::sign_in_page(mechanism, &error.message),
    };
    match render_run(&state, &identity, &project, &id, query.compare.as_deref()) {
        Ok(page) => layout::html_response(StatusCode::OK, page),
        Err(error) => layout::error_page(
            error.status,
            Some(&identity.subject),
            mechanism,
            &error.message,
        ),
    }
}

fn render_run(
    state: &ApiState,
    identity: &Identity,
    project: &str,
    id: &str,
    compare: Option<&str>,
) -> Result<Markup, ApiError> {
    if !identity.may(Permission::Read) {
        return Err(ApiError::forbidden("read permission required"));
    }
    if !identity.may_reach(project) {
        return Err(ApiError::forbidden("project not in scope"));
    }
    let run = load_run(state, identity, project, id)?;
    let difference = match compare.filter(|other| !other.is_empty()) {
        Some(other) if other != id => {
            let other = load_run(state, identity, project, other)?;
            Some(analyses::diff(&other, &run))
        }
        _ => None,
    };
    let mut nav = layout::Nav::load(state, identity, Some(project))?;
    nav.section = Some("analyses");
    nav.branch = Some(run.branch.clone());
    nav.commit = Some(run.commit_hash.clone());
    Ok(run_markup(
        identity,
        state.auth.mechanism(),
        project,
        &run,
        difference.as_ref(),
        &nav,
    ))
}

fn load_run(
    state: &ApiState,
    identity: &Identity,
    project: &str,
    id: &str,
) -> Result<AnalysisRun, ApiError> {
    state
        .store_for(identity)
        .analysis_run(project, id)
        .map_err(map_store_error)?
        .ok_or_else(|| ApiError::not_found(format!("analysis run {}", id)))
}

fn run_markup(
    identity: &Identity,
    mechanism: &str,
    project: &str,
    run: &AnalysisRun,
    difference: Option<&FindingsDiff>,
    nav: &layout::Nav,
) -> Markup {
    let body = html! {
        h1 { (run.definition.name.as_str()) }
        p class="meta" {
            "definition " code { (run.definition.id.as_str()) } " v" (run.definition.version)
            " · provenance " (run.definition.provenance.as_str())
            " · engine " code { (run.engine_version.as_str()) }
        }
        p class="meta" {
            "commit " code title=(run.commit_hash.as_str()) { (run.commit_hash.as_str()) }
            @if !run.branch.is_empty() { " on " (run.branch.as_str()) }
            " · run " code title=(run.id.as_str()) { (run.id.as_str()) }
        }
        p class="meta" {
            "evidence " code title=(run.evidence_hash.as_str()) { (run.evidence_hash.as_str()) }
            @if !run.run_by.is_empty() { " · run by " (run.run_by.as_str()) }
            " · stored " code { (run.created_at.as_str()) }
        }
        p class="analysis-summary" {
            (analyses::summary_sentence(&run.definition, &run.findings, run.measured))
        }

        @if let Some(difference) = difference {
            (diff_markup(project, difference))
        }

        section class="model-section" id="definition" {
            h2 { "What this analysis named" }
            p class="meta" { (run.definition.summary.as_str()) }
            ul class="gaps" {
                @for computation in &run.definition.computations {
                    li {
                        code { (computation.id.as_str()) }
                        " — " (computation.provides.as_str())
                        " " span class="node-type" { (computation.measured_by.as_str()) }
                    }
                }
            }
        }

        @for severity in Severity::ALL {
            section class="model-section" id={ "findings-" (severity.as_str()) } {
                h2 { (severity_heading(severity)) " (" (run.count(severity)) ")" }
                @if run.count(severity) == 0 {
                    p class="none" { (severity_empty(severity)) }
                } @else {
                    ul class="findings" {
                        @for finding in run.of_severity(severity) {
                            (finding_markup(finding))
                        }
                    }
                }
            }
        }

        p class="meta" {
            a href={ "/ui/projects/" (crate::ui::urlencode(project)) "/analyses" } { "Back to the library" }
        }
    };
    let title = format!("modelwrite — {project} — analysis run");
    layout::shell(
        &title,
        nav,
        Some(&identity.subject),
        identity.may(Permission::Administer),
        mechanism,
        body,
    )
}

/// One finding: its severity, its ADDRESS (the identity that survives across runs), the
/// subject it is about, the one-line statement and the measurement behind it.
fn finding_markup(finding: &Finding) -> Markup {
    html! {
        li class="finding" {
            span class=(severity_class(finding.severity)) { (finding.severity.as_str()) }
            " "
            span class="finding-subject" {
                code { (finding.subject.as_str()) }
                @if !finding.subject_name.is_empty() { " — " (finding.subject_name.as_str()) }
            }
            p class="finding-id" { "finding " code { (finding.id.as_str()) } }
            p class="finding-statement" { (finding.statement.as_str()) }
            details class="finding-evidence" {
                summary { "evidence (the measurement behind this finding)" }
                pre { (evidence_text(finding)) }
            }
        }
    }
}

fn evidence_text(finding: &Finding) -> String {
    serde_json::to_string_pretty(&finding.evidence).unwrap_or_default()
}

// ---------------------------------------------------------------------------
// The diff between two runs.

/// GET /ui/projects/:project/analyses/diff?from=&to= - two stored runs compared by finding
/// identity: what appeared, what was resolved, and what was measured differently.
pub async fn diff_page(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(project): Path<String>,
    Query(query): Query<DiffQuery>,
) -> Response {
    let mechanism = state.auth.mechanism();
    let identity = match resolve_identity(&state, &headers).await {
        Ok(identity) => identity,
        Err(error) => return layout::sign_in_page(mechanism, &error.message),
    };
    match render_diff(&state, &identity, &project, &query) {
        Ok(page) => layout::html_response(StatusCode::OK, page),
        Err(error) => layout::error_page(
            error.status,
            Some(&identity.subject),
            mechanism,
            &error.message,
        ),
    }
}

fn render_diff(
    state: &ApiState,
    identity: &Identity,
    project: &str,
    query: &DiffQuery,
) -> Result<Markup, ApiError> {
    if !identity.may(Permission::Read) {
        return Err(ApiError::forbidden("read permission required"));
    }
    if !identity.may_reach(project) {
        return Err(ApiError::forbidden("project not in scope"));
    }
    let from_id = query
        .from
        .as_deref()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ApiError::bad_request("name the earlier run as from="))?;
    let to_id = query
        .to
        .as_deref()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ApiError::bad_request("name the later run as to="))?;
    let from = load_run(state, identity, project, from_id)?;
    let to = load_run(state, identity, project, to_id)?;
    let difference = analyses::diff(&from, &to);
    let mut nav = layout::Nav::load(state, identity, Some(project))?;
    nav.section = Some("analyses");
    nav.branch = Some(to.branch.clone());
    nav.commit = Some(to.commit_hash.clone());
    let body = html! {
        h1 { "Findings diff" }
        p class="meta" {
            "From " (from.definition.name.as_str()) " v" (from.definition.version)
            " at commit " code title=(from.commit_hash.as_str()) { (short_hash(&from.commit_hash)) }
            " to " (to.definition.name.as_str()) " v" (to.definition.version)
            " at commit " code title=(to.commit_hash.as_str()) { (short_hash(&to.commit_hash)) }
        }
        (diff_markup(project, &difference))
        p class="meta" {
            a href={ "/ui/projects/" (crate::ui::urlencode(project)) "/analyses" } { "Back to the library" }
        }
    };
    let title = format!("modelwrite — {project} — findings diff");
    Ok(layout::shell(
        &title,
        &nav,
        Some(&identity.subject),
        identity.may(Permission::Administer),
        state.auth.mechanism(),
        body,
    ))
}

/// The diff itself, shared by the run page and the diff page so the two cannot disagree.
fn diff_markup(project: &str, difference: &FindingsDiff) -> Markup {
    let from = short_hash(&difference.from.id);
    let to = short_hash(&difference.to.id);
    html! {
        section class="model-section" id="diff" {
            h2 { "Diff" }
            p class="meta" {
                "run " code title=(difference.from.id.as_str()) { (from) }
                " → run " code title=(difference.to.id.as_str()) { (to) }
                " · committed " code title=(difference.from.commit_hash.as_str()) { (short_hash(&difference.from.commit_hash)) }
                " → " code title=(difference.to.commit_hash.as_str()) { (short_hash(&difference.to.commit_hash)) }
                " · " (difference.unchanged) " unchanged"
            }
            @if difference.identical() {
                p class="check-passed" { "The two runs measured the same thing: nothing appeared, nothing was resolved and nothing was measured differently." }
            } @else {
                @if !difference.from.measured && difference.to.measured {
                    p class="meta" { "The earlier run could not measure this model; the later one could. That is a change in what the engine could see, not a change in the model." }
                }
                @if difference.from.measured && !difference.to.measured {
                    p class="meta" { "The earlier run measured this model; the later one could not. That is a change in what the engine could see, not a change in the model." }
                }
            }

            section class="diff-section" id="diff-appeared" {
                h3 { "Appeared (" (difference.appeared.len()) ")" }
                @if difference.appeared.is_empty() {
                    p class="none" { "Nothing appeared." }
                } @else {
                    ul class="findings" {
                        @for finding in &difference.appeared { (finding_markup(finding)) }
                    }
                }
            }

            section class="diff-section" id="diff-resolved" {
                h3 { "Resolved (" (difference.resolved.len()) ")" }
                @if difference.resolved.is_empty() {
                    p class="none" { "Nothing was resolved." }
                } @else {
                    ul class="findings" {
                        @for finding in &difference.resolved { (finding_markup(finding)) }
                    }
                }
            }

            section class="diff-section" id="diff-changed" {
                h3 { "Measured differently (" (difference.changed.len()) ")" }
                @if difference.changed.is_empty() {
                    p class="none" { "Nothing was measured differently." }
                } @else {
                    ul class="findings" {
                        @for change in &difference.changed {
                            li class="finding" {
                                span class=(severity_class(change.from.severity)) { (change.from.severity.as_str()) }
                                " → "
                                span class=(severity_class(change.to.severity)) { (change.to.severity.as_str()) }
                                " "
                                span class="finding-subject" {
                                    code { (change.to.subject.as_str()) }
                                    @if !change.to.subject_name.is_empty() { " — " (change.to.subject_name.as_str()) }
                                }
                                p class="finding-id" { "finding " code { (change.to.id.as_str()) } }
                                p class="finding-statement" {
                                    "was " (change.from.statement.as_str()) "; now " (change.to.statement.as_str())
                                }
                            }
                        }
                    }
                }
            }

            p class="meta" {
                a href={ "/ui/projects/" (crate::ui::urlencode(project)) "/analyses/diff?from=" (crate::ui::urlencode(difference.from.id.as_str())) "&to=" (crate::ui::urlencode(difference.to.id.as_str())) } {
                    "This diff has its own address"
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Presentation helpers. Every class used here already exists in the one stylesheet, and every
// colour is one of its tokens - no new colour is introduced by this module.

fn severity_class(severity: Severity) -> &'static str {
    match severity {
        Severity::Gap => "uncovered",
        Severity::Ok => "covered",
        Severity::Unknown => "mw-badge-unknown",
    }
}

fn severity_chip(severity: Severity, count: usize) -> Markup {
    html! {
        span class=(severity_class(severity)) { (count) " " (severity.as_str()) }
    }
}

fn severity_heading(severity: Severity) -> &'static str {
    match severity {
        Severity::Gap => "Gaps",
        Severity::Ok => "Measured clean",
        Severity::Unknown => "Not measured",
    }
}

fn severity_empty(severity: Severity) -> &'static str {
    match severity {
        Severity::Gap => "None: nothing this analysis measures is a gap.",
        Severity::Ok => "None.",
        Severity::Unknown => "None: every check this analysis names was measured.",
    }
}

fn short_hash(hash: &str) -> &str {
    hash.get(..8).unwrap_or(hash)
}
