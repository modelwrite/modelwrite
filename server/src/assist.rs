// SPDX-License-Identifier: AGPL-3.0-or-later
//! The assist loop: a natural-language request about a model becomes a PROPOSAL, never a
//! commit. A reasoner turns the request into concrete model changes (add or edit an element,
//! draft a requirement); the changes are applied to the current model to produce a candidate
//! document; the candidate is embedded in a [agent::ReviewArtifact] (the existing shape a
//! human reads everywhere) and the proposal is recorded, addressable by id, for the human to
//! accept or refuse. Nothing here commits: the only path from a proposal to a model change is
//! a human with Write accepting it through the shared commit core.
//!
//! The reasoning is a trait ([ModelReasoner]) with two implementations: a SCRIPTED reasoner
//! (deterministic, no network, for tests) and a LIVE reasoner behind configuration (reads an
//! Anthropic API key from MW_ANTHROPIC_API_KEY - env only, never logged, never in a request,
//! never committed). If the key is absent the endpoint reports "no live reasoner configured"
//! and points at the scripted mode. No model provider is embedded: the key arrives from the
//! deployment's environment.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use agent::{AgentTask, Confidence, Material, Proposal, ProposedAction, ReviewArtifact};
use okf::types::{Element, GraphNode, OkfRoot, Requirement};

use crate::api::{load_model, map_store_error, validate_name, ApiState};
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
    /// exactly the payload that action needs with a non-empty id.
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
                if element.id.trim().is_empty() {
                    return Err(
                        "EditElement change carries an element with an empty id".to_string()
                    );
                }
            }
            ProposedAction::DraftText => {
                let requirement = self
                    .requirement
                    .as_ref()
                    .ok_or_else(|| "DraftText change has no requirement".to_string())?;
                if self.element.is_some() {
                    return Err("DraftText change also carries an element".to_string());
                }
                if requirement.id.trim().is_empty() {
                    return Err(
                        "DraftText change carries a requirement with an empty id".to_string()
                    );
                }
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

/// The strict contract the live model is asked to honour. Every demand is spelled out so a
/// violation is the model's answer being malformed, never a silent surprise.
fn system_prompt() -> String {
    r#"You are a model editor for the modelwrite platform. Answer with JSON ONLY - no prose, no markdown fences, no commentary. Your answer must be a JSON object of exactly this shape:

{"changes": [
  {"action": "EditElement", "element": {"id": "...", "name": "...", "kind": "block", "stereotypes": [], "attributes": [], "documentation": "..."}, "requirement": null, "rationale": "...", "confidence": "High"}
]}

Rules, all mandatory:
- action is exactly "EditElement" (add or change an element) or "DraftText" (add or change a requirement). An EditElement change carries "element" and a null "requirement"; a DraftText change carries "requirement" and a null "element".
- confidence is exactly "High", "Medium" or "Low".
- Only add or modify elements in the project named in the current model. Never invent requirements, elements or constraints that the request does not ask for.
- Give every new element or requirement an id that does not already exist in the current model. A requirement must have a non-empty reqId.
- If you cannot honour the request with a model change, return {"changes": []}.
Return only the JSON object."#
        .to_string()
}

fn user_prompt(project: &str, request: &str, current: &OkfRoot) -> Result<String, ApiError> {
    let model = serde_json::to_value(current).map_err(|e| {
        eprintln!("current model could not be serialised: {}", e);
        ApiError::internal("the current model could not be prepared")
    })?;
    Ok(format!(
        "Project: {}
The current OKF model is:
{}

The request is: {}

Propose the changes that honour this request.",
        project, model, request
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
    for change in &changes {
        change.validate().map_err(ApiError::bad_request)?;
    }
    Ok(changes)
}

/// Apply the changes to a clone of the current model, producing the candidate document. The
/// summary counts are kept in step exactly as the create-element page does, so the document
/// stays self-consistent. A change whose id already exists replaces that element or
/// requirement in place; a new id appends to the matching section (block, interface, signal,
/// requirement) with a matching graph node.
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
        "interface" => {
            candidate.interfaces.push(element.clone());
            candidate.summary.interfaces += 1;
        }
        "signal" => {
            candidate.signals.push(element.clone());
            candidate.summary.signals += 1;
        }
        _ => {
            candidate.structure.push(element.clone());
            candidate.summary.blocks += 1;
        }
    }
    if let Some(graph) = candidate.graph.as_mut() {
        graph.nodes.push(GraphNode {
            id: element.id.clone(),
            kind: node_kind,
            name: element.name.clone(),
            stereotypes: element.stereotypes.clone(),
        });
        candidate.summary.graph_nodes += 1;
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
    candidate.summary.requirements += 1;
    if let Some(graph) = candidate.graph.as_mut() {
        graph.nodes.push(GraphNode {
            id: requirement.id.clone(),
            kind: "requirement".to_string(),
            name: requirement.name.clone(),
            stereotypes: requirement.stereotypes.clone(),
        });
        candidate.summary.graph_nodes += 1;
    }
}

/// Build the review artifact from the changes, embedding the candidate document as the
/// material so the human reads the SAME document that a later acceptance applies - a single
/// source of truth, never a claim a caller could contradict.
pub fn build_review_artifact(
    agent: &str,
    request: &str,
    current: &OkfRoot,
    changes: &[ModelChange],
) -> ReviewArtifact {
    let candidate = apply_changes(current, changes);
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
    }
}

/// The reasoner the next assist request will use, for the panel's status line. Mirrors
/// [select_reasoner_from] exactly: the two must never disagree about what a request will do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReasonerStatus {
    /// A live reasoner is configured (MW_ANTHROPIC_API_KEY set).
    Live,
    /// The deterministic scripted reasoner is forced (MW_ASSIST_REASONER=scripted).
    Scripted,
    /// No live reasoner and no scripted override: the panel must say so plainly.
    NotConfigured,
    /// MW_ASSIST_REASONER names something other than "scripted".
    UnknownMode(String),
}

/// Read the reasoner status from the process environment WITHOUT running a reasoner. The
/// assist panel calls this to say what a request WOULD do before the caller submits, so a
/// deployment without a key is told up front rather than only after a 503.
pub fn reasoner_status() -> ReasonerStatus {
    let key = std::env::var("MW_ANTHROPIC_API_KEY")
        .ok()
        .filter(|k| !k.trim().is_empty());
    let mode = std::env::var("MW_ASSIST_REASONER")
        .ok()
        .filter(|m| !m.trim().is_empty());
    reasoner_status_from(key.as_deref(), mode.as_deref())
}

/// The pure status decision, factored out of [reasoner_status] so a unit test can exercise
/// every branch without touching the process environment. [select_reasoner_from] delegates to
/// this so the status and the actual selection cannot drift apart.
fn reasoner_status_from(key: Option<&str>, mode: Option<&str>) -> ReasonerStatus {
    match mode {
        Some("scripted") => ReasonerStatus::Scripted,
        Some(other) => ReasonerStatus::UnknownMode(other.to_string()),
        None => match key {
            Some(_) => ReasonerStatus::Live,
            None => ReasonerStatus::NotConfigured,
        },
    }
}

/// Select the reasoner from the environment, so a test can force the deterministic script and
/// a deployment can supply a key. The key is read from the environment ONLY - never from a
/// request, never logged, never committed.
fn select_reasoner() -> Result<Box<dyn ModelReasoner>, ApiError> {
    let key = std::env::var("MW_ANTHROPIC_API_KEY")
        .ok()
        .filter(|k| !k.trim().is_empty());
    let mode = std::env::var("MW_ASSIST_REASONER")
        .ok()
        .filter(|m| !m.trim().is_empty());
    select_reasoner_from(key.as_deref(), mode.as_deref())
}

/// The pure selection decision, factored out of [select_reasoner] so a unit test can exercise
/// every branch without touching the process environment.
fn select_reasoner_from(
    key: Option<&str>,
    mode: Option<&str>,
) -> Result<Box<dyn ModelReasoner>, ApiError> {
    match reasoner_status_from(key, mode) {
        ReasonerStatus::Scripted => Ok(Box::new(ScriptedReasoner::example())),
        ReasonerStatus::Live => Ok(Box::new(LiveReasoner::new(key.unwrap_or("").to_string()))),
        ReasonerStatus::NotConfigured => Err(ApiError::service_unavailable(
            "no live reasoner configured: set MW_ANTHROPIC_API_KEY, or MW_ASSIST_REASONER=scripted for the deterministic test mode",
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
    for change in &changes {
        change.validate().map_err(ApiError::bad_request)?;
    }

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
        state.store.as_ref(),
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

    #[test]
    fn no_key_and_no_mode_reports_no_live_reasoner() {
        let err = select_reasoner_from(None, None)
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
    fn a_key_selects_the_live_reasoner() {
        let reasoner = select_reasoner_from(Some("sk-ant-key"), None).expect("a key selects live");
        assert_eq!(reasoner.agent(), LIVE_AGENT);
    }

    #[test]
    fn scripted_mode_selects_the_scripted_reasoner_without_a_key() {
        let reasoner =
            select_reasoner_from(None, Some("scripted")).expect("scripted mode needs no key");
        assert_eq!(reasoner.agent(), SCRIPTED_AGENT);
    }

    #[test]
    fn an_unknown_mode_is_refused() {
        let err = select_reasoner_from(Some("k"), Some("bogus"))
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
    fn reasoner_status_reports_every_mode() {
        assert_eq!(
            reasoner_status_from(None, None),
            ReasonerStatus::NotConfigured
        );
        assert_eq!(
            reasoner_status_from(Some("sk-ant-key"), None),
            ReasonerStatus::Live
        );
        assert_eq!(
            reasoner_status_from(None, Some("scripted")),
            ReasonerStatus::Scripted
        );
        assert_eq!(
            reasoner_status_from(Some("k"), Some("bogus")),
            ReasonerStatus::UnknownMode("bogus".to_string())
        );
    }

    #[test]
    fn reasoner_status_agrees_with_the_reasoner_selection() {
        // The panel's status and the endpoint's selection must never disagree.
        assert_eq!(
            reasoner_status_from(None, None),
            ReasonerStatus::NotConfigured
        );
        assert!(select_reasoner_from(None, None).is_err());

        assert_eq!(reasoner_status_from(Some("k"), None), ReasonerStatus::Live);
        assert_eq!(
            select_reasoner_from(Some("k"), None).unwrap().agent(),
            LIVE_AGENT
        );

        assert_eq!(
            reasoner_status_from(None, Some("scripted")),
            ReasonerStatus::Scripted
        );
        assert_eq!(
            select_reasoner_from(None, Some("scripted"))
                .unwrap()
                .agent(),
            SCRIPTED_AGENT
        );

        assert_eq!(
            reasoner_status_from(None, Some("bogus")),
            ReasonerStatus::UnknownMode("bogus".to_string())
        );
        assert!(select_reasoner_from(None, Some("bogus")).is_err());
    }
}
