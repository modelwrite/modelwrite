// SPDX-License-Identifier: AGPL-3.0-or-later
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::api::{map_store_error, record_refusal, validate_element_name, validate_name, ApiState};
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
    State(state): State<ApiState>,
    Path(project): Path<String>,
    Json(body): Json<AcquireLocks>,
) -> Result<(StatusCode, Json<Vec<Lock>>), ApiError> {
    validate_name("branch name", &body.branch)?;
    validate_holder(&body.holder)?;
    if body.elements.is_empty() {
        return Err(ApiError::bad_request("at least one element is required"));
    }
    for element in &body.elements {
        validate_element_name(element)?;
    }
    // A repeated element would acquire one lease but be reported twice, which reads as two
    // leases for one element. Deduplicate before anything is written.
    let elements: Vec<String> = {
        let mut seen = std::collections::BTreeSet::new();
        body.elements
            .iter()
            .filter(|e| seen.insert((*e).clone()))
            .cloned()
            .collect()
    };
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
        actor: body.holder.clone(),
        action: "lock.acquire".to_string(),
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
                record_refusal(
                    state.store.as_ref(),
                    &project,
                    &body.holder,
                    "lock.denied",
                    &elements.join(","),
                    &error.to_string(),
                )?;
            }
            return Err(map_store_error(error));
        }
    };
    Ok((StatusCode::CREATED, Json(locks)))
}

pub async fn list_locks(
    State(state): State<ApiState>,
    Path(project): Path<String>,
) -> Result<Json<Vec<Lock>>, ApiError> {
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
    State(state): State<ApiState>,
    Path(project): Path<String>,
    Json(body): Json<ReleaseLocks>,
) -> Result<Json<Value>, ApiError> {
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
        actor: body.holder.clone(),
        action: "lock.release".to_string(),
        subject: body.ids.join(","),
        detail: "released lock(s)".to_string(),
    };
    let released = state
        .store
        .release_locks(&project, &body.holder, &body.ids, Some(&audit))
        .map_err(map_store_error)?;
    Ok(Json(json!({ "released": released })))
}
