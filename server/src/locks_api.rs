// SPDX-License-Identifier: AGPL-3.0-or-later
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::api::{
    map_store_error, prepare_lock_elements, record_refusal, validate_name, verify_actor, ApiState,
};
use crate::auth::{Identity, Permission};
use crate::error::ApiError;
use crate::store::{now_epoch, AuditEntry, Lock};

/// The real clock in seconds, as the store requires it. The store itself never reads the
/// clock: time is passed in so lock expiry is testable without sleeping.
fn now_seconds() -> i64 {
    now_epoch().parse().unwrap_or(0)
}

/// A holder must be a real, bounded name. An empty holder could acquire a lease that blocks
/// everyone — it can never be the holder on a later write — so it is rejected outright.
/// 128 characters is ample for a username or an agent id.
fn validate_holder(holder: &str) -> Result<(), ApiError> {
    if holder.is_empty() {
        return Err(ApiError::bad_request("holder must not be empty"));
    }
    if holder.chars().count() > 128 {
        return Err(ApiError::bad_request(
            "holder must be 128 characters or fewer",
        ));
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AcquireLocks {
    pub branch: String,
    pub elements: Vec<String>,
    pub holder: String,
    pub ttl_seconds: i64,
}

pub async fn acquire_locks(
    identity: Identity,
    State(state): State<ApiState>,
    Path(project): Path<String>,
    Json(body): Json<AcquireLocks>,
) -> Result<(StatusCode, Json<Vec<Lock>>), ApiError> {
    if !identity.may(Permission::Write) {
        return Err(ApiError::forbidden("write permission required"));
    }
    if !identity.may_reach(&project) {
        return Err(ApiError::forbidden("project not in scope"));
    }
    verify_actor(&state.auth, &identity, Some(&body.holder))?;
    validate_name("branch name", &body.branch)?;
    validate_holder(&body.holder)?;
    // Validate and deduplicate the element list through the SAME shared helper the offline
    // CLI uses, so a repeated element cannot acquire one lease and be reported twice.
    let elements = prepare_lock_elements(body.elements)?;
    if !(30..=86400).contains(&body.ttl_seconds) {
        return Err(ApiError::bad_request(
            "ttlSeconds must be between 30 and 86400",
        ));
    }
    if state
        .store
        .project(&project)
        .map_err(map_store_error)?
        .is_none()
    {
        return Err(ApiError::not_found(format!("project {}", project)));
    }

    let now = now_seconds();
    let audit = AuditEntry {
        id: 0,
        project: project.clone(),
        at: now,
        actor: identity.subject.clone(),
        mechanism: state.auth.mechanism().to_string(),
        authorizer: state.auth.authorizer().unwrap_or("").to_string(),
        action: crate::audit::LOCK_ACQUIRE.to_string(),
        subject: elements.join(","),
        detail: format!(
            "{} element(s) by {} until {}",
            elements.len(),
            body.holder,
            now + body.ttl_seconds
        ),
    };
    let locks = match state.store.acquire_locks(
        &project,
        &body.branch,
        &elements,
        &body.holder,
        body.ttl_seconds,
        now,
        Some(&audit),
    ) {
        Ok(locks) => locks,
        Err(error) => {
            // A refused acquire is a refusal worth recording: another holder's lease denied
            // this one. Nothing was written, so the entry appends on its own.
            if crate::store::is_lock_refusal(&error) {
                // As with a refused commit: failing to record a refusal must not change the
                // answer. The acquire was correctly refused; the caller is told so.
                if let Err(recording) = record_refusal(
                    state.store.as_ref(),
                    &project,
                    &identity.subject,
                    state.auth.mechanism(),
                    state.auth.authorizer().unwrap_or(""),
                    crate::audit::LOCK_DENIED,
                    &elements.join(","),
                    &error.to_string(),
                ) {
                    eprintln!("could not record the refusal: {:?}", recording);
                }
            }
            return Err(map_store_error(error));
        }
    };
    Ok((StatusCode::CREATED, Json(locks)))
}

pub async fn list_locks(
    identity: Identity,
    State(state): State<ApiState>,
    Path(project): Path<String>,
) -> Result<Json<Vec<Lock>>, ApiError> {
    if !identity.may(Permission::Read) {
        return Err(ApiError::forbidden("read permission required"));
    }
    if !identity.may_reach(&project) {
        return Err(ApiError::forbidden("project not in scope"));
    }
    if state
        .store
        .project(&project)
        .map_err(map_store_error)?
        .is_none()
    {
        return Err(ApiError::not_found(format!("project {}", project)));
    }
    let locks = state
        .store
        .locks(&project, now_seconds())
        .map_err(map_store_error)?;
    Ok(Json(locks))
}

#[derive(Deserialize)]
pub struct ReleaseLocks {
    pub holder: String,
    pub ids: Vec<String>,
}

pub async fn release_locks(
    identity: Identity,
    State(state): State<ApiState>,
    Path(project): Path<String>,
    Json(body): Json<ReleaseLocks>,
) -> Result<Json<Value>, ApiError> {
    if !identity.may(Permission::Write) {
        return Err(ApiError::forbidden("write permission required"));
    }
    if !identity.may_reach(&project) {
        return Err(ApiError::forbidden("project not in scope"));
    }
    verify_actor(&state.auth, &identity, Some(&body.holder))?;
    if state
        .store
        .project(&project)
        .map_err(map_store_error)?
        .is_none()
    {
        return Err(ApiError::not_found(format!("project {}", project)));
    }
    // A release that removes nothing is not a mutation (a foreign holder, or already
    // released ids), so the store writes the audit row only when it actually removed one.
    let audit = AuditEntry {
        id: 0,
        project: project.clone(),
        at: now_seconds(),
        actor: identity.subject.clone(),
        mechanism: state.auth.mechanism().to_string(),
        authorizer: state.auth.authorizer().unwrap_or("").to_string(),
        action: crate::audit::LOCK_RELEASE.to_string(),
        subject: body.ids.join(","),
        detail: "released lock(s)".to_string(),
    };
    let released = state
        .store
        .release_locks(&project, &body.holder, &body.ids, Some(&audit))
        .map_err(map_store_error)?;
    Ok(Json(json!({ "released": released })))
}
