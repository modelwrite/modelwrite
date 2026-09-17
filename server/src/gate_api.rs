// SPDX-License-Identifier: AGPL-3.0-or-later
use axum::extract::{Path, State};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::api::{load_model, map_store_error, ApiState};
use crate::auth::{Identity, Permission};
use crate::error::ApiError;
use crate::store::{now_epoch, now_seconds, AuditEntry, GateRun};

#[derive(Deserialize)]
pub struct GateRequest {
    pub reference: String,
    pub candidate: String,
}

pub async fn run_gate(
    identity: Identity,
    State(state): State<ApiState>,
    Path(project): Path<String>,
    Json(body): Json<GateRequest>,
) -> Result<Json<Value>, ApiError> {
    if !identity.may(Permission::Write) {
        return Err(ApiError::forbidden("write permission required"));
    }
    if !identity.may_reach(&project) {
        return Err(ApiError::forbidden("project not in scope"));
    }
    let reference = load_model(&state, &project, &body.reference)?;
    let candidate = load_model(&state, &project, &body.candidate)?;

    let outcome = gate::run(&reference, &candidate, false);
    let branch = state
        .store
        .commit(&project, &body.candidate)
        .map_err(map_store_error)?
        .map(|c| c.branch)
        .unwrap_or_else(|| "main".to_string());

    // The run is recorded first: the store carries the whole evidence payload, so it is
    // the authority, and the file written below is an export of it. Writing first would
    // let a failed insert leave an evidence file no recorded run cites.
    let run = GateRun {
        project: project.clone(),
        branch,
        reference_hash: body.reference.clone(),
        candidate_hash: body.candidate.clone(),
        passed: outcome.passed,
        evidence: outcome.evidence.to_string(),
        created_at: now_epoch(),
    };
    // The audit entry records what was ATTEMPTED, so a FAILED gate is recorded too: the
    // verdict is part of the detail, and the log exists to show what happened, not only
    // what succeeded. The entry rides the same transaction as the run row, so a recorded
    // run always has its record and vice versa.
    let verdict = if outcome.passed { "passed" } else { "failed" };
    let audit = AuditEntry {
        id: 0,
        project: project.clone(),
        at: now_seconds(),
        actor: "unknown".to_string(),
        action: "gate.run".to_string(),
        subject: body.candidate.clone(),
        detail: format!(
            "{}: candidate {} against reference {}",
            verdict, body.candidate, body.reference
        ),
    };
    state
        .store
        .record_gate_run(&run, Some(&audit))
        .map_err(map_store_error)?;

    // The name carries both FULL hashes. Truncating them would keep determinism but lose
    // uniqueness: two commit pairs sharing a prefix would overwrite each other's evidence
    // while both runs stayed recorded, so a run would cite a file it never wrote.
    // The name carries both FULL hashes and no project name: the hashes are globally
    // unique, and keeping an unvalidated string out of a filesystem path removes any
    // chance of a separator escaping the evidence directory.
    let file_name = format!("server-{}-{}.json", body.reference, body.candidate);
    let path = state.evidence_dir.join(file_name);
    gate::write_evidence(&path, &outcome.evidence).map_err(|e| {
        eprintln!("could not write evidence to {}: {}", path.display(), e);
        ApiError::internal("the evidence record could not be exported")
    })?;

    Ok(Json(outcome.evidence))
}

pub async fn list_gate_runs(
    identity: Identity,
    State(state): State<ApiState>,
    Path(project): Path<String>,
) -> Result<Json<Value>, ApiError> {
    if !identity.may(Permission::Review) {
        return Err(ApiError::forbidden("review permission required"));
    }
    if !identity.may_reach(&project) {
        return Err(ApiError::forbidden("project not in scope"));
    }
    let runs = state.store.gate_runs(&project).map_err(map_store_error)?;
    let out: Vec<Value> = runs
        .iter()
        .map(|r| {
            json!({
                "project": r.project,
                "branch": r.branch,
                "referenceHash": r.reference_hash,
                "candidateHash": r.candidate_hash,
                "passed": r.passed,
                "evidence": serde_json::from_str::<Value>(&r.evidence).unwrap_or(Value::Null),
                "createdAt": r.created_at
            })
        })
        .collect();
    Ok(Json(Value::Array(out)))
}
