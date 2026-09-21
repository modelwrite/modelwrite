// SPDX-License-Identifier: AGPL-3.0-or-later
//! The assist loop: a natural-language request about a model becomes a PROPOSAL, never a
//! commit. A reasoner turns the request into concrete model changes (add or edit an element,
//! draft a requirement); the changes are applied to the current model to produce a candidate
//! document; the candidate is embedded in a [agent::ReviewArtifact] (the existing shape a
//! human reads everywhere) and the proposal is recorded, addressable by id, for the human to
//! accept or refuse. Nothing here commits: the only path from a proposal to a model change is
//! a human with Write accepting it through the shared commit core.
//!
//! The reasoning is a trait ([ModelReasoner]) with three implementations: a SCRIPTED reasoner
//! (deterministic, no network, for tests) and TWO live reasoners behind configuration - an
//! OpenAI-compatible chat-completions endpoint the organisation runs itself
//! (MW_ASSIST_BASE_URL + MW_ASSIST_MODEL, with an optional MW_ASSIST_API_KEY bearer) and the
//! Anthropic Messages API (MW_ANTHROPIC_API_KEY). Both read their credentials from the
//! environment ONLY - never logged, never in a request, never committed. The local fleet wins
//! when both are set; if neither is set the endpoint reports "no live reasoner configured"
//! and points at the scripted mode. No model provider is embedded: the endpoints and keys
//! arrive from the deployment's environment.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use agent::{
    AgentTask, Confidence, Material, Proposal, ProposalCheck, ProposedAction, ReviewArtifact,
};
use okf::types::{Element, GraphNode, OkfRoot, Requirement};

use crate::api::{load_model, map_store_error, validate_element_name, validate_name, ApiState};
use crate::audit::PROPOSAL_RECORD;
use crate::auth::{Identity, Permission};
use crate::error::ApiError;
use crate::proposal_api::proposal_json;
use crate::store::{now_seconds, proposal_id, AuditEntry, ProposalRecord, Store};

/// The name the scripted reasoner records as its agent.
pub const SCRIPTED_AGENT: &str = "scripted-assist";
/// The name the live reasoner records as its agent.
pub const LIVE_AGENT: &str = "mw-assist";
/// The Anthropic model the live reasoner asks, overridable by MW_ANTHROPIC_MODEL. A model
/// NAME is not a provider: the endpoint and the key come from the deployment's environment.
const DEFAULT_MODEL: &str = "claude-sonnet-4-20250514";
/// The Messages API endpoint. The key is sent as a header on this request only, never stored.
const ANTHROPIC_ENDPOINT: &str = "https://api.anthropic.com/v1/messages";
/// The token ceiling on the live OpenAI-compatible reasoner's answer. A proposal is one to
/// three strict-JSON changes (a few hundred tokens), so bounding the completion here keeps
/// generation time proportional to the WORK, not to a generous ceiling the model could fill.
/// Reasoning is disabled in the request (see [call_openai]) so this cap is spent on the JSON
/// answer itself, never on a thinking pass.
const OPENAI_MAX_TOKENS: u64 = 512;

/// One concrete model change a reasoner proposed: the action (always model-changing) plus the
/// element or requirement it carries. This is the STRICT JSON shape the live model must
/// answer with, and the value the scripted reasoner returns directly. The action maps onto
/// [ProposedAction]; an answer whose action, confidence or payload does not deserialize into
/// these types is REFUSED, never trusted.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelChange {
    pub action: ProposedAction,
    #[serde(default)]
    pub element: Option<Element>,
    #[serde(default)]
    pub requirement: Option<Requirement>,
    #[serde(default)]
    pub rationale: String,
    pub confidence: Confidence,
}

impl ModelChange {
    /// The stable identity of this change: the id of the element or requirement it carries.
    /// This is what the acceptance names as an accepted item.
    pub fn subject(&self) -> String {
        match self.action {
            ProposedAction::EditElement => self
                .element
                .as_ref()
                .map(|e| e.id.clone())
                .unwrap_or_default(),
            ProposedAction::DraftText => self
                .requirement
                .as_ref()
                .map(|r| r.id.clone())
                .unwrap_or_default(),
            _ => String::new(),
        }
    }

    /// Whether this change is structurally valid: its action changes the model, and it carries
    /// exactly the payload that action needs with a non-empty id in the existing id shape.
    fn validate(&self) -> Result<(), String> {
        match &self.action {
            ProposedAction::EditElement => {
                let element = self
                    .element
                    .as_ref()
                    .ok_or_else(|| "EditElement change has no element".to_string())?;
                if self.requirement.is_some() {
                    return Err("EditElement change also carries a requirement".to_string());
                }
                validate_proposed_id(&element.id)?;
            }
            ProposedAction::DraftText => {
                let requirement = self
                    .requirement
                    .as_ref()
                    .ok_or_else(|| "DraftText change has no requirement".to_string())?;
                if self.element.is_some() {
                    return Err("DraftText change also carries an element".to_string());
                }
                validate_proposed_id(&requirement.id)?;
            }
            other => {
                return Err(format!(
                    "action {:?} is not a model change (only EditElement and DraftText change the model)",
                    other
                ));
            }
        }
        Ok(())
    }
}

/// The id-shape rules a proposed id must satisfy. Non-empty is checked after trimming (an id
/// of only whitespace is empty), then the existing element-id shape rules from the create/edit
/// path apply. Style is deliberately NOT enforced here: an opaque id is still a valid id, so
/// the prompt makes the readable kebab-case form the default rather than this refusing a valid
/// answer over taste.
fn validate_proposed_id(id: &str) -> Result<(), String> {
    if id.trim().is_empty() {
        return Err("a proposed element or requirement has an empty id".to_string());
    }
    validate_element_name(id).map_err(|e| e.message)
}

/// Validate a whole proposal: every change must be structurally valid AND the ids it carries
/// must be unique within the proposal, so two changes cannot silently overwrite each other
/// when the candidate document is applied.
fn validate_changes(changes: &[ModelChange]) -> Result<(), String> {
    let mut seen = std::collections::HashSet::new();
    for change in changes {
        change.validate()?;
        // validate() already guarantees a non-empty id for every model-changing action, so
        // subject() is exactly the id to uniqueness-check.
        let id = change.subject();
        if !seen.insert(id.clone()) {
            return Err(format!(
                "the proposal repeats the id {}; ids must be unique within the document",
                id
            ));
        }
    }
    Ok(())
}

/// Turns a natural-language request into concrete model changes. A trait so the contract is
/// exercised by a scripted implementation in tests before any live model is connected. Nothing
/// here can commit: the only thing a reasoner may return is changes for a human to read.
pub trait ModelReasoner: Send + Sync {
    /// The identity recorded as the proposing agent.
    fn agent(&self) -> &str;
    /// Produce the concrete changes for the request, or refuse with a typed error. A live
    /// implementation may block on a network call; callers run it off the async executor via
    /// spawn_blocking.
    fn propose(
        &self,
        project: &str,
        request: &str,
        current: &OkfRoot,
    ) -> Result<Vec<ModelChange>, ApiError>;
}

/// A reasoner with its changes written down in advance. It exists so tests can pin the exact
/// artifact a deterministic script produces, without a network call.
#[derive(Debug, Clone)]
pub struct ScriptedReasoner {
    agent: String,
    changes: Vec<ModelChange>,
}

impl ScriptedReasoner {
    pub fn new(agent: impl Into<String>, changes: Vec<ModelChange>) -> Self {
        ScriptedReasoner {
            agent: agent.into(),
            changes,
        }
    }

    /// The deterministic script the scripted mode runs: the example from the brief - a heater
    /// block with a water inlet port and a requirement that it heats to 95C. Fresh ids that do
    /// not collide with the seed model, so the acceptance produces a valid commit.
    pub fn example() -> Self {
        ScriptedReasoner::new(
            SCRIPTED_AGENT,
            vec![
                ModelChange {
                    action: ProposedAction::EditElement,
                    element: Some(Element {
                        id: "heater-block".to_string(),
                        name: "Heater Block".to_string(),
                        kind: "block".to_string(),
                        stereotypes: Vec::new(),
                        attributes: vec![okf::types::Attribute {
                            name: "waterInletPort".to_string(),
                            attr_type: String::new(),
                            aggregation: String::new(),
                            default: String::new(),
                        }],
                        documentation: "a heater block with a water inlet port".to_string(),
                    }),
                    requirement: None,
                    rationale: "add a heater block with a water inlet port".to_string(),
                    confidence: Confidence::High,
                },
                ModelChange {
                    action: ProposedAction::DraftText,
                    element: None,
                    requirement: Some(Requirement {
                        id: "req-heat".to_string(),
                        name: "Heat to 95C".to_string(),
                        kind: "requirement".to_string(),
                        stereotypes: Vec::new(),
                        attributes: Vec::new(),
                        documentation: String::new(),
                        req_id: "REQ-HEAT".to_string(),
                        req_text: "the heater block heats water to 95C".to_string(),
                    }),
                    rationale: "the request requires heating to 95C".to_string(),
                    confidence: Confidence::High,
                },
            ],
        )
    }
}

impl ModelReasoner for ScriptedReasoner {
    fn agent(&self) -> &str {
        &self.agent
    }

    fn propose(
        &self,
        _project: &str,
        request: &str,
        _current: &OkfRoot,
    ) -> Result<Vec<ModelChange>, ApiError> {
        if request.trim().is_empty() {
            return Err(ApiError::bad_request(
                "the assist request must not be empty",
            ));
        }
        Ok(self.changes.clone())
    }
}

/// A live reasoner behind configuration: it asks an Anthropic model, using the API key read
/// from MW_ANTHROPIC_API_KEY (env only), to answer with the strict JSON shape of
/// [ModelChange]. The answer is validated against the proposal types and REFUSED, naming what
/// was malformed, rather than trusted.
pub struct LiveReasoner {
    api_key: String,
    model: String,
}

impl LiveReasoner {
    pub fn new(api_key: impl Into<String>) -> Self {
        let model = std::env::var("MW_ANTHROPIC_MODEL")
            .ok()
            .filter(|m| !m.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_MODEL.to_string());
        LiveReasoner {
            api_key: api_key.into(),
            model,
        }
    }
}

impl ModelReasoner for LiveReasoner {
    fn agent(&self) -> &str {
        LIVE_AGENT
    }

    fn propose(
        &self,
        project: &str,
        request: &str,
        current: &OkfRoot,
    ) -> Result<Vec<ModelChange>, ApiError> {
        if request.trim().is_empty() {
            return Err(ApiError::bad_request(
                "the assist request must not be empty",
            ));
        }
        let system = system_prompt();
        let user = user_prompt(project, request, current)?;
        let raw = call_anthropic(&self.api_key, &self.model, &system, &user)?;
        parse_and_validate(&raw)
    }
}

/// A live reasoner behind the organisation's OWN model fleet: an OpenAI-compatible
/// chat-completions endpoint (MW_ASSIST_BASE_URL, with the model named by MW_ASSIST_MODEL and
/// an optional MW_ASSIST_API_KEY bearer). It asks the SAME strict-JSON prompt and validates
/// the answer the SAME way as the Anthropic path - a malformed answer is REFUSED, naming
/// what was malformed, rather than trusted.
pub struct OpenAiReasoner {
    base_url: String,
    model: String,
    api_key: Option<String>,
}

impl OpenAiReasoner {
    pub fn new(
        base_url: impl Into<String>,
        model: impl Into<String>,
        api_key: Option<String>,
    ) -> Self {
        OpenAiReasoner {
            base_url: base_url.into(),
            model: model.into(),
            api_key,
        }
    }
}

impl ModelReasoner for OpenAiReasoner {
    fn agent(&self) -> &str {
        LIVE_AGENT
    }

    fn propose(
        &self,
        project: &str,
        request: &str,
        current: &OkfRoot,
    ) -> Result<Vec<ModelChange>, ApiError> {
        if request.trim().is_empty() {
            return Err(ApiError::bad_request(
                "the assist request must not be empty",
            ));
        }
        let system = system_prompt();
        let user = user_prompt(project, request, current)?;
        let raw = call_openai(
            &self.base_url,
            &self.model,
            self.api_key.as_deref(),
            &system,
            &user,
        )?;
        // The answer may still arrive wrapped in prose or a markdown fence despite
        // response_format; locate the JSON object defensively, then validate it strictly.
        parse_and_validate(&extract_json_object(&raw))
    }
}

/// The strict contract the live model is asked to honour. Every demand is spelled out so a
/// violation is the model's answer being malformed, never a silent surprise.
fn system_prompt() -> String {
    r#"You are a model editor for the modelwrite platform. Answer with JSON ONLY - no prose, no markdown fences, no commentary. Your answer must be a JSON object of exactly this shape:

{"changes": [
  {"action": "EditElement", "element": {"id": "heater-block", "name": "...", "kind": "block", "stereotypes": [], "attributes": [], "documentation": "..."}, "requirement": null, "rationale": "...", "confidence": "High"}
]}

Rules, all mandatory:
- action is exactly "EditElement" (add or change an element) or "DraftText" (add or change a requirement). An EditElement change carries "element" and a null "requirement"; a DraftText change carries "requirement" and a null "element".
- confidence is exactly "High", "Medium" or "Low".
- Only add or modify elements in the project named in the current model. Never invent requirements, elements or constraints that the request does not ask for.
- Give every new element or requirement a SHORT, HUMAN-READABLE id in kebab-case, for example "heater-block" or "heat-requirement". A reader should not have to decode a timestamp to find an element you added, so never invent an opaque id in the corpus's internal scheme. Each id must be UNIQUE within the document: it must not repeat an id already in the current model nor an id in another change you propose. A requirement must have a non-empty reqId.
- If you cannot honour the request with a model change, return {"changes": []}.
Return only the JSON object."#
        .to_string()
}

/// The COMPACT model inventory a reasoner reads instead of the full document. Proposing a
/// change needs the NAME/KIND inventory - what elements and requirements already exist, so the
/// model proposes a sensible, readable id for a new one - and the request; it does not need the
/// full opaque ids (the bulk of the old prompt), documentation, attributes, provenance or the
/// graph edge list. The opaque ids are deliberately omitted: a readable kebab-case id cannot
/// collide with the corpus's internal scheme, and [validate_changes] still rejects a duplicate
/// id within the proposal. Nothing else is serialised here: no ids, no documentation bodies, no
/// attributes, no edge endpoints, no provenance.
fn model_inventory(current: &OkfRoot) -> Value {
    let structure: Vec<Value> = current
        .structure
        .iter()
        .map(|e| {
            json!({
                "name": e.name.clone(),
                "kind": e.kind.clone(),
            })
        })
        .collect();
    let interfaces: Vec<Value> = current
        .interfaces
        .iter()
        .map(|e| json!({ "name": e.name.clone(), "kind": e.kind.clone() }))
        .collect();
    let signals: Vec<Value> = current
        .signals
        .iter()
        .map(|e| json!({ "name": e.name.clone(), "kind": e.kind.clone() }))
        .collect();
    let requirements: Vec<Value> = current
        .requirements
        .iter()
        .map(|r| {
            json!({
                "name": r.name.clone(),
                "kind": r.kind.clone(),
            })
        })
        .collect();
    // The model must know edges EXIST, but proposing a change needs the name/kind inventory,
    // not every edge endpoint. Report the counts from the graph itself (the truth of "edges
    // exist"); fall back to the summary when a graph is absent.
    let (graph_nodes, graph_edges) = match current.graph.as_ref() {
        Some(graph) => (graph.nodes.len(), graph.edges.len()),
        None => (
            current.summary.graph_nodes as usize,
            current.summary.graph_edges as usize,
        ),
    };
    json!({
        "structure": structure,
        "interfaces": interfaces,
        "signals": signals,
        "requirements": requirements,
        "graphNodes": graph_nodes,
        "graphEdges": graph_edges,
    })
}

fn user_prompt(project: &str, request: &str, current: &OkfRoot) -> Result<String, ApiError> {
    let inventory = serde_json::to_string(&model_inventory(current)).map_err(|e| {
        eprintln!("current model inventory could not be serialised: {}", e);
        ApiError::internal("the current model could not be prepared")
    })?;
    Ok(format!(
        "Project: {}\nThe current model inventory (names and kinds only; no ids, documentation, attributes or edge lists) is:\n{}\n\nThe request is: {}\n\nPropose the changes that honour this request.",
        project, inventory, request
    ))
}

/// Call the Anthropic Messages API and return the assistant's text. The key travels only as a
/// header on this request; it is never logged or returned.
fn call_anthropic(key: &str, model: &str, system: &str, user: &str) -> Result<String, ApiError> {
    let response = ureq::post(ANTHROPIC_ENDPOINT)
        .set("x-api-key", key)
        .set("anthropic-version", "2023-06-01")
        .set("content-type", "application/json")
        .send_json(json!({
            "model": model,
            "max_tokens": 8192,
            "system": system,
            "messages": [{"role": "user", "content": user}]
        }))
        .map_err(|e| {
            // ureq's error Display names the transport or HTTP status, never the key.
            ApiError::service_unavailable(format!("the live reasoner request failed: {}", e))
        })?;
    let body: Value = response.into_json().map_err(|e| {
        ApiError::service_unavailable(format!("the live reasoner answered non-JSON: {}", e))
    })?;
    let text = body
        .get("content")
        .and_then(|c| c.as_array())
        .and_then(|blocks| {
            blocks
                .iter()
                .find(|b| b.get("type").and_then(|t| t.as_str()) == Some("text"))
        })
        .and_then(|b| b.get("text"))
        .and_then(|t| t.as_str())
        .ok_or_else(|| ApiError::bad_request("the live reasoner answer had no text content"))?;
    Ok(text.to_string())
}

/// Call an OpenAI-compatible chat-completions endpoint and return the assistant's text. The
/// optional bearer key travels only as a header on this request; it is never logged or
/// returned. The endpoint is the organisation's own fleet, so no provider is embedded: the
/// base URL, model and key come from the deployment's environment.
fn call_openai(
    base_url: &str,
    model: &str,
    api_key: Option<&str>,
    system: &str,
    user: &str,
) -> Result<String, ApiError> {
    let endpoint = format!("{}/chat/completions", base_url.trim_end_matches('/'));
    let request = ureq::post(&endpoint).set("content-type", "application/json");
    let request = match api_key.filter(|k| !k.is_empty()) {
        Some(key) => request.set("authorization", &format!("Bearer {}", key)),
        None => request,
    };
    let response = request
        .send_json(json!({
            "model": model,
            "messages": [
                {"role": "system", "content": system},
                {"role": "user", "content": user}
            ],
            "temperature": 0,
            "max_tokens": OPENAI_MAX_TOKENS,
            // The fleet's qwen3 models reason before answering by default; that thinking pass
            // would consume the max_tokens budget and leave the strict-JSON answer truncated
            // (or empty). Turn it off so the cap bounds the JSON answer, not the thinking.
            "chat_template_kwargs": {"enable_thinking": false},
            "response_format": {"type": "json_object"}
        }))
        .map_err(|e| {
            // ureq's error Display names the transport or HTTP status, never the key.
            ApiError::service_unavailable(format!("the live reasoner request failed: {}", e))
        })?;
    let body: Value = response.into_json().map_err(|e| {
        ApiError::service_unavailable(format!("the live reasoner answered non-JSON: {}", e))
    })?;
    let text = body
        .get("choices")
        .and_then(|c| c.as_array())
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("message"))
        .and_then(|message| message.get("content"))
        .and_then(|content| content.as_str())
        .ok_or_else(|| ApiError::bad_request("the live reasoner answer had no content"))?;
    Ok(text.to_string())
}

/// Locate the JSON object in the model's answer, tolerating a markdown fence or a little
/// surrounding prose. The STRICT validation still happens downstream in [parse_and_validate];
/// this only finds the object to hand it there, never loosens it. When no object can be
/// located the raw text is returned so [parse_and_validate] refuses it with the parse error
/// naming what was malformed.
fn extract_json_object(raw: &str) -> String {
    let trimmed = raw.trim();
    if serde_json::from_str::<Value>(trimmed).is_ok() {
        return trimmed.to_string();
    }
    let un_fenced = trimmed
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();
    if let Some(start) = un_fenced.find('{') {
        if let Some(end) = un_fenced.rfind('}') {
            if end >= start {
                let candidate = &un_fenced[start..=end];
                if serde_json::from_str::<Value>(candidate).is_ok() {
                    return candidate.to_string();
                }
            }
        }
    }
    trimmed.to_string()
}

/// Parse the live model's answer into concrete changes, validating it against the proposal
/// types. A malformed answer is REFUSED with a clear error naming what was malformed, never
/// trusted.
fn parse_and_validate(raw: &str) -> Result<Vec<ModelChange>, ApiError> {
    let trimmed = raw.trim();
    let body: Value = serde_json::from_str(trimmed).map_err(|e| {
        ApiError::bad_request(format!("the live reasoner answered malformed JSON: {}", e))
    })?;
    let changes: Vec<ModelChange> = serde_json::from_value(
        body.get("changes")
            .cloned()
            .unwrap_or_else(|| Value::Array(Vec::new())),
    )
    .map_err(|e| {
        ApiError::bad_request(format!(
            "the live reasoner answered a malformed proposal: {}",
            e
        ))
    })?;
    validate_changes(&changes).map_err(ApiError::bad_request)?;
    Ok(changes)
}

/// Apply the changes to a clone of the current model, producing the candidate document. A
/// change whose id already exists replaces that element or requirement in place; a new id
/// appends to the matching section (block, interface, signal, requirement) with a matching
/// graph node. The summary is RECOMPUTED from the sections afterwards rather than
/// incremented field by field, so the candidate can never carry a stale count inherited from
/// a base whose summary was already wrong.
pub fn apply_changes(current: &OkfRoot, changes: &[ModelChange]) -> OkfRoot {
    let mut candidate = current.clone();
    for change in changes {
        match change.action {
            ProposedAction::EditElement => {
                if let Some(element) = change.element.as_ref() {
                    place_element(&mut candidate, element);
                }
            }
            ProposedAction::DraftText => {
                if let Some(requirement) = change.requirement.as_ref() {
                    place_requirement(&mut candidate, requirement);
                }
            }
            _ => {}
        }
    }
    okf::summary::recompute(&mut candidate);
    candidate
}

fn replace_element(list: &mut [Element], element: &Element) -> bool {
    if let Some(existing) = list.iter_mut().find(|e| e.id == element.id) {
        *existing = element.clone();
        true
    } else {
        false
    }
}

fn place_element(candidate: &mut OkfRoot, element: &Element) {
    if replace_element(&mut candidate.structure, element)
        || replace_element(&mut candidate.interfaces, element)
        || replace_element(&mut candidate.signals, element)
    {
        return;
    }
    let node_kind = if element.kind.is_empty() {
        "block".to_string()
    } else {
        element.kind.clone()
    };
    match element.kind.as_str() {
        "interface" => candidate.interfaces.push(element.clone()),
        "signal" => candidate.signals.push(element.clone()),
        _ => candidate.structure.push(element.clone()),
    }
    // A new element is always attached to the graph as a node, so it is at least visible and
    // countable. It may still be ISOLATED (no edges) when the change has no relationship to
    // express; the proposal's gate check reports that, never buries it.
    if let Some(graph) = candidate.graph.as_mut() {
        graph.nodes.push(GraphNode {
            id: element.id.clone(),
            kind: node_kind,
            name: element.name.clone(),
            stereotypes: element.stereotypes.clone(),
        });
    }
}

fn place_requirement(candidate: &mut OkfRoot, requirement: &Requirement) {
    if let Some(existing) = candidate
        .requirements
        .iter_mut()
        .find(|r| r.id == requirement.id)
    {
        *existing = requirement.clone();
        return;
    }
    candidate.requirements.push(requirement.clone());
    if let Some(graph) = candidate.graph.as_mut() {
        graph.nodes.push(GraphNode {
            id: requirement.id.clone(),
            kind: "requirement".to_string(),
            name: requirement.name.clone(),
            stereotypes: requirement.stereotypes.clone(),
        });
    }
}

/// Build the review artifact from the changes, embedding the candidate document as the
/// material so the human reads the SAME document that a later acceptance applies - a single
/// source of truth, never a claim a caller could contradict. The artifact also carries the
/// validator-and-gate result over that candidate, so a human sees BEFORE accepting that the
/// result would leave an isolated node, a disconnected component or a coverage regression.
pub fn build_review_artifact(
    agent: &str,
    request: &str,
    current: &OkfRoot,
    changes: &[ModelChange],
) -> ReviewArtifact {
    let candidate = apply_changes(current, changes);
    let check = proposal_check(current, &candidate);
    let proposals: Vec<Proposal> = changes
        .iter()
        .map(|change| Proposal {
            subject: change.subject(),
            action: change.action.clone(),
            rationale: change.rationale.clone(),
            confidence: change.confidence,
        })
        .collect();
    ReviewArtifact {
        task: AgentTask {
            goal: request.to_string(),
            material: Material::Document(candidate),
            constraints: vec![
                "only add or modify elements in the stated project".to_string(),
                "never invent requirements outside the request".to_string(),
            ],
        },
        proposals,
        agent: agent.to_string(),
        rationale_summary: format!("{} proposed {} change(s)", agent, changes.len()),
        gaps: Vec::new(),
        check: Some(check),
    }
}

/// Run the SAME validator and gate the proposal loop must run, and distill the parts a human
/// must see before accepting: validation errors (a candidate that cannot commit), isolated
/// nodes, disconnected components, and the coverage delta. The round-trip diff is deliberately
/// excluded from `passed`: a proposal is SUPPOSED to differ from the current model, so
/// "passed" here means "the candidate would not fail the gate's integration checks".
fn proposal_check(current: &OkfRoot, candidate: &OkfRoot) -> ProposalCheck {
    let outcome = gate::run(current, candidate, false);
    let evidence = &outcome.evidence;

    let validation_errors = string_array(evidence, &["validationErrors"]);
    let isolated_nodes = string_array(evidence, &["integration", "isolated"]);
    let component_count = evidence
        .get("integration")
        .and_then(|i| i.get("componentCount"))
        .and_then(|c| c.as_u64())
        .unwrap_or(0) as usize;
    let uncovered_requirements = string_array(evidence, &["coverage", "uncovered"]);
    let prior_uncovered_requirements = if current.graph.is_some() {
        graph::requirement_coverage(current).uncovered
    } else {
        Vec::new()
    };

    let passed = validation_errors.is_empty() && isolated_nodes.is_empty() && component_count == 1;

    ProposalCheck {
        passed,
        validation_errors,
        isolated_nodes,
        component_count,
        uncovered_requirements,
        prior_uncovered_requirements,
    }
}

/// Read a string array from the gate evidence at the given JSON path, returning an empty list
/// when the key is absent or not an array of strings. The evidence schema is stable, but a
/// reader must never panic on a shape it does not recognise.
fn string_array(value: &serde_json::Value, path: &[&str]) -> Vec<String> {
    let mut cursor = value;
    for key in path {
        cursor = match cursor.get(key) {
            Some(next) => next,
            None => return Vec::new(),
        };
    }
    cursor
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

/// Which live backend answers a request: the organisation's OWN OpenAI-compatible fleet
/// (local) or Anthropic's Messages API. Local wins when both are configured.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiveBackend {
    /// An OpenAI-compatible chat-completions endpoint (MW_ASSIST_BASE_URL).
    OpenAi,
    /// The Anthropic Messages API (MW_ANTHROPIC_API_KEY).
    Anthropic,
}

/// The reasoner the next assist request will use, for the panel's status line. Mirrors
/// [select_reasoner_from] exactly: the two must never disagree about what a request will do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReasonerStatus {
    /// A live reasoner is configured; [LiveBackend] names which one.
    Live(LiveBackend),
    /// The deterministic scripted reasoner is forced (MW_ASSIST_REASONER=scripted).
    Scripted,
    /// No live reasoner and no scripted override: the panel must say so plainly.
    NotConfigured,
    /// MW_ASSIST_REASONER names something other than "scripted".
    UnknownMode(String),
}

/// Read an environment variable, treating a blank value as unset.
fn env_nonempty(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

/// Read the reasoner status from the process environment WITHOUT running a reasoner. The
/// assist panel calls this to say what a request WOULD do before the caller submits, so a
/// deployment without a backend is told up front rather than only after a 503.
pub fn reasoner_status() -> ReasonerStatus {
    let base_url = env_nonempty("MW_ASSIST_BASE_URL");
    let key = env_nonempty("MW_ANTHROPIC_API_KEY");
    let mode = env_nonempty("MW_ASSIST_REASONER");
    reasoner_status_from(base_url.as_deref(), key.as_deref(), mode.as_deref())
}

/// The pure status decision, factored out of [reasoner_status] so a unit test can exercise
/// every branch without touching the process environment. [select_reasoner_from] delegates to
/// this so the status and the actual selection cannot drift apart. The local fleet wins: a
/// base URL is checked before an Anthropic key.
fn reasoner_status_from(
    base_url: Option<&str>,
    key: Option<&str>,
    mode: Option<&str>,
) -> ReasonerStatus {
    match mode {
        Some("scripted") => ReasonerStatus::Scripted,
        Some(other) => ReasonerStatus::UnknownMode(other.to_string()),
        None => match base_url {
            Some(_) => ReasonerStatus::Live(LiveBackend::OpenAi),
            None => match key {
                Some(_) => ReasonerStatus::Live(LiveBackend::Anthropic),
                None => ReasonerStatus::NotConfigured,
            },
        },
    }
}

/// Select the reasoner from the environment, so a test can force the deterministic script and
/// a deployment can supply a backend. Every credential is read from the environment ONLY -
/// never from a request, never logged, never committed.
fn select_reasoner() -> Result<Box<dyn ModelReasoner>, ApiError> {
    let base_url = env_nonempty("MW_ASSIST_BASE_URL");
    let model = env_nonempty("MW_ASSIST_MODEL");
    let api_key = env_nonempty("MW_ASSIST_API_KEY");
    let key = env_nonempty("MW_ANTHROPIC_API_KEY");
    let mode = env_nonempty("MW_ASSIST_REASONER");
    select_reasoner_from(
        base_url.as_deref(),
        model.as_deref(),
        api_key.as_deref(),
        key.as_deref(),
        mode.as_deref(),
    )
}

/// The pure selection decision, factored out of [select_reasoner] so a unit test can exercise
/// every branch without touching the process environment.
fn select_reasoner_from(
    base_url: Option<&str>,
    model: Option<&str>,
    api_key: Option<&str>,
    key: Option<&str>,
    mode: Option<&str>,
) -> Result<Box<dyn ModelReasoner>, ApiError> {
    match reasoner_status_from(base_url, key, mode) {
        ReasonerStatus::Scripted => Ok(Box::new(ScriptedReasoner::example())),
        ReasonerStatus::Live(LiveBackend::OpenAi) => Ok(Box::new(OpenAiReasoner::new(
            base_url.unwrap_or("").to_string(),
            model.unwrap_or("").to_string(),
            api_key.map(|k| k.to_string()),
        ))),
        ReasonerStatus::Live(LiveBackend::Anthropic) => {
            Ok(Box::new(LiveReasoner::new(key.unwrap_or("").to_string())))
        }
        ReasonerStatus::NotConfigured => Err(ApiError::service_unavailable(
            "no live reasoner configured: set MW_ASSIST_BASE_URL (with MW_ASSIST_MODEL) or MW_ANTHROPIC_API_KEY, or MW_ASSIST_REASONER=scripted for the deterministic test mode",
        )),
        ReasonerStatus::UnknownMode(other) => Err(ApiError::bad_request(format!(
            "unknown assist reasoner mode {:?}; expected 'scripted'",
            other
        ))),
    }
}

#[derive(Deserialize)]
pub struct AssistRequest {
    pub request: String,
    pub branch: String,
}

/// What an assist request produced: the addressable proposal id, the review artifact a human
/// reads, and the persisted proposal record. Nothing here commits: the artifact's candidate
/// document is the SAME document a later acceptance applies.
pub struct AssistOutcome {
    pub id: String,
    pub artifact: ReviewArtifact,
    pub record: ProposalRecord,
}

/// The ONE assist sequence, shared by the JSON handler and the workbench panel so the two
/// callers can never disagree about what a request did. Permission decisions live in each
/// caller; this core does the work: load the branch's model, run the selected reasoner,
/// build and persist the review artifact (a PROPOSAL, never a commit), and read it back.
pub async fn assist_core(
    store: &dyn Store,
    project: &str,
    branch: &str,
    request: &str,
    actor: &str,
    mechanism: &str,
    authorizer: &str,
) -> Result<AssistOutcome, ApiError> {
    validate_name("branch name", branch)?;
    if request.trim().is_empty() {
        return Err(ApiError::bad_request(
            "the assist request must not be empty",
        ));
    }
    if store.project(project).map_err(map_store_error)?.is_none() {
        return Err(ApiError::not_found(format!("project {}", project)));
    }
    let tip = store
        .branch_tip(project, branch)
        .map_err(map_store_error)?
        .ok_or_else(|| ApiError::not_found(format!("branch {} has no commits", branch)))?;
    let current = load_model(store, project, &tip).map_err(map_store_error)?;

    let reasoner = select_reasoner()?;
    let agent = reasoner.agent().to_string();
    // A live reasoner blocks on a network call; run it off the async worker so one slow
    // model answer cannot stall the request executor. The scripted reasoner returns
    // immediately and pays only the cost of a move into the blocking pool. The current model
    // is cloned for the task so the core keeps its copy for the artifact below.
    let project_for_reasoner = project.to_string();
    let request_for_reasoner = request.to_string();
    let current_for_reasoner = current.clone();
    let changes = tokio::task::spawn_blocking(move || {
        reasoner.propose(
            &project_for_reasoner,
            &request_for_reasoner,
            &current_for_reasoner,
        )
    })
    .await
    .map_err(|e| {
        eprintln!("assist reasoner task failed: {}", e);
        ApiError::internal("the reasoner could not run")
    })??;
    validate_changes(&changes).map_err(ApiError::bad_request)?;

    let artifact = build_review_artifact(&agent, request, &current, &changes);
    let artifact_json = serde_json::to_string(&artifact).map_err(|e| {
        eprintln!("review artifact could not be serialised: {}", e);
        ApiError::internal("the review artifact could not be prepared")
    })?;
    let id = proposal_id(project, &agent, &artifact_json);
    let audit = AuditEntry {
        id: 0,
        project: project.to_string(),
        at: now_seconds(),
        actor: actor.to_string(),
        mechanism: mechanism.to_string(),
        authorizer: authorizer.to_string(),
        action: PROPOSAL_RECORD.to_string(),
        subject: id.clone(),
        detail: request.to_string(),
    };
    store
        .record_proposal(
            project,
            &agent,
            request,
            None,
            None,
            &artifact_json,
            Some(&audit),
        )
        .map_err(map_store_error)?;
    let record = store
        .proposal(project, &id)
        .map_err(map_store_error)?
        .ok_or_else(|| ApiError::internal("the proposal could not be read after recording"))?;
    Ok(AssistOutcome {
        id,
        artifact,
        record,
    })
}

/// POST /projects/:project/assist - run a reasoner and return its proposal as a review
/// artifact. This records a PROPOSAL, never a commit: the caller's identity is the verified
/// subject (the audit actor), the agent is the reasoner's own identity, and the review
/// artifact is stored verbatim so a later acceptance is recorded against exactly what was
/// proposed. A caller needs Write (the assist panel serves people who are about to edit).
pub async fn assist(
    identity: Identity,
    State(state): State<ApiState>,
    Path(project): Path<String>,
    Json(body): Json<AssistRequest>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    if !identity.may(Permission::Write) {
        return Err(ApiError::forbidden("write permission required"));
    }
    if !identity.may_reach(&project) {
        return Err(ApiError::forbidden("project not in scope"));
    }
    let outcome = assist_core(
        state.store_for(&identity).as_ref(),
        &project,
        &body.branch,
        &body.request,
        &identity.subject,
        state.auth.mechanism(),
        state.auth.authorizer().unwrap_or(""),
    )
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "id": outcome.id,
            "reviewArtifact": serde_json::to_value(&outcome.artifact).unwrap_or(Value::Null),
            "proposal": proposal_json(&outcome.record),
        })),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    use okf::types::{Attribute, Graph, GraphEdge, Provenance, Summary};

    fn empty_model() -> OkfRoot {
        OkfRoot {
            okf: "1.0".to_string(),
            project: "coffee".to_string(),
            exported_at: String::new(),
            summary: Summary::default(),
            structure: Vec::new(),
            interfaces: Vec::new(),
            signals: Vec::new(),
            requirements: Vec::new(),
            state_machine: None,
            activities: Vec::new(),
            graph: None,
            provenance: None,
            references: Vec::new(),
        }
    }

    #[test]
    fn the_inventory_lists_names_and_kinds_and_omits_ids() {
        let mut current = empty_model();
        current.structure.push(Element {
            id: "_2026x_1_12a70364_1789357107731_106889_3635".to_string(),
            name: "Block A".to_string(),
            kind: "block".to_string(),
            stereotypes: vec!["Block".to_string()],
            attributes: Vec::new(),
            documentation: String::new(),
        });
        current.interfaces.push(Element {
            id: "_2026x_1_12a70364_1789357175665_504215_3638".to_string(),
            name: "Iface A".to_string(),
            kind: "interface".to_string(),
            stereotypes: Vec::new(),
            attributes: Vec::new(),
            documentation: String::new(),
        });
        current.signals.push(Element {
            id: "_2026x_1_12a70364_1789357305852_992852_3641".to_string(),
            name: "Sig A".to_string(),
            kind: "signal".to_string(),
            stereotypes: Vec::new(),
            attributes: Vec::new(),
            documentation: String::new(),
        });
        current.requirements.push(Requirement {
            id: "_2026x_1_12a70364_1789522470201_737876_5617".to_string(),
            name: "Req A".to_string(),
            kind: "requirement".to_string(),
            stereotypes: Vec::new(),
            attributes: Vec::new(),
            documentation: String::new(),
            req_id: "REQ-A".to_string(),
            req_text: String::new(),
        });
        current.graph = Some(Graph {
            nodes: vec![GraphNode {
                id: "block-a".to_string(),
                kind: "block".to_string(),
                name: "Block A".to_string(),
                stereotypes: Vec::new(),
            }],
            edges: vec![GraphEdge {
                source: "block-a".to_string(),
                target: "iface-a".to_string(),
                kind: "part".to_string(),
                label: String::new(),
            }],
        });

        let inventory = serde_json::to_string(&model_inventory(&current)).unwrap();
        for name in ["Block A", "Iface A", "Sig A", "Req A"] {
            assert!(
                inventory.contains(name),
                "the inventory must carry the name {}, got: {}",
                name,
                inventory
            );
        }
        for kind in ["block", "interface", "signal", "requirement"] {
            assert!(
                inventory.contains(&format!("\"kind\":\"{}\"", kind)),
                "the inventory must carry the kind {}, got: {}",
                kind,
                inventory
            );
        }
        // The opaque corpus ids were the bulk of the old prompt; the model reasons about names
        // and kinds, so the ids are NOT sent. The validator still enforces id uniqueness within
        // the proposal.
        for id in [
            "_2026x_1_12a70364_1789357107731_106889_3635",
            "_2026x_1_12a70364_1789357175665_504215_3638",
            "_2026x_1_12a70364_1789357305852_992852_3641",
            "_2026x_1_12a70364_1789522470201_737876_5617",
            "block-a",
            "iface-a",
            "sig-a",
            "req-a",
        ] {
            assert!(
                !inventory.contains(id),
                "the inventory must omit the id {}, got: {}",
                id,
                inventory
            );
        }
        assert!(
            inventory.contains("\"graphNodes\":1"),
            "the graph node count must be carried"
        );
        assert!(
            inventory.contains("\"graphEdges\":1"),
            "the graph edge count must be carried"
        );
    }

    #[test]
    fn the_inventory_omits_documentation_attributes_edges_provenance_and_ids() {
        let mut current = empty_model();
        current.structure.push(Element {
            id: "_2026x_1_12a70364_1789357107731_106889_3635".to_string(),
            name: "Block A".to_string(),
            kind: "block".to_string(),
            stereotypes: Vec::new(),
            attributes: vec![Attribute {
                name: "secretAttr".to_string(),
                attr_type: "secretType".to_string(),
                aggregation: "secretAgg".to_string(),
                default: "secretDefault".to_string(),
            }],
            documentation: "SECRET-DOC-BLOCK".to_string(),
        });
        current.requirements.push(Requirement {
            id: "_2026x_1_12a70364_1789522470201_737876_5617".to_string(),
            name: "Req A".to_string(),
            kind: "requirement".to_string(),
            stereotypes: Vec::new(),
            attributes: Vec::new(),
            documentation: "SECRET-DOC-REQ".to_string(),
            req_id: "REQ-A".to_string(),
            req_text: "SECRET-REQ-TEXT".to_string(),
        });
        current.graph = Some(Graph {
            nodes: vec![GraphNode {
                id: "block-a".to_string(),
                kind: "block".to_string(),
                name: "Block A".to_string(),
                stereotypes: Vec::new(),
            }],
            edges: vec![GraphEdge {
                source: "block-a".to_string(),
                target: "req-a".to_string(),
                kind: "part".to_string(),
                label: "SECRET-EDGE-LABEL".to_string(),
            }],
        });
        current.provenance = Some(Provenance {
            source_tool: "SECRET-TOOL".to_string(),
            exporter: "SECRET-EXPORTER".to_string(),
            exporter_version: "SECRET-VER".to_string(),
        });

        let inventory = serde_json::to_string(&model_inventory(&current)).unwrap();
        for omitted in [
            "SECRET-DOC-BLOCK",
            "SECRET-DOC-REQ",
            "SECRET-REQ-TEXT",
            "secretAttr",
            "secretType",
            "secretAgg",
            "secretDefault",
            "SECRET-EDGE-LABEL",
            "SECRET-TOOL",
            "SECRET-EXPORTER",
            "SECRET-VER",
            // The opaque ids and the reqId are not sent: names and kinds only.
            "_2026x_1_12a70364_1789357107731_106889_3635",
            "_2026x_1_12a70364_1789522470201_737876_5617",
            "REQ-A",
            "block-a",
            "req-a",
        ] {
            assert!(
                !inventory.contains(omitted),
                "the inventory must omit {}, got: {}",
                omitted,
                inventory
            );
        }
        assert!(inventory.contains("Block A"), "names must still be present");
        assert!(inventory.contains("Req A"));
    }

    #[test]
    fn the_coffee_machine_inventory_is_names_and_kinds_and_well_under_two_thousand_tokens() {
        let full = test_support::load_okf_expected();
        let current: OkfRoot =
            serde_json::from_str(&full).expect("the corpus fixture must deserialise");
        let inventory = serde_json::to_string(&model_inventory(&current)).unwrap();

        // Every element and requirement NAME must be present so the reasoner can reason about
        // what already exists; the opaque corpus ids (the old prompt's bulk) are NOT sent. The
        // validator still enforces id uniqueness within the proposal, and a readable kebab-case
        // id cannot collide with the corpus's opaque internal scheme.
        for element in current
            .structure
            .iter()
            .chain(current.interfaces.iter())
            .chain(current.signals.iter())
        {
            assert!(
                inventory.contains(&element.name),
                "the inventory must carry element name {}",
                element.name
            );
            assert!(
                !inventory.contains(&element.id),
                "the inventory must omit the opaque element id {}",
                element.id
            );
        }
        for requirement in &current.requirements {
            assert!(
                inventory.contains(&requirement.name),
                "the inventory must carry requirement name {}",
                requirement.name
            );
            assert!(
                !inventory.contains(&requirement.id),
                "the inventory must omit the opaque requirement id {}",
                requirement.id
            );
        }

        // Nothing else: the full document's keys for documentation bodies, attributes,
        // requirement text, edge lists, provenance, state machines, activities, references,
        // export metadata and the okf marker must not appear in the inventory.
        for needle in [
            "\"documentation\"",
            "\"attributes\"",
            "\"reqText\"",
            "\"edges\"",
            "\"provenance\"",
            "\"stateMachine\"",
            "\"activities\"",
            "\"references\"",
            "\"exportedAt\"",
            "\"okf\"",
        ] {
            assert!(
                !inventory.contains(needle),
                "the inventory must omit full-model key {}, got: {}",
                needle,
                inventory
            );
        }

        // Compact: the names-and-kinds inventory is ~3.7k chars (~850 fleet tokens) versus the
        // old id-laden inventory (~8.6k chars, ~4.9k fleet tokens) and the ~64k-char full
        // document. Bound it well under the 2,000-token target and far below the full document.
        assert!(
            inventory.len() < 4_500,
            "the inventory must be well under 2,000 fleet tokens, got {} chars",
            inventory.len()
        );
        assert!(
            inventory.len() < full.len() / 10,
            "the inventory must be much smaller than the full document ({} vs {} chars)",
            inventory.len(),
            full.len()
        );
    }

    #[test]
    fn a_multi_change_proposal_fits_inside_the_answer_cap() {
        assert_eq!(
            OPENAI_MAX_TOKENS, 512,
            "the cap must be 512, not the old 2048"
        );

        // A one-to-three change proposal is the whole output. Serialise a realistic
        // three-change proposal (two element adds, one requirement) the way the model answers
        // and prove it is a small fraction of the cap: the local tokenizer is ~4 characters per
        // token, so 512 tokens is roughly 2,000 characters.
        let three = json!({ "changes": [
            { "action": "EditElement", "element": { "id": "heater-block", "name": "Heater Block", "kind": "block", "stereotypes": [], "attributes": [], "documentation": "a heater block with a water inlet port" }, "requirement": null, "rationale": "add a heater block", "confidence": "High" },
            { "action": "EditElement", "element": { "id": "water-pump", "name": "Water Pump", "kind": "block", "stereotypes": [], "attributes": [], "documentation": "a water pump" }, "requirement": null, "rationale": "add a water pump", "confidence": "High" },
            { "action": "DraftText", "requirement": { "id": "heat-requirement", "name": "Heat to 95C", "kind": "requirement", "stereotypes": [], "attributes": [], "documentation": "", "reqId": "REQ-HEAT", "reqText": "the heater heats water to 95C" }, "element": null, "rationale": "the request requires heating to 95C", "confidence": "High" }
        ]});
        let text = serde_json::to_string(&three).unwrap();
        assert!(
            text.len() < 1_600,
            "a three-change proposal must fit far under the 512-token cap, got {} chars",
            text.len()
        );
    }

    #[test]
    fn no_key_and_no_mode_reports_no_live_reasoner() {
        let err = select_reasoner_from(None, None, None, None, None)
            .err()
            .expect("no key means no live reasoner");
        assert_eq!(err.status, StatusCode::SERVICE_UNAVAILABLE);
        assert!(
            err.message.contains("no live reasoner configured"),
            "message: {}",
            err.message
        );
    }

    #[test]
    fn a_base_url_selects_the_local_fleet_reasoner() {
        let reasoner = select_reasoner_from(
            Some("http://idc-1:8012/v1"),
            Some("qwen3.8-27b-fp8"),
            None,
            None,
            None,
        )
        .expect("a base URL selects the local fleet");
        assert_eq!(reasoner.agent(), LIVE_AGENT);
    }

    #[test]
    fn an_anthropic_key_selects_the_live_reasoner() {
        let reasoner = select_reasoner_from(None, None, None, Some("sk-ant-key"), None)
            .expect("a key selects live");
        assert_eq!(reasoner.agent(), LIVE_AGENT);
    }

    #[test]
    fn scripted_mode_selects_the_scripted_reasoner_without_a_key() {
        let reasoner = select_reasoner_from(None, None, None, None, Some("scripted"))
            .expect("scripted mode needs no key");
        assert_eq!(reasoner.agent(), SCRIPTED_AGENT);
    }

    #[test]
    fn an_unknown_mode_is_refused() {
        let err = select_reasoner_from(None, None, None, Some("k"), Some("bogus"))
            .err()
            .expect("unknown mode");
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
        assert!(err.message.contains("unknown assist reasoner mode"));
    }

    #[test]
    fn a_model_change_validates_its_shape() {
        assert!(ModelChange {
            action: ProposedAction::EditElement,
            element: Some(Element {
                id: "heater".to_string(),
                name: String::new(),
                kind: "block".to_string(),
                stereotypes: Vec::new(),
                attributes: Vec::new(),
                documentation: String::new(),
            }),
            requirement: None,
            rationale: String::new(),
            confidence: Confidence::High,
        }
        .validate()
        .is_ok());

        let err = ModelChange {
            action: ProposedAction::AcceptLoss,
            element: None,
            requirement: None,
            rationale: String::new(),
            confidence: Confidence::High,
        }
        .validate()
        .expect_err("a non-model-changing action must be refused");
        assert!(err.contains("not a model change"), "err: {}", err);
    }

    #[test]
    fn the_prompt_demands_short_human_readable_unique_ids() {
        let prompt = system_prompt();
        for needle in [
            "kebab-case",
            "heater-block",
            "heat-requirement",
            "HUMAN-READABLE",
            "UNIQUE",
        ] {
            assert!(
                prompt.contains(needle),
                "the prompt must demand {}, but it does not: {}",
                needle,
                prompt
            );
        }
    }

    #[test]
    fn a_proposal_repeating_an_id_is_refused() {
        let element = |id: &str| {
            format!(
                r#"{{"action":"EditElement","element":{{"id":"{}","name":"","kind":"block","stereotypes":[],"attributes":[],"documentation":""}},"requirement":null,"rationale":"","confidence":"High"}}"#,
                id
            )
        };

        // Two changes carrying the same id would silently overwrite each other when applied;
        // the proposal must be refused for repeating it.
        let repeated = format!(
            r#"{{"changes":[{},{}]}}"#,
            element("heater-block"),
            element("heater-block")
        );
        let err = parse_and_validate(&repeated).expect_err("a repeated id must be refused");
        assert!(
            err.message.contains("unique"),
            "the refusal must name uniqueness, got: {}",
            err.message
        );

        // Distinct, human-readable ids are accepted.
        let ok = format!(
            r#"{{"changes":[{},{{"action":"DraftText","requirement":{{"id":"heat-requirement","name":"","kind":"requirement","stereotypes":[],"attributes":[],"documentation":"","reqId":"REQ-HEAT","reqText":"heat"}},"element":null,"rationale":"","confidence":"High"}}]}}"#,
            element("heater-block")
        );
        let changes = parse_and_validate(&ok).expect("distinct ids are accepted");
        assert_eq!(changes.len(), 2);
    }

    #[test]
    fn malformed_live_json_is_refused_not_trusted() {
        let err = parse_and_validate("not json").expect_err("non-JSON must be refused");
        assert!(
            err.message.contains("malformed JSON"),
            "message: {}",
            err.message
        );

        // An action that is not a ProposedAction variant must be refused with the parse error.
        let bad_action = r#"{"changes":[{"action":"Edit","element":null,"requirement":null,"rationale":"","confidence":"High"}]}"#;
        let err = parse_and_validate(bad_action).expect_err("bad action must be refused");
        assert!(
            err.message.contains("malformed proposal"),
            "message: {}",
            err.message
        );
    }

    #[test]
    fn extract_json_object_finds_the_object_behind_prose_or_a_fence() {
        // A clean object passes through untouched.
        let clean = r#"{"changes":[]}"#;
        assert_eq!(extract_json_object(clean), clean);

        // A markdown fence is stripped.
        let fenced = "```json\n{\"changes\":[]}\n```";
        assert_eq!(extract_json_object(fenced), "{\"changes\":[]}");

        // Prose around a JSON object is tolerated.
        let prose = "Sure: {\"changes\":[]} done.";
        assert_eq!(extract_json_object(prose), "{\"changes\":[]}");

        // No object anywhere: the raw text is returned for the strict parser to refuse.
        let none = "no json here at all";
        assert_eq!(extract_json_object(none), none);
    }

    #[test]
    fn reasoner_status_reports_every_mode() {
        assert_eq!(
            reasoner_status_from(None, None, None),
            ReasonerStatus::NotConfigured
        );
        assert_eq!(
            reasoner_status_from(None, Some("sk-ant-key"), None),
            ReasonerStatus::Live(LiveBackend::Anthropic)
        );
        assert_eq!(
            reasoner_status_from(Some("http://idc-1:8012/v1"), None, None),
            ReasonerStatus::Live(LiveBackend::OpenAi)
        );
        assert_eq!(
            reasoner_status_from(None, None, Some("scripted")),
            ReasonerStatus::Scripted
        );
        assert_eq!(
            reasoner_status_from(None, Some("k"), Some("bogus")),
            ReasonerStatus::UnknownMode("bogus".to_string())
        );
    }

    #[test]
    fn the_local_fleet_wins_when_both_backends_are_set() {
        assert_eq!(
            reasoner_status_from(Some("http://idc-1:8012/v1"), Some("sk-ant-key"), None),
            ReasonerStatus::Live(LiveBackend::OpenAi)
        );
    }

    #[test]
    fn reasoner_status_agrees_with_the_reasoner_selection() {
        // The panel's status and the endpoint's selection must never disagree.
        assert_eq!(
            reasoner_status_from(None, None, None),
            ReasonerStatus::NotConfigured
        );
        assert!(select_reasoner_from(None, None, None, None, None).is_err());

        assert_eq!(
            reasoner_status_from(None, Some("k"), None),
            ReasonerStatus::Live(LiveBackend::Anthropic)
        );
        assert_eq!(
            select_reasoner_from(None, None, None, Some("k"), None)
                .unwrap()
                .agent(),
            LIVE_AGENT
        );

        assert_eq!(
            reasoner_status_from(Some("http://idc-1:8012/v1"), None, None),
            ReasonerStatus::Live(LiveBackend::OpenAi)
        );
        assert_eq!(
            select_reasoner_from(
                Some("http://idc-1:8012/v1"),
                Some("qwen3.8-27b-fp8"),
                None,
                None,
                None
            )
            .unwrap()
            .agent(),
            LIVE_AGENT
        );

        assert_eq!(
            reasoner_status_from(None, None, Some("scripted")),
            ReasonerStatus::Scripted
        );
        assert_eq!(
            select_reasoner_from(None, None, None, None, Some("scripted"))
                .unwrap()
                .agent(),
            SCRIPTED_AGENT
        );

        assert_eq!(
            reasoner_status_from(None, None, Some("bogus")),
            ReasonerStatus::UnknownMode("bogus".to_string())
        );
        assert!(select_reasoner_from(None, None, None, None, Some("bogus")).is_err());
    }

    #[test]
    fn apply_changes_attaches_a_node_and_recomputes_a_stale_summary() {
        // A base whose summary UNDER-counts its graph: one real node, claimed zero.
        let mut current = empty_model();
        current.graph = Some(Graph {
            nodes: vec![GraphNode {
                id: "root".to_string(),
                kind: "block".to_string(),
                name: "root".to_string(),
                stereotypes: Vec::new(),
            }],
            edges: Vec::new(),
        });
        current.summary = Summary::default();

        let change = ModelChange {
            action: ProposedAction::EditElement,
            element: Some(Element {
                id: "heater-block".to_string(),
                name: "Heater Block".to_string(),
                kind: "block".to_string(),
                stereotypes: Vec::new(),
                attributes: Vec::new(),
                documentation: String::new(),
            }),
            requirement: None,
            rationale: "add a heater".to_string(),
            confidence: Confidence::High,
        };

        let candidate = apply_changes(&current, &[change]);

        // The added element is attached to the graph as a node, visible and countable.
        assert!(candidate
            .graph
            .as_ref()
            .unwrap()
            .nodes
            .iter()
            .any(|n| n.id == "heater-block"));
        // The summary is RECOMPUTED from the sections, never incremented from the stale base.
        assert_eq!(
            candidate.summary.graph_nodes as usize, 2,
            "root + heater-block"
        );
        assert_eq!(candidate.summary.blocks as usize, 1, "one structure block");
        assert_eq!(candidate.summary.graph_edges as usize, 0);
    }
}
