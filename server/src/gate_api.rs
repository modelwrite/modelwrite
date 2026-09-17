// SPDX-License-Identifier: AGPL-3.0-or-later
use axum::extract::{Path, State};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::api::{map_store_error, ApiState};
use crate::error::ApiError;
use crate::store::{now_epoch, GateRun};

/// Load the OKF document behind a commit hash.
fn load_model(
    state: &ApiState,
    project: &str,
    hash: &str,
) -> Result<okf::types::OkfRoot, ApiError> {
    let commit = state
        .store
        .commit(project, hash)
        .map_err(map_store_error)?
        .ok_or_else(|| ApiError::not_found(format!("commit {}", hash)))?;
    let bytes = state
        .store
        .blob(&commit.okf_hash)
        .map_err(map_store_error)?
        .ok_or_else(|| ApiError::internal(format!("missing blob {}", commit.okf_hash)))?;
    serde_json::from_slice(&bytes).map_err(|e| ApiError::internal(e.to_string()))
}

fn short(hash: &str) -> &str {
    let end = 12.min(hash.len());
    &hash[..end]
}

#[derive(Deserialize)]
pub struct GateRequest {
    pub reference: String,
    pub candidate: String,
}

pub async fn run_gate(
    State(state): State<ApiState>,
    Path(project): Path<String>,
    Json(body): Json<GateRequest>,
) -> Result<Json<Value>, ApiError> {
    let reference = load_model(&state, &project, &body.reference)?;
    let candidate = load_model(&state, &project, &body.candidate)?;

    let outcome = gate::run(&reference, &candidate, false);
    let branch = state
        .store
        .commit(&project, &body.candidate)
        .map_err(map_store_error)?
        .map(|c| c.branch)
        .unwrap_or_else(|| "main".to_string());

    // Evidence lands beside the engine's records under a deterministic name: the same
    // two commits always produce the same file, so a run is reproducible and citable.
    let file_name = format!(
        "server-{}-{}-{}.json",
        project,
        short(&body.reference),
        short(&body.candidate)
    );
    let path = state.evidence_dir.join(file_name);
    gate::write_evidence(&path, &outcome.evidence)
        .map_err(|e| ApiError::internal(e.to_string()))?;

    let run = GateRun {
        project: project.clone(),
        branch,
        reference_hash: body.reference.clone(),
        candidate_hash: body.candidate.clone(),
        passed: outcome.passed,
        evidence: outcome.evidence.to_string(),
        created_at: now_epoch(),
    };
    state.store.record_gate_run(&run).map_err(map_store_error)?;

    Ok(Json(outcome.evidence))
}

pub async fn list_gate_runs(
    State(state): State<ApiState>,
    Path(project): Path<String>,
) -> Result<Json<Value>, ApiError> {
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
