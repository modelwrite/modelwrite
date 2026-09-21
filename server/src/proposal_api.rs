// SPDX-License-Identifier: AGPL-3.0-or-later
//! The agent acceptance flow: the path by which an agent's proposal becomes a change.
//!
//! An agent proposes; a human commits. The proposal is persisted here, addressable by id,
//! with the agent's identity, the task it answered and the review artifact a human reads. An
//! agent still cannot write: recording a proposal needs the review role, but accepting or
//! refusing one needs the write role, which an agent token can never hold - so an agent is
//! refused at the acceptance endpoint exactly as on every other write path.
//!
//! A human with write accepts a proposal by id. The acceptance re-runs the import the
//! proposal resolves (loading the retained artifact the proposal names), and the resulting
//! commit's provenance names BOTH parties: the agent that proposed and the human that
//! accepted, plus the accepted items. A refusal is recorded too, because a refused proposal
//! is as much a decision as an accepted one.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use agent::losses::entry_identity;

use crate::api::{
    commit_core, commit_json, load_model, map_store_error, resolve_author, validate_name,
    verify_actor, ApiState, CommitCore, CommitFailure,
};
use crate::audit::{PROPOSAL_RECORD, PROPOSAL_REFUSED};
use crate::auth::{Identity, Permission};
use crate::binding_api::{import_core, ImportCore, ImportOutcome};
use crate::error::ApiError;
use crate::store::{
    now_seconds, AcceptanceProvenance, AuditEntry, Commit, ProposalAcceptance, ProposalRecord,
    Store,
};

/// The proposal record as the JSON a reader sees. The review artifact is returned as the
/// object the agent produced (not a nested JSON string), and the decision names the human
/// who made it, never a name from a request body.
pub fn proposal_json(record: &ProposalRecord) -> Value {
    json!({
        "id": record.id,
        "project": record.project,
        "agent": record.agent,
        "taskGoal": record.task_goal,
        "artifactHash": record.artifact_hash,
        "binding": record.binding,
        "reviewArtifact": serde_json::from_str::<Value>(&record.review_artifact).unwrap_or(Value::Null),
        "createdAt": record.created_at,
        "decision": record.decision.map(|d| d.as_str()),
        "decidedBy": record.decided_by,
        "acceptedItems": record.accepted_items,
        "commitHash": record.commit_hash,
        "decidedAt": record.decided_at,
    })
}

/// Extract the retained artifact hash and binding selector from a review artifact whose
/// material is a loss report. The review artifact is stored verbatim, so these two fields are
/// derived from the SAME JSON the human reads - a single source of truth, never a claim the
/// caller could contradict in a separate field. Any other material yields None for both.
fn extract_loss_material(artifact: &Value) -> (Option<String>, Option<String>) {
    let loss = artifact
        .get("task")
        .and_then(|t| t.get("material"))
        .and_then(|m| m.get("LossReport"));
    let artifact_hash = loss
        .and_then(|l| l.get("artifact_hash"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let binding = loss.and_then(|l| l.get("binding")).and_then(|b| {
        let id = b.get("id").and_then(|v| v.as_str()).unwrap_or("");
        let version = b.get("version").and_then(|v| v.as_str()).unwrap_or("");
        if id.is_empty() || version.is_empty() {
            None
        } else {
            Some(format!("{}@{}", id, version))
        }
    });
    (artifact_hash, binding)
}

/// POST /projects/:project/proposals - record an agent's proposal.
///
/// The body IS the review artifact. The agent identity is the verified subject (never a name
/// from the body), the task goal and the retained artifact/binding are derived from the
/// artifact's material, and the artifact itself is stored verbatim so the later decision is
/// recorded against exactly what the agent wrote. Recording needs the review role, which an
/// agent token may hold; it grants no write.
pub async fn record_proposal(
    identity: Identity,
    State(state): State<ApiState>,
    Path(project): Path<String>,
    Json(body): Json<Value>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    if !identity.may(Permission::Review) {
        return Err(ApiError::forbidden("review permission required"));
    }
    if !identity.may_reach(&project) {
        return Err(ApiError::forbidden("project not in scope"));
    }
    if state
        .store_for(&identity)
        .project(&project)
        .map_err(map_store_error)?
        .is_none()
    {
        return Err(ApiError::not_found(format!("project {}", project)));
    }
    let task_goal = body
        .get("task")
        .and_then(|t| t.get("goal"))
        .and_then(|g| g.as_str())
        .filter(|g| !g.trim().is_empty())
        .ok_or_else(|| ApiError::bad_request("the proposal must carry a task goal"))?
        .to_string();
    let (artifact_hash, binding) = extract_loss_material(&body);
    let review_artifact = body.to_string();
    let id = crate::store::proposal_id(&project, &identity.subject, &review_artifact);
    let audit = AuditEntry {
        id: 0,
        project: project.clone(),
        at: now_seconds(),
        actor: identity.subject.clone(),
        mechanism: state.auth.mechanism().to_string(),
        authorizer: state.auth.authorizer().unwrap_or("").to_string(),
        action: PROPOSAL_RECORD.to_string(),
        subject: id.clone(),
        detail: task_goal.clone(),
    };
    state
        .store_for(&identity)
        .record_proposal(
            &project,
            &identity.subject,
            &task_goal,
            artifact_hash.as_deref(),
            binding.as_deref(),
            &review_artifact,
            Some(&audit),
        )
        .map_err(map_store_error)?;
    let record = state
        .store_for(&identity)
        .proposal(&project, &id)
        .map_err(map_store_error)?
        .ok_or_else(|| ApiError::internal("the proposal could not be read after recording"))?;
    Ok((StatusCode::CREATED, Json(proposal_json(&record))))
}

/// GET /projects/:project/proposals/:id - fetch a proposal by id.
pub async fn get_proposal(
    identity: Identity,
    State(state): State<ApiState>,
    Path((project, id)): Path<(String, String)>,
) -> Result<Json<Value>, ApiError> {
    if !identity.may(Permission::Read) {
        return Err(ApiError::forbidden("read permission required"));
    }
    if !identity.may_reach(&project) {
        return Err(ApiError::forbidden("project not in scope"));
    }
    let record = state
        .store_for(&identity)
        .proposal(&project, &id)
        .map_err(map_store_error)?
        .ok_or_else(|| ApiError::not_found(format!("proposal {}", id)))?;
    Ok(Json(proposal_json(&record)))
}

/// GET /projects/:project/proposals - the project's proposals, newest first, each carrying its
/// decision (when decided) so the proposals page can render the agent, the deciding human and
/// the decision without a second fetch.
pub async fn list_proposals(
    identity: Identity,
    State(state): State<ApiState>,
    Path(project): Path<String>,
) -> Result<Json<Value>, ApiError> {
    if !identity.may(Permission::Read) {
        return Err(ApiError::forbidden("read permission required"));
    }
    if !identity.may_reach(&project) {
        return Err(ApiError::forbidden("project not in scope"));
    }
    let records = state
        .store_for(&identity)
        .list_proposals(&project)
        .map_err(map_store_error)?;
    Ok(Json(Value::Array(
        records.iter().map(proposal_json).collect(),
    )))
}

#[derive(Deserialize)]
pub struct AcceptProposal {
    #[serde(default, rename = "acceptedItems")]
    pub accepted_items: Vec<String>,
    pub branch: String,
    #[serde(default)]
    pub author: String,
    pub message: String,
    pub holder: Option<String>,
}

/// POST /projects/:project/proposals/:id/accept - a human with write accepts a proposal.
///
/// The identity is the verified token's subject, never a name from the request; the author
/// and holder are reconciled against that identity exactly as on every other write path. The
/// acceptance goes through the SAME import core and commit path as a direct import, so the
/// enforcement lives in the commit path, not here: an agent token is refused by the write
/// check above, and a lying acceptance is refused by the store's commit transaction.
pub async fn accept_proposal(
    identity: Identity,
    State(state): State<ApiState>,
    Path((project, id)): Path<(String, String)>,
    Json(body): Json<AcceptProposal>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    if !identity.may(Permission::Write) {
        return Err(ApiError::forbidden("write permission required"));
    }
    if !identity.may_reach(&project) {
        return Err(ApiError::forbidden("project not in scope"));
    }
    let author = resolve_author(&state.auth, &identity, &body.author)?;
    verify_actor(&state.auth, &identity, body.holder.as_deref())?;
    validate_name("branch name", &body.branch)?;

    let commit = accept_proposal_core(
        state.store_for(&identity).as_ref(),
        &project,
        &id,
        &identity.subject,
        state.auth.mechanism(),
        state.auth.authorizer().unwrap_or(""),
        &author,
        &body.branch,
        &body.message,
        body.holder.as_deref(),
        &body.accepted_items,
    )?;
    let record = state
        .store_for(&identity)
        .proposal(&project, &id)
        .map_err(map_store_error)?
        .ok_or_else(|| ApiError::internal("the proposal could not be read after accepting"))?;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "commit": commit_json(&commit), "proposal": proposal_json(&record) })),
    ))
}

/// The ONE implementation of an acceptance: load the proposal, check it is undecided, load
/// the retained artifact it resolves, and re-run the import through the SAME import core as a
/// direct import - with the acceptance attached, so the commit's provenance names the agent
/// and the human. The store's commit transaction is the authority that re-checks the proposal
/// is undecided and recorded by the named agent; the check here is only the fast refusal.
#[allow(clippy::too_many_arguments)]
pub(crate) fn accept_proposal_core(
    store: &dyn Store,
    project: &str,
    proposal_id: &str,
    actor: &str,
    mechanism: &str,
    authorizer: &str,
    author: &str,
    branch: &str,
    message: &str,
    holder: Option<&str>,
    accepted_items: &[String],
) -> Result<Commit, ApiError> {
    let record = store
        .proposal(project, proposal_id)
        .map_err(map_store_error)?
        .ok_or_else(|| ApiError::not_found(format!("proposal {}", proposal_id)))?;
    if record.decision.is_some() {
        return Err(ApiError::conflict(format!(
            "proposal {} has already been decided",
            proposal_id
        )));
    }
    if record.binding.is_some() {
        // A loss-report proposal is accepted by re-running its import. The proposal's binding
        // selector is the discriminator: a loss report always carries one, a model-change
        // proposal never does.
        if accepted_items.is_empty() {
            return Err(ApiError::bad_request(
                "at least one accepted item is required",
            ));
        }
        let artifact_hash = record.artifact_hash.as_deref().ok_or_else(|| {
            ApiError::bad_request(
                "the proposal is not a loss report and cannot be accepted as an import",
            )
        })?;
        let binding = record.binding.as_deref().ok_or_else(|| {
            ApiError::bad_request("the proposal has no binding and cannot be accepted as an import")
        })?;
        let artifact = store
            .blob(artifact_hash)
            .map_err(map_store_error)?
            .ok_or_else(|| {
                ApiError::not_found(format!(
                    "retained artifact {} for proposal {}",
                    artifact_hash, proposal_id
                ))
            })?;
        let acceptance = ProposalAcceptance {
            proposal_id: proposal_id.to_string(),
            agent: record.agent.clone(),
            accepted_by: actor.to_string(),
        };
        return match import_core(
            store,
            project,
            &ImportCore {
                binding,
                branch,
                author,
                message,
                artifact: &artifact,
                accept_losses: accepted_items,
                holder,
                actor,
                mechanism,
                authorizer,
                acceptance: Some(acceptance),
            },
        )? {
            ImportOutcome::Committed { commit, .. } => Ok(*commit),
            ImportOutcome::Blocking { unaccepted, .. } => {
                let subjects: Vec<String> = unaccepted.iter().map(entry_identity).collect();
                Err(ApiError::unprocessable(
                    "the acceptance is incomplete: blocking losses not accepted",
                    subjects,
                ))
            }
        };
    }

    // A model-change proposal is accepted by committing its candidate document through the
    // shared commit core, with Accepted provenance naming both parties.
    accept_model_change_core(
        store,
        &record,
        project,
        actor,
        mechanism,
        authorizer,
        author,
        branch,
        message,
        holder,
        accepted_items,
    )
}

/// The subjects of a model-change proposal's model-changing proposals, read from the SAME
/// review artifact JSON the human read. These are the accepted-item identities the provenance
/// records.
fn model_change_subjects(artifact: &Value) -> Vec<String> {
    artifact
        .get("proposals")
        .and_then(|p| p.as_array())
        .map(|proposals| {
            proposals
                .iter()
                .filter(|p| {
                    matches!(
                        p.get("action").and_then(|a| a.as_str()),
                        Some("EditElement") | Some("DraftText")
                    )
                })
                .filter_map(|p| p.get("subject").and_then(|s| s.as_str()).map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

/// Accept a model-change proposal: extract the candidate document from the proposal's own
/// review artifact (the SAME document the human read), and commit it through the shared commit
/// core with [CommitProvenance::Accepted]. The candidate passes the SAME validator every
/// commit passes, so a proposal that would produce an invalid document is refused with the
/// validator's own errors, exactly as the editor shows them.
#[allow(clippy::too_many_arguments)]
fn accept_model_change_core(
    store: &dyn Store,
    record: &ProposalRecord,
    project: &str,
    actor: &str,
    mechanism: &str,
    authorizer: &str,
    author: &str,
    branch: &str,
    message: &str,
    holder: Option<&str>,
    accepted_items: &[String],
) -> Result<Commit, ApiError> {
    let artifact: Value = serde_json::from_str(&record.review_artifact).map_err(|e| {
        ApiError::bad_request(format!(
            "the proposal's review artifact is not readable: {}",
            e
        ))
    })?;
    let document = artifact
        .get("task")
        .and_then(|t| t.get("material"))
        .and_then(|m| m.get("Document"))
        .cloned()
        .ok_or_else(|| {
            ApiError::bad_request(
                "the proposal carries no candidate document and cannot be accepted as a model change",
            )
        })?;
    let candidate: okf::types::OkfRoot = serde_json::from_value(document).map_err(|e| {
        ApiError::bad_request(format!(
            "the proposal's candidate document is malformed: {}",
            e
        ))
    })?;

    // The accepted items are the model-changing subjects. An empty list accepts every one;
    // a non-empty list must name exactly the full set, so the provenance never claims less
    // than the commit applies.
    let subjects = model_change_subjects(&artifact);
    let accepted: Vec<String> = if accepted_items.is_empty() {
        subjects.clone()
    } else {
        let mut proposed = subjects.clone();
        proposed.sort();
        let mut named = accepted_items.to_vec();
        named.sort();
        named.dedup();
        if proposed != named {
            return Err(ApiError::bad_request(
                "acceptedItems must name every proposed change (or be empty to accept all)",
            ));
        }
        accepted_items.to_vec()
    };

    let bytes = serde_json::to_vec(&candidate).map_err(|e| {
        eprintln!("candidate model could not be serialised: {}", e);
        ApiError::internal("the candidate model could not be prepared")
    })?;
    let now = now_seconds();
    let tip = store
        .branch_tip(project, branch)
        .map_err(map_store_error)?
        .ok_or_else(|| ApiError::not_found(format!("branch {} has no commits", branch)))?;
    let reference = load_model(store, project, &tip).map_err(map_store_error)?;
    let provenance = AcceptanceProvenance {
        proposal_id: record.id.clone(),
        agent: record.agent.clone(),
        accepted_by: actor.to_string(),
        accepted_items: accepted,
    };
    commit_core(
        store,
        &CommitCore {
            project,
            branch,
            author,
            message,
            actor,
            mechanism,
            authorizer,
            candidate: &candidate,
            bytes: &bytes,
            import: None,
            acceptance: Some(&provenance),
            holder: holder.unwrap_or(""),
            now,
            tip: Some(&tip),
            reference: Some(&reference),
        },
    )
    .map_err(|failure| match failure {
        CommitFailure::Invalid { errors } => {
            ApiError::unprocessable("the model failed validation", errors)
        }
        CommitFailure::Store(error) => map_store_error(error),
    })
}

#[derive(Deserialize)]
pub struct RefuseProposal {
    #[serde(default)]
    pub reason: String,
}

/// POST /projects/:project/proposals/:id/refuse - a human with write refuses a proposal.
///
/// A refusal is a decision, recorded exactly as an acceptance is: the proposal is marked
/// refused with the verified subject, and an audit entry records the decision. The proposal
/// must exist and be undecided.
pub async fn refuse_proposal(
    identity: Identity,
    State(state): State<ApiState>,
    Path((project, id)): Path<(String, String)>,
    Json(body): Json<RefuseProposal>,
) -> Result<Json<Value>, ApiError> {
    if !identity.may(Permission::Write) {
        return Err(ApiError::forbidden("write permission required"));
    }
    if !identity.may_reach(&project) {
        return Err(ApiError::forbidden("project not in scope"));
    }
    let detail = if body.reason.trim().is_empty() {
        "proposal refused".to_string()
    } else {
        format!("proposal refused: {}", body.reason)
    };
    let audit = AuditEntry {
        id: 0,
        project: project.clone(),
        at: now_seconds(),
        actor: identity.subject.clone(),
        mechanism: state.auth.mechanism().to_string(),
        authorizer: state.auth.authorizer().unwrap_or("").to_string(),
        action: PROPOSAL_REFUSED.to_string(),
        subject: id.clone(),
        detail,
    };
    state
        .store_for(&identity)
        .refuse_proposal(&project, &id, &identity.subject, Some(&audit))
        .map_err(map_store_error)?;
    let record = state
        .store_for(&identity)
        .proposal(&project, &id)
        .map_err(map_store_error)?
        .ok_or_else(|| ApiError::internal("the proposal could not be read after refusing"))?;
    Ok(Json(proposal_json(&record)))
}
