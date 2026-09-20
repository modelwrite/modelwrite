// SPDX-License-Identifier: AGPL-3.0-or-later
//! The review pages: a compare view and a merge form.
//!
//! The compare view renders the ENGINE's own section-scoped diff ([okf::diff::diff]) and the
//! ENGINE's own gate verdict ([gate::run]) for a pair of commits or branches, never a diff the
//! page computes for itself. The merge form performs a merge through the SAME functions the
//! JSON handler [crate::merge_api::merge_branches] uses, in the SAME order, and renders a
//! conflicting merge as a resolvable page showing base, ours and theirs for every conflict
//! rather than as an error page: a 409 is information, not a failure.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use axum::extract::{Form, Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use maud::{html, Markup};
use serde::Deserialize;

use graph::requirement_coverage;
use okf::types::{OkfRoot, Requirement, SubsystemReference};

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
    let mut nav = layout::Nav::load(state, identity, Some(project))?;
    nav.section = Some("changes");
    let from = query
        .from
        .as_deref()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ApiError::bad_request("the from endpoint is required"))?;
    let Some(to) = query.to.as_deref().filter(|value| !value.is_empty()) else {
        // No "to" chosen yet: offer the other versions, with the "from" side already fixed.
        let body = compare_chooser_markup(project, from, &nav);
        let title = format!("modelwrite — {} — compare", project);
        return Ok(layout::shell(
            &title,
            &nav,
            Some(&identity.subject),
            identity.may(Permission::Administer),
            state.auth.mechanism(),
            body,
        ));
    };
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
    let from_label = side_label(from, &from_hash);
    let to_label = side_label(to, &to_hash);
    let impact = impact_panel(&from_label, &to_label, &reference, &candidate);

    let body = html! {
        h1 { "Compare" }
        p class="meta" {
            "from " code { (short_hash(&from_hash)) }
            @if let Some(commit) = &from_commit { " · " (commit.message.as_str()) }
            " → to " code { (short_hash(&to_hash)) }
            @if let Some(commit) = &to_commit { " · " (commit.message.as_str()) }
        }
        @if let Some(panel) = impact { (panel) }
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
    Ok(layout::shell(
        &title,
        &nav,
        Some(&identity.subject),
        identity.may(Permission::Administer),
        state.auth.mechanism(),
        body,
    ))
}

/// The compare chooser: when the "to" side is not chosen yet, offer each version as a link
/// with the "from" side already fixed, plus a typed form for two arbitrary endpoints.
fn compare_chooser_markup(project: &str, from: &str, nav: &layout::Nav) -> Markup {
    html! {
        h1 { "Compare" }
        p class="meta" {
            "from " code { (from) } " — choose the other version to compare against."
        }
        @if nav.branches.is_empty() {
            p { "This project has no versions yet." }
        } @else {
            ul class="branches" {
                @for (name, tip) in &nav.branches {
                    @if name.as_str() != from {
                        li {
                            a class="branch-name" href={
                                "/ui/projects/" (crate::ui::urlencode(project))
                                "/compare?from=" (crate::ui::urlencode(from))
                                "&to=" (crate::ui::urlencode(name.as_str()))
                            } {
                                (name) code class="tip" { (short_hash(tip)) }
                            }
                        }
                    }
                }
            }
        }
        h2 { "Or type two endpoints" }
        form method="get" action={ "/ui/projects/" (crate::ui::urlencode(project)) "/compare" } class="compare-form" {
            p {
                label for="from" { "from" }
                input type="text" id="from" name="from" value=(from);
            }
            p {
                label for="to" { "to" }
                input type="text" id="to" name="to" placeholder="branch or commit";
            }
            button type="submit" { "Compare" }
        }
    }
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
// The variant impact panel.

/// One subsystem reference rendered as a row, keyed by its role: the project@revision each
/// side pins, and whether the two sides differ.
struct ImpactRef {
    role: String,
    from: Option<(String, String)>,
    to: Option<(String, String)>,
    changed: bool,
}

/// The coverage state of a platform requirement across the two compared versions.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ImpactState {
    Both,
    FromOnly,
    ToOnly,
    Neither,
}

impl ImpactState {
    fn data(self) -> &'static str {
        match self {
            ImpactState::Both => "both",
            ImpactState::FromOnly => "from",
            ImpactState::ToOnly => "to",
            ImpactState::Neither => "neither",
        }
    }
}

struct ImpactRequirement {
    req_id: String,
    name: String,
    state: ImpactState,
}

/// A short, human label for one compare side: the branch name as typed, or a shortened hash
/// when the endpoint was itself a commit hash.
fn side_label(endpoint: &str, resolved: &str) -> String {
    if endpoint == resolved {
        short_hash(endpoint).to_string()
    } else {
        endpoint.to_string()
    }
}

/// The variant impact panel for a PLATFORM comparison: the two sides' references (role-keyed,
/// changed choices marked) and each platform requirement's coverage state on each side — the
/// ENGINE's [graph::requirement_coverage] over each version's own graph, never a
/// reimplementation — plus a plain summary and the honest measured-vs-asserted boundary.
/// Returns `None` when neither side declares a subsystem reference, so there is no variant
/// impact to show.
fn impact_panel(
    from_label: &str,
    to_label: &str,
    reference: &OkfRoot,
    candidate: &OkfRoot,
) -> Option<Markup> {
    if reference.references.is_empty() && candidate.references.is_empty() {
        return None;
    }

    let rows = reference_rows(reference, candidate);
    let requirements = requirement_rows(reference, candidate);
    let summary = summary_line(&requirements, from_label, to_label);

    Some(html! {
        section id="impact" class="model-section" {
            h2 { "Variant impact" }
            p class="meta" {
                "Which platform requirements each side satisfies, and where the two versions' subsystem choices differ."
            }

            h3 { "References" }
            table class="impact-refs" {
                thead {
                    tr {
                        th { "role" }
                        th { "from · " (from_label) }
                        th { "to · " (to_label) }
                    }
                }
                tbody {
                    @for row in &rows {
                        tr class=(if row.changed { "impact-ref changed" } else { "impact-ref" })
                           data-role=(row.role.as_str())
                           data-changed=(if row.changed { "true" } else { "false" }) {
                            td class="impact-role" { span class="role-chip" { (row.role.as_str()) } }
                            td class="impact-side" { (reference_cell(&row.from)) }
                            td class="impact-side" {
                                (reference_cell(&row.to))
                                @if row.changed { span class="impact-changed" { "changed" } }
                            }
                        }
                    }
                }
            }

            h3 { "Requirements" }
            ul class="impact-reqs" {
                @for req in &requirements {
                    li class="impact-req"
                       data-state=(req.state.data())
                       data-req-id=(req.req_id.as_str()) {
                        code class="impact-req-id" { (req.req_id.as_str()) }
                        span class="impact-req-name" { (req.name.as_str()) }
                        span class=(state_chip_class(req.state)) {
                            (state_chip_text(req.state, from_label, to_label))
                        }
                    }
                }
            }

            p class="impact-summary" { (summary.as_str()) }

            (impact_boundary())
        }
    })
}

/// `project@short-revision`, or an absent marker when one side has no reference for a role.
fn reference_cell(value: &Option<(String, String)>) -> Markup {
    match value {
        Some((project, revision)) => html! {
            span class="impact-project" { (project.as_str()) }
            code title=(revision.as_str()) { "@" (short_hash(revision.as_str())) }
        },
        None => html! { span class="none" { "—" } },
    }
}

/// The two sides' references as role-keyed rows. A role whose project or revision differs is
/// marked changed; a role present on only one side reads as a change too (added or removed).
fn reference_rows(reference: &OkfRoot, candidate: &OkfRoot) -> Vec<ImpactRef> {
    let from_map: BTreeMap<&str, &SubsystemReference> = reference
        .references
        .iter()
        .map(|r| (r.role.as_str(), r))
        .collect();
    let to_map: BTreeMap<&str, &SubsystemReference> = candidate
        .references
        .iter()
        .map(|r| (r.role.as_str(), r))
        .collect();

    let mut roles: BTreeSet<&str> = BTreeSet::new();
    roles.extend(from_map.keys().copied());
    roles.extend(to_map.keys().copied());

    roles
        .into_iter()
        .map(|role| {
            let from = from_map
                .get(role)
                .map(|r| (r.project.clone(), r.revision.clone()));
            let to = to_map
                .get(role)
                .map(|r| (r.project.clone(), r.revision.clone()));
            ImpactRef {
                role: role.to_string(),
                changed: from != to,
                from,
                to,
            }
        })
        .collect()
}

/// Each platform requirement (the union across both sides) with its coverage state, from the
/// ENGINE's [graph::requirement_coverage] over each version's own graph. A requirement that
/// is absent on one side is simply not satisfied there.
fn requirement_rows(reference: &OkfRoot, candidate: &OkfRoot) -> Vec<ImpactRequirement> {
    let from_uncovered = uncovered_ids(reference);
    let to_uncovered = uncovered_ids(candidate);
    let from_by_id: BTreeMap<&str, &Requirement> = reference
        .requirements
        .iter()
        .map(|r| (r.id.as_str(), r))
        .collect();
    let to_by_id: BTreeMap<&str, &Requirement> = candidate
        .requirements
        .iter()
        .map(|r| (r.id.as_str(), r))
        .collect();

    let mut seen: HashSet<String> = HashSet::new();
    let mut rows = Vec::new();
    for req in reference
        .requirements
        .iter()
        .chain(candidate.requirements.iter())
    {
        if !seen.insert(req.id.clone()) {
            continue;
        }
        let from_present = from_by_id.contains_key(req.id.as_str());
        let to_present = to_by_id.contains_key(req.id.as_str());
        let from_satisfied = from_present && !from_uncovered.contains(req.id.as_str());
        let to_satisfied = to_present && !to_uncovered.contains(req.id.as_str());
        let state = match (from_satisfied, to_satisfied) {
            (true, true) => ImpactState::Both,
            (true, false) => ImpactState::FromOnly,
            (false, true) => ImpactState::ToOnly,
            (false, false) => ImpactState::Neither,
        };
        let display = to_by_id
            .get(req.id.as_str())
            .or_else(|| from_by_id.get(req.id.as_str()))
            .copied()
            .unwrap_or(req);
        let req_id = if display.req_id.trim().is_empty() {
            display.id.clone()
        } else {
            display.req_id.clone()
        };
        rows.push(ImpactRequirement {
            req_id,
            name: display.name.clone(),
            state,
        });
    }
    rows
}

/// The requirement ids the ENGINE reports as uncovered on one side, or empty when that side
/// carries no graph (there is then no coverage to report).
fn uncovered_ids(root: &OkfRoot) -> HashSet<String> {
    match root.graph.as_ref() {
        Some(_) => requirement_coverage(root).uncovered.into_iter().collect(),
        None => HashSet::new(),
    }
}

/// The plain-words summary: one clause per coverage state.
fn summary_line(requirements: &[ImpactRequirement], from_label: &str, to_label: &str) -> String {
    let both: Vec<&str> = requirements
        .iter()
        .filter(|r| r.state == ImpactState::Both)
        .map(|r| r.req_id.as_str())
        .collect();
    let from_only: Vec<&str> = requirements
        .iter()
        .filter(|r| r.state == ImpactState::FromOnly)
        .map(|r| r.req_id.as_str())
        .collect();
    let to_only: Vec<&str> = requirements
        .iter()
        .filter(|r| r.state == ImpactState::ToOnly)
        .map(|r| r.req_id.as_str())
        .collect();
    let neither: Vec<&str> = requirements
        .iter()
        .filter(|r| r.state == ImpactState::Neither)
        .map(|r| r.req_id.as_str())
        .collect();

    let mut parts: Vec<String> = Vec::new();
    if !both.is_empty() {
        parts.push(group_phrase(&both, "satisfied in both versions"));
    }
    if !from_only.is_empty() {
        parts.push(group_phrase(
            &from_only,
            &format!("satisfied only in {}", from_label),
        ));
    }
    if !to_only.is_empty() {
        parts.push(group_phrase(
            &to_only,
            &format!("satisfied only in {}", to_label),
        ));
    }
    if !neither.is_empty() {
        parts.push(group_phrase(&neither, "satisfied in neither version"));
    }
    if parts.is_empty() {
        "No platform requirements to compare.".to_string()
    } else {
        format!("{}.", parts.join("; "))
    }
}

/// "Requirement CS-1 is satisfied in both versions" / "Requirements CS-1 and CS-2 are …",
/// with the subject and verb agreeing.
fn group_phrase(ids: &[&str], predicate: &str) -> String {
    let subject = if ids.len() == 1 {
        "Requirement"
    } else {
        "Requirements"
    };
    let verb = if ids.len() == 1 { "is" } else { "are" };
    format!("{} {} {} {}", subject, join_ids(ids), verb, predicate)
}

/// "CS-1", "CS-1 and CS-2", or "CS-1, CS-2 and CS-3".
fn join_ids(ids: &[&str]) -> String {
    match ids {
        [] => String::new(),
        [one] => (*one).to_string(),
        [a, b] => format!("{} and {}", a, b),
        _ => {
            let (last, rest) = ids.split_last().expect("a non-empty id list");
            format!("{} and {}", rest.join(", "), last)
        }
    }
}

/// The chip class for a requirement's coverage state: the green/red engine chips for "both"
/// and "neither", the amber partial chip for a one-sided difference.
fn state_chip_class(state: ImpactState) -> &'static str {
    match state {
        ImpactState::Both => "impact-state covered",
        ImpactState::Neither => "impact-state uncovered",
        ImpactState::FromOnly | ImpactState::ToOnly => "impact-state impact-partial",
    }
}

/// The chip label for a requirement's coverage state, naming the side for a one-sided
/// difference.
fn state_chip_text(state: ImpactState, from_label: &str, to_label: &str) -> String {
    match state {
        ImpactState::Both => "both".to_string(),
        ImpactState::FromOnly => format!("{} only", from_label),
        ImpactState::ToOnly => format!("{} only", to_label),
        ImpactState::Neither => "neither".to_string(),
    }
}

/// The honest boundary: what this panel computed (the platform model's own coverage on each
/// side, and the reference differences) and what it did not (coverage proved inside a
/// referenced subsystem), in the same measured-vs-asserted discipline as the composition
/// page.
fn impact_boundary() -> Markup {
    html! {
        div class="impact-boundary" {
            h3 { "What this panel proves — and what it does not" }
            p class="meta" {
                "Coverage is computed over each version's own graph; what a referenced subsystem satisfies internally is not."
            }
            div class="boundary-grid" {
                div class="boundary-panel boundary-measured" {
                    h3 { "Computed" }
                    ul {
                        li {
                            "the platform model's own requirement coverage on each side, from the engine's "
                            code { "requirement_coverage" } " over that version's graph"
                        }
                        li {
                            "the subsystem reference differences, shown role → project@revision and marked where they change"
                        }
                    }
                }
                div class="boundary-panel boundary-asserted" {
                    h3 { "Not computed — asserted, not proved" }
                    ul {
                        li {
                            "coverage proved inside a referenced subsystem: a platform requirement satisfied by an activity allocated to a role is not proved to be satisfied by the bound element inside that subsystem"
                        }
                        li {
                            "where a difference in a requirement's state is explained by a changed reference, it is explained by the change but not proved by it"
                        }
                    }
                }
            }
            p class="fidelity-note" {
                "An activity's link to a specific element inside a subsystem is carried by its "
                code { "allocatedTo" } " attribute (a role name), not a first-class cross-model edge, so coverage inside the referenced subsystems is not computed here."
            }
        }
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
