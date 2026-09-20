// SPDX-License-Identifier: AGPL-3.0-or-later
//! The Composition section: the system-of-systems view (which subsystems a platform model
//! integrates, at which pinned revision) and the business-process view (the ordered flow
//! across those subsystems), plus the measured-vs-asserted boundary the compositional gate
//! (S1) states as data.
//!
//! The resolve chip reuses the SAME [crate::api::check_reference] the resolve endpoint and
//! the commit path use, so the page can never disagree with the JSON answer. The boundary
//! wording is read from [crate::composition::check]'s own evidence, never re-typed here, so
//! what the page says "measured" and "asserted" is exactly what the gate reports.

use std::collections::{HashMap, HashSet, VecDeque};

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use maud::{html, Markup};
use serde_json::Value;

use okf::types::{OkfRoot, Requirement};

use crate::api::{check_reference, map_store_error, ApiState};
use crate::auth::{identity as resolve_identity, Identity, Permission};
use crate::error::ApiError;
use crate::store::Commit;
use crate::ui::layout;
use crate::ui::model::{load_view, view_branch, LoadedView, ModelQuery};

struct SubsystemCard {
    project: String,
    revision: String,
    role: String,
    bounds: Vec<String>,
    resolves: bool,
    reason: Option<String>,
}

#[derive(Clone)]
struct RequirementRef {
    req_id: String,
    name: String,
}

struct ProcessStep {
    id: String,
    name: String,
    allocated_to: Option<String>,
    satisfies: Vec<RequirementRef>,
}

struct ProcessLayer {
    steps: Vec<ProcessStep>,
}

/// `GET /ui/projects/:project/composition` - the system-of-systems and process views.
pub async fn composition_page(
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
    match render_composition_page(&state, &identity, &project, &query) {
        Ok(page) => layout::html_response(StatusCode::OK, page),
        Err(error) => layout::error_page(
            error.status,
            Some(&identity.subject),
            mechanism,
            &error.message,
        ),
    }
}

fn render_composition_page(
    state: &ApiState,
    identity: &Identity,
    project: &str,
    query: &ModelQuery,
) -> Result<Markup, ApiError> {
    let nav = layout::Nav::load(state, identity, Some(project))?;
    match load_view(state, identity, project, query)? {
        LoadedView::Empty => Ok(crate::ui::create::empty_model_page(
            identity,
            state.auth.mechanism(),
            project,
            &[],
            &nav,
        )),
        LoadedView::Model { commit, root } => {
            let mut nav = nav;
            nav.section = Some("composition");
            nav.branch = Some(view_branch(query, &commit));
            nav.commit = Some(commit.hash.clone());
            composition_markup(state, identity, project, &commit, &root, &nav)
        }
    }
}

fn composition_markup(
    state: &ApiState,
    identity: &Identity,
    project: &str,
    commit: &Commit,
    root: &OkfRoot,
    nav: &layout::Nav,
) -> Result<Markup, ApiError> {
    let raw = raw_model_json(state, commit)?;
    let allocated = allocated_to_map(&raw);

    let mut cards: Vec<SubsystemCard> = Vec::new();
    let mut resolved_count = 0usize;
    for reference in &root.references {
        let reason = check_reference(
            state.store.as_ref(),
            &reference.project,
            &reference.revision,
        )
        .map_err(map_store_error)?;
        let resolves = reason.is_none();
        if resolves {
            resolved_count += 1;
        }
        cards.push(SubsystemCard {
            project: reference.project.clone(),
            revision: reference.revision.clone(),
            role: reference.role.clone(),
            bounds: reference.bounds.clone(),
            resolves,
            reason,
        });
    }

    let composition =
        crate::composition::check(state.store.as_ref(), root).map_err(map_store_error)?;
    let (measured, asserted) = boundary_lists(&composition.evidence);

    let example_reachable = nav.projects.iter().any(|p| p == "cafe-stand");

    let body = html! {
        h1 { "Composition" }
        p class="meta" {
            "The system-of-systems view: which subsystems this platform integrates, at which pinned revision, and the business process that runs across them."
        }
        @if root.references.is_empty() {
            (no_references_markup(example_reachable))
        } @else {
            (subsystems_section(&cards, resolved_count))
            (boundary_section(&measured, &asserted))
            (process_section(root, &allocated))
        }
    };
    let title = format!("modelwrite — {} — composition", project);
    Ok(layout::shell(
        &title,
        nav,
        Some(&identity.subject),
        identity.may(Permission::Administer),
        state.auth.mechanism(),
        body,
    ))
}

/// A model with no references has no system-of-systems to compose; say so and point at the
/// platform model that does.
fn no_references_markup(example_reachable: bool) -> Markup {
    html! {
        section class="model-section" id="subsystems" {
            h2 { "Subsystems" }
            p {
                "This model declares no subsystem references, so there is no system-of-systems composition to show."
            }
            p {
                @if example_reachable {
                    "See "
                    a href="/ui/projects/cafe-stand/composition" { "cafe-stand" }
                    " — the platform model that composes the three subsystems — for the composition and process views."
                } @else {
                    "cafe-stand is the platform model that composes the three subsystems and carries the composition and process views."
                }
            }
        }
    }
}

fn subsystems_section(cards: &[SubsystemCard], resolved_count: usize) -> Markup {
    let total = cards.len();
    let summary = if total == 1 {
        "1 subsystem, all resolving at its pinned revision".to_string()
    } else if resolved_count == total {
        format!("{total} subsystems, all resolving at their pinned revisions")
    } else {
        format!("{resolved_count} of {total} resolve")
    };
    html! {
        section class="model-section" id="subsystems" {
            h2 { "Subsystems" }
            p class="composition-summary" { (summary) }
            ul class="subsystems" {
                @for card in cards {
                    (subsystem_card(card))
                }
            }
        }
    }
}

fn subsystem_card(card: &SubsystemCard) -> Markup {
    html! {
        li class="subsystem-card" data-resolves=(if card.resolves { "true" } else { "false" }) {
            div class="subsystem-head" {
                span class="role-chip" { (card.role.as_str()) }
                @if card.resolves {
                    span class="covered" { "resolves" }
                } @else {
                    span class="uncovered" { "does not resolve" }
                }
            }
            div class="subsystem-project" { (card.project.as_str()) }
            div class="subsystem-revision" {
                "revision " code title=(card.revision.as_str()) { (short_hash(&card.revision)) }
            }
            @if let Some(reason) = &card.reason {
                p class="subsystem-reason" { (reason.as_str()) }
            }
            @if card.bounds.is_empty() {
                div class="subsystem-bounds" {
                    span class="bounds-label" { "bounds" }
                    span class="none" { "whole subsystem" }
                }
            } @else {
                div class="subsystem-bounds" {
                    span class="bounds-label" { "bounds" }
                    @for bound in &card.bounds {
                        code class="bound" { (bound.as_str()) }
                    }
                }
            }
        }
    }
}

/// The measured-vs-asserted boundary, shown as two clearly separated lists so a proof and a
/// claim can never be mistaken for one another.
fn boundary_section(measured: &[String], asserted: &[String]) -> Markup {
    html! {
        section class="model-section" id="boundary" {
            h2 { "What the gate proves — and what it does not" }
            p class="meta" {
                "The compositional gate reports a measured-vs-asserted boundary as data. Measured means proved over the platform model's own material; asserted means claimed but not proved, and the two are shown apart."
            }
            div class="boundary-grid" {
                div class="boundary-panel boundary-measured" {
                    h3 { "Measured — proved" }
                    ul {
                        @for item in measured {
                            li { (item.as_str()) }
                        }
                    }
                }
                div class="boundary-panel boundary-asserted" {
                    h3 { "Not measured — asserted, not proved" }
                    ul {
                        @for item in asserted {
                            li { (item.as_str()) }
                        }
                    }
                }
            }
        }
    }
}

fn process_section(root: &OkfRoot, allocated: &HashMap<String, String>) -> Markup {
    let (layers, has_cycle) = process_layers(root, allocated);
    html! {
        section class="model-section" id="process" {
            h2 { "Process" }
            p class="meta" {
                "The business process as an ordered flow, read from the activity steps and their "
                code { "include" } " / " code { "triggers" } " edges."
            }
            p class="fidelity-note" {
                "The link from a platform activity to a specific element inside a subsystem is the typed "
                code { "crossModelEdges" } " declared on the subsystem reference (implementedBy / satisfiedBy), resolved at its pinned revision; the activity's "
                code { "allocatedTo" } " attribute still names the role for this flow view."
            }
            @if layers.is_empty() {
                p { "This model has no activity steps to lay out." }
            } @else {
                (process_flow_markup(&layers))
                @if has_cycle {
                    p class="meta" {
                        "Some steps could not be ordered: the include / triggers edges contain a cycle, so those steps are shown at the end."
                    }
                }
            }
        }
    }
}

fn process_flow_markup(layers: &[ProcessLayer]) -> Markup {
    html! {
        div class="process-flow" {
            @for (index, layer) in layers.iter().enumerate() {
                @if index > 0 {
                    span class="flow-arrow" aria-hidden="true" { "→" }
                }
                div class="flow-layer" {
                    @if layer.steps.len() > 1 {
                        span class="flow-parallel-label" { "parallel" }
                    }
                    @for step in &layer.steps {
                        div class="flow-step"
                           data-step-id=(step.id.as_str())
                           data-role=(step.allocated_to.as_deref().unwrap_or("")) {
                            div class="step-name" { (step.name.as_str()) }
                            @if let Some(role) = &step.allocated_to {
                                span class="role-chip" { (role.as_str()) }
                            }
                            @for req in &step.satisfies {
                                span class="step-satisfies" {
                                    "satisfies " (req.req_id.as_str()) " · " (req.name.as_str())
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Read the raw OKF document behind a commit, preserving the fields the typed model drops
/// (an activity's attributes).
fn raw_model_json(state: &ApiState, commit: &Commit) -> Result<Value, ApiError> {
    let bytes = state
        .store
        .blob(&commit.okf_hash)
        .map_err(map_store_error)?
        .ok_or_else(|| ApiError::internal("stored model is missing"))?;
    serde_json::from_slice(&bytes).map_err(|e| {
        eprintln!("stored model is not valid JSON: {}", e);
        ApiError::internal("stored model could not be read")
    })
}

/// Activity id -> allocatedTo value, read from the raw document's activities array.
fn allocated_to_map(raw: &Value) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let Some(activities) = raw.get("activities").and_then(Value::as_array) else {
        return map;
    };
    for activity in activities {
        let Some(id) = activity.get("id").and_then(Value::as_str) else {
            continue;
        };
        let Some(attributes) = activity.get("attributes").and_then(Value::as_array) else {
            continue;
        };
        for attr in attributes {
            if attr.get("name").and_then(Value::as_str) == Some("allocatedTo") {
                if let Some(role) = attr.get("type").and_then(Value::as_str) {
                    if !role.is_empty() {
                        map.insert(id.to_string(), role.to_string());
                    }
                }
            }
        }
    }
    map
}

/// The measured and asserted lists from the compositional gate's evidence, verbatim.
fn boundary_lists(evidence: &Value) -> (Vec<String>, Vec<String>) {
    fn strings(evidence: &Value, key: &str) -> Vec<String> {
        evidence
            .get("boundary")
            .and_then(|b| b.get(key))
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default()
    }
    (strings(evidence, "measured"), strings(evidence, "asserted"))
}

/// Lay the activity steps out in order, grouping parallel branches, from the graph's activity
/// nodes and their include / triggers edges. Returns the layers and whether a cycle kept some
/// steps from being ordered.
fn process_layers(
    root: &OkfRoot,
    allocated: &HashMap<String, String>,
) -> (Vec<ProcessLayer>, bool) {
    let Some(graph) = &root.graph else {
        return (Vec::new(), false);
    };

    let activity_nodes: Vec<_> = graph
        .nodes
        .iter()
        .filter(|node| node.kind == "activity")
        .collect();
    let ids: Vec<String> = activity_nodes.iter().map(|node| node.id.clone()).collect();
    let id_set: HashSet<String> = ids.iter().cloned().collect();

    let mut edges: Vec<(String, String)> = Vec::new();
    for edge in &graph.edges {
        if (edge.kind == "include" || edge.kind == "triggers")
            && id_set.contains(&edge.source)
            && id_set.contains(&edge.target)
        {
            edges.push((edge.source.clone(), edge.target.clone()));
        }
    }

    let mut indegree: HashMap<String, usize> = HashMap::new();
    for id in &ids {
        indegree.insert(id.clone(), 0);
    }
    let mut outgoing: HashMap<String, Vec<String>> = HashMap::new();
    let mut predecessors: HashMap<String, Vec<String>> = HashMap::new();
    for (source, target) in &edges {
        *indegree.entry(target.clone()).or_insert(0) += 1;
        outgoing
            .entry(source.clone())
            .or_default()
            .push(target.clone());
        predecessors
            .entry(target.clone())
            .or_default()
            .push(source.clone());
    }

    let mut queue: VecDeque<String> = ids
        .iter()
        .filter(|id| indegree.get(*id).copied().unwrap_or(0) == 0)
        .cloned()
        .collect();
    let mut order: Vec<String> = Vec::new();
    while let Some(node) = queue.pop_front() {
        order.push(node.clone());
        if let Some(targets) = outgoing.get(&node) {
            for target in targets {
                let degree = indegree
                    .get_mut(target)
                    .expect("target has an in-degree entry");
                *degree -= 1;
                if *degree == 0 {
                    queue.push_back(target.clone());
                }
            }
        }
    }

    let has_cycle = order.len() < ids.len();

    let mut layer: HashMap<String, usize> = HashMap::new();
    for node in &order {
        let mut depth = 0usize;
        if let Some(preds) = predecessors.get(node) {
            for pred in preds {
                if let Some(pred_depth) = layer.get(pred) {
                    depth = depth.max(pred_depth + 1);
                }
            }
        }
        layer.insert(node.clone(), depth);
    }

    let max_layer = layer.values().copied().max().unwrap_or(0);
    let mut ordered = order.clone();
    for id in &ids {
        if !layer.contains_key(id) {
            ordered.push(id.clone());
            layer.insert(id.clone(), max_layer + 1);
        }
    }

    let req_by_id: HashMap<&str, &Requirement> = root
        .requirements
        .iter()
        .map(|req| (req.id.as_str(), req))
        .collect();
    let mut satisfies: HashMap<String, Vec<RequirementRef>> = HashMap::new();
    for edge in &graph.edges {
        if edge.kind == "dependency" && edge.label == "Satisfy" {
            if let Some(req) = req_by_id.get(edge.target.as_str()) {
                satisfies
                    .entry(edge.source.clone())
                    .or_default()
                    .push(RequirementRef {
                        req_id: req.req_id.clone(),
                        name: req.name.clone(),
                    });
            }
        }
    }

    let name_by_id: HashMap<&str, String> = activity_nodes
        .iter()
        .map(|node| (node.id.as_str(), node.name.clone()))
        .collect();

    let mut layers: Vec<ProcessLayer> = Vec::new();
    let mut current_layer: Option<usize> = None;
    let mut current_steps: Vec<ProcessStep> = Vec::new();
    for id in ordered {
        let depth = layer[&id];
        if current_layer != Some(depth) {
            if !current_steps.is_empty() {
                layers.push(ProcessLayer {
                    steps: std::mem::take(&mut current_steps),
                });
            }
            current_layer = Some(depth);
        }
        let name = name_by_id
            .get(id.as_str())
            .cloned()
            .unwrap_or_else(|| id.clone());
        let allocated_to = allocated.get(&id).cloned();
        let satisfied = satisfies.get(&id).cloned().unwrap_or_default();
        current_steps.push(ProcessStep {
            id: id.clone(),
            name,
            allocated_to,
            satisfies: satisfied,
        });
    }
    if !current_steps.is_empty() {
        layers.push(ProcessLayer {
            steps: current_steps,
        });
    }

    (layers, has_cycle)
}

fn short_hash(hash: &str) -> &str {
    hash.get(..8).unwrap_or(hash)
}
