// SPDX-License-Identifier: AGPL-3.0-or-later
//! Migration as a gated operation: importing a source artifact through a binding is a
//! change to a repository, so it obeys the same four rules as any other migration.
//!
//! 1. The source artifact is retained byte-for-byte and content-addressed BEFORE import is
//!    even attempted, so a migration can never destroy the thing it migrated.
//! 2. No mapping is applied silently: a blocking loss (Lossy or Unmappable) refuses the
//!    import unless the request names that exact loss as accepted, and the acceptance is
//!    recorded in the audit trail.
//! 3. Fidelity is measured, not trusted: the imported document is round-tripped through the
//!    binding and diffed by the engine, which must agree nothing was lost.
//! 4. Provenance is durable: the commit records the artifact hash, the binding id and
//!    version, and the accepted losses.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::api::{
    commit_core, commit_json, load_model, map_store_error, record_refusal, resolve_author,
    validate_name, verify_actor, ApiState, CommitCore, CommitFailure,
};
use crate::audit::{IMPORT_ACCEPT, IMPORT_REFUSED};
use crate::auth::{Identity, Permission};
use crate::binding_registry;
use crate::error::ApiError;
use crate::store::{now_seconds, AuditEntry};

#[derive(Deserialize)]
pub struct ImportRequest {
    /// The binding selector, e.g. `sysml-v1-xmi@2.4`.
    pub binding: String,
    pub branch: String,
    #[serde(default)]
    pub author: String,
    pub message: String,
    /// The source artifact, base64-encoded or as a raw UTF-8 body.
    pub artifact: String,
    /// The blocking loss subjects the request accepts by name.
    #[serde(default, rename = "acceptLosses")]
    pub accept_losses: Vec<String>,
    /// Who is importing; supplying it lets a lease holder proceed, omitting it means any
    /// live lease on a changed element refuses the commit.
    pub holder: Option<String>,
}

/// Split a binding selector like `sysml-v1-xmi@2.4` into its id and version. A missing or
/// empty id or version is a request problem, reported as 400.
pub fn parse_binding(selector: &str) -> Result<(String, String), ApiError> {
    match selector.rsplit_once('@') {
        Some((id, version)) if !id.is_empty() && !version.is_empty() => {
            Ok((id.to_string(), version.to_string()))
        }
        _ => Err(ApiError::bad_request(format!(
            "binding must be of the form id@version, got {}",
            selector
        ))),
    }
}

/// Decode the artifact field: base64 when it decodes, the raw UTF-8 bytes otherwise. An XMI
/// document is text, so a raw body is the common case; base64 is accepted for binary source
/// formats a later binding may read.
fn decode_artifact(artifact: &str) -> Vec<u8> {
    use base64::Engine as _;
    match base64::engine::general_purpose::STANDARD.decode(artifact) {
        Ok(bytes) => bytes,
        Err(_) => artifact.as_bytes().to_vec(),
    }
}

/// Serialize a `Serialize` value to a JSON string, mapping a serialization failure to a
/// generic internal error rather than leaking the type's internals.
fn to_json_string<T: serde::Serialize>(value: &T) -> Result<String, ApiError> {
    serde_json::to_string(value).map_err(|e| {
        eprintln!("could not serialize import record: {}", e);
        ApiError::internal("the import record could not be prepared")
    })
}

pub async fn import_artifact(
    identity: Identity,
    State(state): State<ApiState>,
    Path(project): Path<String>,
    Json(body): Json<ImportRequest>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    // Permission and scope before anything else, exactly as every other write path.
    if !identity.may(Permission::Write) {
        return Err(ApiError::forbidden("write permission required"));
    }
    if !identity.may_reach(&project) {
        return Err(ApiError::forbidden("project not in scope"));
    }
    let author = resolve_author(&state.auth, &identity, &body.author)?;
    verify_actor(&state.auth, &identity, body.holder.as_deref())?;
    if state
        .store
        .project(&project)
        .map_err(map_store_error)?
        .is_none()
    {
        return Err(ApiError::not_found(format!("project {}", project)));
    }
    validate_name("branch name", &body.branch)?;

    // A binding that does not exist is a request problem, not a server fault.
    let (binding_id, binding_version) = parse_binding(&body.binding)?;
    let binding = binding_registry::resolve(&binding_id, &binding_version)
        .ok_or_else(|| ApiError::bad_request(format!("unknown binding {}", body.binding)))?;

    // Rule 1: retain the source artifact byte-for-byte and content-addressed BEFORE import
    // is even attempted, so nothing that follows can destroy the thing being migrated.
    let artifact_bytes = decode_artifact(&body.artifact);
    let artifact_hash = state
        .store
        .put_blob(&artifact_bytes)
        .map_err(map_store_error)?;

    // Read the artifact through the binding. A binding that cannot read it is a clean 422,
    // never a panic; the artifact is already retained, so the refusal costs nothing.
    let (root, loss_report) = binding.import(&artifact_bytes).map_err(|e| {
        ApiError::unprocessable(
            "the binding could not read the artifact",
            vec![e.to_string()],
        )
    })?;

    // Rule 3: measure fidelity with the engine's harness, not the binding's claim. The
    // imported document is round-tripped through the binding and the engine's own diff must
    // agree nothing was lost.
    let import_bytes = serde_json::to_vec(&root).map_err(|e| {
        eprintln!("imported model could not be serialised: {}", e);
        ApiError::internal("the imported model could not be prepared")
    })?;
    let fidelity = binding::round_trip(binding.as_ref(), &import_bytes).map_err(|e| {
        eprintln!("fidelity harness failed: {}", e);
        ApiError::internal("the fidelity harness could not measure the import")
    })?;

    // Record the report and the measurement keyed by the retained artifact, so a refusal
    // still has a retrievable report and the caller can read exactly which losses to accept.
    let loss_report_json = to_json_string(&loss_report)?;
    let fidelity_json = to_json_string(&fidelity.diff)?;
    state
        .store
        .record_import(
            &project,
            &artifact_hash,
            &binding_id,
            &binding_version,
            &loss_report_json,
            &fidelity_json,
        )
        .map_err(map_store_error)?;

    // A binding whose own round trip is not lossless is refused: the engine disagrees with
    // the binding, and that is exactly the failure the harness exists to catch.
    if !fidelity.diff.equal {
        if let Err(e) = record_refusal(
            state.store.as_ref(),
            &project,
            &identity.subject,
            state.auth.mechanism(),
            IMPORT_REFUSED,
            &artifact_hash,
            "the binding could not round-trip the imported model",
        ) {
            eprintln!("could not record the import refusal: {:?}", e);
        }
        return Err(ApiError::unprocessable(
            "the binding could not round-trip the imported model; the import is refused",
            fidelity.diff.missing_elements,
        ));
    }

    // Rule 2: every blocking loss must be accepted by name. Anything not accepted refuses
    // the import and is returned so the caller can decide rather than lose it silently.
    let blocking = loss_report.blocking();
    let accepted = &body.accept_losses;
    let unaccepted: Vec<&binding::Mapping> = blocking
        .iter()
        .filter(|m| !accepted.iter().any(|a| a == &m.subject))
        .copied()
        .collect();
    if !unaccepted.is_empty() {
        let subjects: Vec<String> = unaccepted.iter().map(|m| m.subject.clone()).collect();
        if let Err(e) = record_refusal(
            state.store.as_ref(),
            &project,
            &identity.subject,
            state.auth.mechanism(),
            IMPORT_REFUSED,
            &artifact_hash,
            &format!("blocking losses not accepted: {}", subjects.join(", ")),
        ) {
            eprintln!("could not record the import refusal: {:?}", e);
        }
        let blocking_json: Vec<Value> = unaccepted
            .iter()
            .map(|m| serde_json::to_value(*m).unwrap_or(Value::Null))
            .collect();
        return Ok((
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({
                "error": "the import has blocking losses that must be accepted by name",
                "blocking": blocking_json
            })),
        ));
    }

    // The acceptance is recorded BEFORE the commit, so a committed import always has its
    // acceptance on the audit trail; the actor is the verified subject, which is the "who"
    // of "who accepted what".
    let accepted_subjects: Vec<String> = blocking.iter().map(|m| m.subject.clone()).collect();
    if !accepted_subjects.is_empty() {
        state
            .store
            .append_audit(&AuditEntry {
                id: 0,
                project: project.clone(),
                at: now_seconds(),
                actor: identity.subject.clone(),
                mechanism: state.auth.mechanism().to_string(),
                action: IMPORT_ACCEPT.to_string(),
                subject: artifact_hash.clone(),
                detail: format!(
                    "binding {}@{}; accepted losses: {}",
                    binding_id,
                    binding_version,
                    accepted_subjects.join(", ")
                ),
            })
            .map_err(map_store_error)?;
    }

    // The SysML v1 XMI subset has no state-machine concept, so the binding emits none; OKF
    // requires the section to be present. An empty state machine is the honest
    // representation of "this model has no state machine", and it lets the imported document
    // pass the SAME validation every other commit passes. It is added AFTER the fidelity
    // measurement, which runs on the binding's own output and must agree nothing was lost.
    let mut commit_root = root;
    if commit_root.state_machine.is_none() {
        commit_root.state_machine = Some(okf::types::StateMachine {
            name: "stateMachine".to_string(),
            regions: Vec::new(),
        });
    }
    let commit_bytes = serde_json::to_vec(&commit_root).map_err(|e| {
        eprintln!("imported model could not be re-serialised: {}", e);
        ApiError::internal("the imported model could not be prepared")
    })?;

    // The import commit is a commit like any other: it goes through the shared commit core,
    // so validation, the lock guard and the commit.create audit entry behave exactly as they
    // do for a plain commit, a reset or a merge.
    let now = now_seconds();
    let tip_hash = state
        .store
        .branch_tip(&project, &body.branch)
        .map_err(map_store_error)?;
    let reference: Option<okf::types::OkfRoot> = match tip_hash.as_deref() {
        Some(tip) => {
            Some(load_model(state.store.as_ref(), &project, tip).map_err(map_store_error)?)
        }
        None => None,
    };
    let commit = commit_core(
        state.store.as_ref(),
        &CommitCore {
            project: &project,
            branch: &body.branch,
            author: &author,
            message: &body.message,
            actor: &identity.subject,
            mechanism: state.auth.mechanism(),
            candidate: &commit_root,
            bytes: &commit_bytes,
            holder: body.holder.as_deref().unwrap_or(""),
            now,
            tip: tip_hash.as_deref(),
            reference: reference.as_ref(),
        },
    )
    .map_err(|failure| match failure {
        CommitFailure::Invalid { errors } => {
            ApiError::unprocessable("the imported model failed validation", errors)
        }
        CommitFailure::Store(error) => map_store_error(error),
    })?;

    // Rule 4: provenance is durable. The import record now names the landed commit, the
    // artifact hash, the binding and the accepted losses. The link is best-effort: the
    // commit has already landed and the acceptance is already on the audit trail, so a
    // failed link must not turn a successful commit into a misleading 500 that invites a
    // retry - which would create a second commit for the same artifact.
    if let Err(e) =
        state
            .store
            .attach_import_commit(&project, &artifact_hash, &commit.hash, &accepted_subjects)
    {
        eprintln!(
            "could not link import {} to commit {}: {:?}",
            artifact_hash, commit.hash, e
        );
    }

    let mut commit_obj = commit_json(&commit);
    commit_obj["provenance"] = json!({
        "artifactHash": artifact_hash,
        "bindingId": binding_id,
        "bindingVersion": binding_version,
        "acceptedLosses": accepted_subjects
    });
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "commit": commit_obj,
            "lossReport": serde_json::from_str::<Value>(&loss_report_json).unwrap_or(Value::Null),
            "fidelity": serde_json::from_str::<Value>(&fidelity_json).unwrap_or(Value::Null)
        })),
    ))
}

pub async fn import_report(
    identity: Identity,
    State(state): State<ApiState>,
    Path((project, artifact_hash)): Path<(String, String)>,
) -> Result<Json<Value>, ApiError> {
    if !identity.may(Permission::Read) {
        return Err(ApiError::forbidden("read permission required"));
    }
    if !identity.may_reach(&project) {
        return Err(ApiError::forbidden("project not in scope"));
    }
    let record = state
        .store
        .import_report(&project, &artifact_hash)
        .map_err(map_store_error)?
        .ok_or_else(|| ApiError::not_found(format!("import {}", artifact_hash)))?;
    Ok(Json(json!({
        "artifactHash": record.artifact_hash,
        "bindingId": record.binding_id,
        "bindingVersion": record.binding_version,
        "lossReport": serde_json::from_str::<Value>(&record.loss_report).unwrap_or(Value::Null),
        "fidelity": serde_json::from_str::<Value>(&record.fidelity_diff).unwrap_or(Value::Null)
    })))
}
