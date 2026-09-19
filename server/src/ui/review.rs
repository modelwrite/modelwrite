// SPDX-License-Identifier: AGPL-3.0-or-later
//! The review pages: a compare view and a merge form.
//!
//! The compare view renders the ENGINE's own section-scoped diff ([okf::diff::diff]) and the
//! ENGINE's own gate verdict ([gate::run]) for a pair of commits or branches, never a diff the
//! page computes for itself. The merge form performs a merge through the SAME functions the
//! JSON handler [crate::merge_api::merge_branches] uses, in the SAME order, and renders a
//! conflicting merge as a resolvable page showing base, ours and theirs for every conflict
//! rather than as an error page: a 409 is information, not a failure.

use axum::extract::{Form, Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use maud::{html, Markup};
use serde::Deserialize;

use crate::api::{load_model, map_store_error, validate_name, verify_actor, ApiState};
use crate::auth::{identity as resolve_identity, Identity, Permission};
use crate::error::ApiError;
use crate::merge_api::{merge_core, MergeCore, MergeOutcome};
use crate::store::{Commit, Store};
use crate::ui::layout;

/// The query parameters of the compare page: two endpoints, each a commit hash or a branch
/// name. Both are required.
#[derive(Deserialize)]
pub struct CompareQuery {
    pub from: Option<String>,
    pub to: Option<String>,
}

/// The merge form body, submitted as `application/x-www-form-urlencoded`. There is no
/// author field: like the editor, the merge takes the author from the verified identity, so
/// a browser cannot put a name into the commit record. `holder` is optional; the empty string
/// means "no holder supplied", exactly as an absent field does in the JSON request.
#[derive(Deserialize)]
pub struct MergeForm {
    pub branch: String,
    pub other: String,
    pub message: String,
    #[serde(default)]
    pub holder: String,
}

/// `GET /ui/projects/:project/compare?from=&to=` - the engine's diff and gate verdict for a
/// pair of commits or branches, grouped by section.
pub async fn compare_page(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(project): Path<String>,
    Query(query): Query<CompareQuery>,
) -> Response {
    let mechanism = state.auth.mechanism();
    let identity = match resolve_identity(&state, &headers).await {
        Ok(identity) => identity,
        Err(error) => return layout::sign_in_page(mechanism, &error.message),
    };
    match render_compare_page(&state, &identity, &project, &query) {
        Ok(page) => layout::html_response(StatusCode::OK, page),
        Err(error) => layout::error_page(
            error.status,
            Some(&identity.subject),
            mechanism,
            &error.message,
        ),
    }
}

fn render_compare_page(
    state: &ApiState,
    identity: &Identity,
    project: &str,
    query: &CompareQuery,
) -> Result<Markup, ApiError> {
    // The SAME identity, Read-permission and project-scope decisions as the JSON handlers,
    // in the SAME order.
    if !identity.may(Permission::Read) {
        return Err(ApiError::forbidden("read permission required"));
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
    let from = query
        .from
        .as_deref()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ApiError::bad_request("the from endpoint is required"))?;
    let to = query
        .to
        .as_deref()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ApiError::bad_request("the to endpoint is required"))?;
    let from_hash = resolve_ref(state.store.as_ref(), project, from)?;
    let to_hash = resolve_ref(state.store.as_ref(), project, to)?;
    let from_commit = state
        .store
        .commit(project, &from_hash)
        .map_err(map_store_error)?;
    let to_commit = state
        .store
        .commit(project, &to_hash)
        .map_err(map_store_error)?;
    let reference =
        load_model(state.store.as_ref(), project, &from_hash).map_err(map_store_error)?;
    let candidate = load_model(state.store.as_ref(), project, &to_hash).map_err(map_store_error)?;

    // THE ENGINE'S DIFF and THE ENGINE'S GATE: rendered, never recomputed, so the page can
    // never disagree with the gate. Note what is NOT here: this page runs the engine and
    // records nothing. It writes no evidence file and no gate run, so it must not name one.
    let report = okf::diff::diff(&reference, &candidate);
    let outcome = gate::run(&reference, &candidate, false);
    let isolated = isolated_nodes(&outcome.evidence);

    let body = html! {
        h1 { "Compare" }
        p class="meta" {
            "from " code { (short_hash(&from_hash)) }
            @if let Some(commit) = &from_commit { " · " (commit.message.as_str()) }
            " → to " code { (short_hash(&to_hash)) }
            @if let Some(commit) = &to_commit { " · " (commit.message.as_str()) }
        }
        section id="diff" class="model-section" {
            h2 { "Diff" }
            @if report.equal {
                p { "The two models are identical." }
            } @else {
                (diff_markup(&report))
            }
        }
        section id="gate" class="model-section" {
            h2 { "Gate" }
            p {
                "Verdict: "
                @if outcome.passed { span class="covered" { "passed" } }
                @else { span class="uncovered" { "failed" } }
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
                    @for node in &isolated { li { code { (node.as_str()) } } }
                }
            }
            // A COMPARISON IS NOT A GATE RUN. This page computes a verdict by calling the
            // engine, but it records nothing and writes no evidence file, and only the gate
            // endpoint does. Printing the path of a file that does not exist would tell a
            // reviewer the record lives here when it does not - the false assurance this
            // whole platform is built to avoid.
            p class="meta" {
                "This comparison is not recorded. Run the gate for "
                code { (short_hash(&from_hash)) } " → " code { (short_hash(&to_hash)) }
                " to persist an evidence record; the run will then appear in the gate list."
            }
        }
    };
    let title = format!("modelwrite — {} — compare", project);
    let mut nav = layout::Nav::load(state, identity, Some(project))?;
    nav.section = Some("changes");
    Ok(layout::shell(
        &title,
        &nav,
        Some(&identity.subject),
        identity.may(Permission::Administer),
        state.auth.mechanism(),
        body,
    ))
}

/// Resolve one endpoint to a commit hash: an existing commit hash wins, otherwise the value
/// is treated as a branch name and resolves to its tip. Both readings use the store's own
/// lookups, so no new store method exists for this page.
fn resolve_ref(store: &dyn Store, project: &str, value: &str) -> Result<String, ApiError> {
    if store
        .commit(project, value)
        .map_err(map_store_error)?
        .is_some()
    {
        return Ok(value.to_string());
    }
    if let Some(tip) = store.branch_tip(project, value).map_err(map_store_error)? {
        return Ok(tip);
    }
    Err(ApiError::not_found(format!("commit or branch {}", value)))
}

/// The isolated graph nodes the gate reported, read straight out of the evidence's
/// `integration.isolated` field rather than recomputed.
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

// ---------------------------------------------------------------------------
// The diff, grouped by section.

/// The display order of the sections, and the human name for each. The trailing empty-string
/// entry catches a malformed key that has no section separator, so nothing is silently
/// dropped.
const SECTION_ORDER: [(&str, &str); 9] = [
    ("structure", "Structure"),
    ("interfaces", "Interfaces"),
    ("signals", "Signals"),
    ("requirements", "Requirements"),
    ("state", "State machine"),
    ("activity", "Activities"),
    ("graphnode", "Graph nodes"),
    ("doc", "Document"),
    ("", "Other"),
];

fn section_of(key: &str) -> &str {
    key.split_once(':')
        .map(|(section, _)| section)
        .unwrap_or("")
}

struct SectionDiff {
    name: &'static str,
    added: Vec<String>,
    removed: Vec<String>,
    changed: Vec<String>,
}

/// Group the engine's diff report by section. Element and attribute keys are
/// `<section>:<id>`; the edge keys are JSON arrays and are rendered separately.
fn section_diffs(report: &okf::diff::DiffReport) -> Vec<SectionDiff> {
    let mut sections: Vec<SectionDiff> = SECTION_ORDER
        .iter()
        .map(|(_, name)| SectionDiff {
            name,
            added: Vec::new(),
            removed: Vec::new(),
            changed: Vec::new(),
        })
        .collect();
    let index = |key: &str| -> usize {
        let section = section_of(key);
        SECTION_ORDER
            .iter()
            .position(|(candidate, _)| *candidate == section)
            .unwrap_or(SECTION_ORDER.len() - 1)
    };
    for key in &report.missing_elements {
        sections[index(key)].removed.push(key.clone());
    }
    for key in &report.extra_elements {
        sections[index(key)].added.push(key.clone());
    }
    for key in &report.changed_attributes {
        sections[index(key)].changed.push(key.clone());
    }
    sections
}

fn diff_markup(report: &okf::diff::DiffReport) -> Markup {
    let sections = section_diffs(report);
    html! {
        @for section in &sections {
            @if !section.added.is_empty() || !section.removed.is_empty() || !section.changed.is_empty() {
                h3 { (section.name) }
                @if !section.removed.is_empty() {
                    h4 class="diff-remove" { "Removed" }
                    ul class="diff-list" {
                        @for key in &section.removed { li { code { (key.as_str()) } } }
                    }
                }
                @if !section.added.is_empty() {
                    h4 class="diff-add" { "Added" }
                    ul class="diff-list" {
                        @for key in &section.added { li { code { (key.as_str()) } } }
                    }
                }
                @if !section.changed.is_empty() {
                    h4 class="diff-change" { "Changed" }
                    ul class="diff-list" {
                        @for key in &section.changed { li { code { (key.as_str()) } } }
                    }
                }
            }
        }
        @if !report.missing_edges.is_empty() || !report.extra_edges.is_empty() {
            h3 { "Graph edges" }
            @if !report.missing_edges.is_empty() {
                h4 class="diff-remove" { "Removed" }
                ul class="diff-list" {
                    @for key in &report.missing_edges { li { code { (edge_display(key)) } } }
                }
            }
            @if !report.extra_edges.is_empty() {
                h4 class="diff-add" { "Added" }
                ul class="diff-list" {
                    @for key in &report.extra_edges { li { code { (edge_display(key)) } } }
                }
            }
        }
    }
}

/// An edge key is a JSON array of source, target, kind and label; render it readably.
fn edge_display(key: &str) -> String {
    if let Ok(parts) = serde_json::from_str::<Vec<String>>(key) {
        let source = parts.first().cloned().unwrap_or_default();
        let target = parts.get(1).cloned().unwrap_or_default();
        let kind = parts.get(2).cloned().unwrap_or_default();
        let label = parts.get(3).cloned().unwrap_or_default();
        if label.is_empty() {
            format!("{} → {} [{}]", source, target, kind)
        } else {
            format!("{} → {} [{} {}]", source, target, kind, label)
        }
    } else {
        key.to_string()
    }
}

// ---------------------------------------------------------------------------
// The merge form.

/// `POST /ui/projects/:project/merge` - perform a merge through the same code path as the
/// JSON endpoint, and render the outcome: a success page for a clean merge, or a conflict
/// page showing base, ours and theirs for every conflict.
pub async fn merge_form(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(project): Path<String>,
    Form(form): Form<MergeForm>,
) -> Response {
    let mechanism = state.auth.mechanism();
    let identity = match resolve_identity(&state, &headers).await {
        Ok(identity) => identity,
        Err(error) => return layout::sign_in_page(mechanism, &error.message),
    };
    let nav = match layout::Nav::load(&state, &identity, Some(&project)) {
        Ok(mut nav) => {
            nav.section = Some("changes");
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
    match perform_merge(&state, &identity, &project, &form) {
        Ok(result) => {
            let status = match &result.outcome {
                MergeOutcomeKind::Clean { .. } => StatusCode::CREATED,
                MergeOutcomeKind::Conflict { .. } => StatusCode::CONFLICT,
            };
            layout::html_response(
                status,
                merge_result_page(&identity, mechanism, &project, &result, &nav),
            )
        }
        Err(error) => layout::error_page(
            error.status,
            Some(&identity.subject),
            mechanism,
            &error.message,
        ),
    }
}

enum MergeOutcomeKind {
    Clean {
        commit: Box<Commit>,
    },
    Conflict {
        conflicts: Vec<crate::merge::Conflict>,
    },
}

struct MergeResult {
    branch: String,
    other: String,
    base: String,
    outcome: MergeOutcomeKind,
}

/// The merge itself, through the SAME core the JSON handler uses
/// ([crate::merge_api::merge_core]), with the same permission decisions and the same audit
/// entries. A conflict is a RESULT here, not an error: it carries every conflicting subject's
/// base, ours and theirs values for the page to render. The author is the verified identity,
/// never a browser field.
fn perform_merge(
    state: &ApiState,
    identity: &Identity,
    project: &str,
    form: &MergeForm,
) -> Result<MergeResult, ApiError> {
    if !identity.may(Permission::Write) {
        return Err(ApiError::forbidden("write permission required"));
    }
    if !identity.may_reach(project) {
        return Err(ApiError::forbidden("project not in scope"));
    }
    let holder = form.holder.as_str();
    let holder = if holder.is_empty() {
        None
    } else {
        Some(holder)
    };
    verify_actor(&state.auth, identity, holder)?;
    validate_name("branch name", &form.branch)?;
    validate_name("branch name", &form.other)?;
    // The author is the verified identity's subject, exactly as the editor resolves it: the
    // form has no author field, so a browser cannot put a name into the commit record.
    let author = identity.subject.clone();

    match merge_core(
        state.store.as_ref(),
        project,
        &MergeCore {
            branch: &form.branch,
            other: &form.other,
            author: &author,
            message: &form.message,
            holder,
            actor: &identity.subject,
            mechanism: state.auth.mechanism(),
            authorizer: state.auth.authorizer().unwrap_or(""),
        },
    )
    .map_err(map_store_error)?
    {
        MergeOutcome::Merged { commit, base } => Ok(MergeResult {
            branch: form.branch.clone(),
            other: form.other.clone(),
            base,
            outcome: MergeOutcomeKind::Clean { commit },
        }),
        MergeOutcome::Conflict { base, conflicts } => Ok(MergeResult {
            branch: form.branch.clone(),
            other: form.other.clone(),
            base,
            outcome: MergeOutcomeKind::Conflict { conflicts },
        }),
    }
}

fn merge_result_page(
    identity: &Identity,
    mechanism: &str,
    project: &str,
    result: &MergeResult,
    nav: &layout::Nav,
) -> Markup {
    let body = match &result.outcome {
        MergeOutcomeKind::Clean { commit } => html! {
            h1 { "Merge complete" }
            p {
                "Merged " strong { (result.other.as_str()) } " into " strong { (result.branch.as_str()) } "."
            }
            p { "Commit " code { (short_hash(&commit.hash)) } }
            p { "Merge base " code { (short_hash(&result.base)) } }
            p {
                a href={ "/ui/projects/" (crate::ui::urlencode(project)) "/model?commit=" (crate::ui::urlencode(commit.hash.as_str())) } {
                    "View the merged model"
                }
            }
        },
        MergeOutcomeKind::Conflict { conflicts } => html! {
            h1 { "Merge conflict" }
            p {
                "Merging " strong { (result.other.as_str()) } " into " strong { (result.branch.as_str()) }
                " conflicts; nothing was written. Resolve each conflict, then merge again."
            }
            p class="meta" { "Merge base " code { (short_hash(&result.base)) } }
            @for conflict in conflicts {
                section class="conflict" {
                    h2 { (conflict.subject.as_str()) }
                    p class="meta" { "kind: " code { (conflict.kind.as_str()) } }
                    div class="conflict-columns" {
                        div class="conflict-side" {
                            h3 { "Base" }
                            (conflict_value(&conflict.base))
                        }
                        div class="conflict-side" {
                            h3 { "Ours" }
                            (conflict_value(&conflict.ours))
                        }
                        div class="conflict-side" {
                            h3 { "Theirs" }
                            (conflict_value(&conflict.theirs))
                        }
                    }
                }
            }
            h2 { "Resolve" }
            p { "Edit one of the branches to choose its value, then merge again." }
            (merge_form_markup(project, &result.branch, &result.other))
        },
    };
    let title = format!("modelwrite — {} — merge", project);
    layout::shell(
        &title,
        nav,
        Some(&identity.subject),
        identity.may(Permission::Administer),
        mechanism,
        body,
    )
}

fn conflict_value(value: &Option<String>) -> Markup {
    match value {
        Some(text) => html! { pre class="conflict-value" { (text.as_str()) } },
        None => html! { pre class="conflict-value" { "(absent)" } },
    }
}

/// The merge form, shared by the project page and the conflict page's resolution form.
pub fn merge_form_markup(project: &str, branch: &str, other: &str) -> Markup {
    html! {
        form method="post" action={ "/ui/projects/" (crate::ui::urlencode(project)) "/merge" } class="merge-form" {
            p {
                label for="branch" { "Merge into branch" }
                input type="text" id="branch" name="branch" value=(branch) required;
            }
            p {
                label for="other" { "from branch" }
                input type="text" id="other" name="other" value=(other) required;
            }
            p {
                label for="message" { "message" }
                input type="text" id="message" name="message" required;
            }
            p {
                label for="holder" { "holder (optional)" }
                input type="text" id="holder" name="holder";
            }
            button type="submit" { "Merge" }
        }
    }
}

fn short_hash(hash: &str) -> &str {
    hash.get(..8).unwrap_or(hash)
}
