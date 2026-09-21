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
    let reference = load_model(
        state.store_for(&identity).as_ref(),
        &project,
        &body.reference,
    )
    .map_err(map_store_error)?;
    let candidate = load_model(
        state.store_for(&identity).as_ref(),
        &project,
        &body.candidate,
    )
    .map_err(map_store_error)?;

    let mut outcome = gate::run(&reference, &candidate, false);

    // The compositional gate (S1) runs ON TOP of the single-model gate. It measures, over
    // the candidate platform model: that every reference resolves at its pinned revision,
    // that every integrated revision was itself gated, and that platform coverage holds
    // locally. It also states the measured-vs-asserted boundary as data. A named
    // resolution or was-gated failure refuses the integration, exactly like a single-model
    // failure; the composition findings are merged into the evidence so the report cannot
    // blur a proof and a claim.
    let composition = crate::composition::check(state.store_for(&identity).as_ref(), &candidate)
        .map_err(map_store_error)?;
    outcome.failures.extend(composition.failures);
    outcome.passed = outcome.failures.is_empty();
    outcome.evidence["composition"] = composition.evidence;
    outcome.evidence["passed"] = json!(outcome.passed);
    outcome.evidence["failures"] =
        serde_json::to_value(&outcome.failures).expect("failures serialize");

    let branch = state
        .store_for(&identity)
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
        actor: identity.subject.clone(),
        mechanism: state.auth.mechanism().to_string(),
        authorizer: state.auth.authorizer().unwrap_or("").to_string(),
        action: crate::audit::GATE_RUN.to_string(),
        subject: body.candidate.clone(),
        detail: format!(
            "{}: candidate {} against reference {}",
            verdict, body.candidate, body.reference
        ),
    };
    state
        .store_for(&identity)
        .record_gate_run(&run, Some(&audit))
        .map_err(map_store_error)?;

    // The name carries both FULL hashes. Truncating them would keep determinism but lose
    // uniqueness: two commit pairs sharing a prefix would overwrite each other's evidence
    // while both runs stayed recorded, so a run would cite a file it never wrote.
    // The name carries both FULL hashes and no project name: the hashes are globally
    // unique, and keeping an unvalidated string out of a filesystem path removes any
    // chance of a separator escaping the evidence directory.
    let file_name = format!("server-{}-{}.json", body.reference, body.candidate);
    // The evidence directory may not exist yet on a fresh install (every test passes a
    // pre-existing tempdir, which is why this went unnoticed). Create it so the first gate
    // run does not return 500 after the run itself was already recorded.
    std::fs::create_dir_all(&state.evidence_dir).map_err(|e| {
        eprintln!(
            "could not create evidence directory {}: {}",
            state.evidence_dir.display(),
            e
        );
        ApiError::internal("the evidence directory could not be created")
    })?;
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
    // An author may RUN a gate (Write), so it must be able to SEE the result too; a
    // reviewer reads runs it did not start. Either role may list.
    if !identity.may(Permission::Write) && !identity.may(Permission::Review) {
        return Err(ApiError::forbidden("write or review permission required"));
    }
    if !identity.may_reach(&project) {
        return Err(ApiError::forbidden("project not in scope"));
    }
    let runs = state
        .store_for(&identity)
        .gate_runs(&project)
        .map_err(map_store_error)?;
    let out: Vec<Value> = runs.iter().map(gate_run_json).collect();
    Ok(Json(Value::Array(out)))
}

/// The JSON a recorded gate run reads as. Shared by the project-wide listing and the
/// per-commit checks endpoint, so the two surfaces can never disagree on the shape of one
/// run - and both render the recorded evidence, which is the authority, never a re-run.
fn gate_run_json(run: &GateRun) -> Value {
    json!({
        "project": run.project,
        "branch": run.branch,
        "referenceHash": run.reference_hash,
        "candidateHash": run.candidate_hash,
        "passed": run.passed,
        "evidence": serde_json::from_str::<Value>(&run.evidence).unwrap_or(Value::Null),
        "createdAt": run.created_at
    })
}

/// `GET /projects/:project/commits/:hash/checks` - what has been checked about THIS model.
///
/// A check is attached to the exact commit it was run against, never to a branch or a
/// project: the store filters on the candidate hash, so a later commit does not inherit an
/// earlier verdict. A commit that exists but has no runs is the one answer that must be
/// explicit rather than implied - an empty list reads like a pass, which is the most
/// dangerous ambiguity in a tool whose proposition is that a model can be proved - so the
/// response carries a `checked` flag: `false` says UNCHECKED, never a bare empty list.
pub async fn commit_checks(
    identity: Identity,
    State(state): State<ApiState>,
    Path((project, hash)): Path<(String, String)>,
) -> Result<Json<Value>, ApiError> {
    // The answer exposes a gate run's verdict and evidence, so it is held to the same
    // permission as the project-wide listing: an author may run a gate and see its result,
    // a reviewer reads runs it did not start.
    if !identity.may(Permission::Write) && !identity.may(Permission::Review) {
        return Err(ApiError::forbidden("write or review permission required"));
    }
    if !identity.may_reach(&project) {
        return Err(ApiError::forbidden("project not in scope"));
    }
    // A missing commit is a 404, never a silent "unchecked": a caller must be able to tell
    // "this model does not exist" from "this model exists and nobody checked it", and the
    // 404 names only the missing commit, never internal detail.
    let commit = state
        .store_for(&identity)
        .commit(&project, &hash)
        .map_err(map_store_error)?
        .ok_or_else(|| ApiError::not_found(format!("commit {}", hash)))?;
    let runs = state
        .store_for(&identity)
        .gate_runs_for_commit(&project, &hash)
        .map_err(map_store_error)?;
    let checked = !runs.is_empty();
    let checks: Vec<Value> = runs.iter().map(gate_run_json).collect();
    Ok(Json(json!({
        "commit": commit.hash,
        "checked": checked,
        "checks": checks,
    })))
}
