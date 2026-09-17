// SPDX-License-Identifier: AGPL-3.0-or-later
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::api::{map_store_error, validate_name, ApiState};
use crate::error::ApiError;
use crate::store::{now_epoch, AuditEntry, Lock};

/// The real clock in seconds, as the store requires it. The store itself never reads the
/// clock: time is passed in so lock expiry is testable without sleeping.
fn now_seconds() -> i64 {
    now_epoch().parse().unwrap_or(0)
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
    if body.elements.is_empty() {
        return Err(ApiError::bad_request("at least one element is required"));
    }
    for element in &body.elements {
        validate_name("element name", element)?;
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

    let locks = state
        .store
        .acquire_locks(
            &project,
            &body.branch,
            &elements,
            &body.holder,
            body.ttl_seconds,
            now_seconds(),
        )
        .map_err(map_store_error)?;
    let expiry = locks.first().map(|l| l.expires_at).unwrap_or(0);
    state
        .store
        .append_audit(&AuditEntry {
            id: 0,
            project: project.clone(),
            at: now_seconds(),
            actor: body.holder.clone(),
            action: "lock.acquire".to_string(),
            subject: elements.join(","),
            detail: format!(
                "{} element(s) by {} until {}",
                locks.len(),
                body.holder,
                expiry
            ),
        })
        .map_err(map_store_error)?;
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
    let released = state
        .store
        .release_locks(&project, &body.holder, &body.ids)
        .map_err(map_store_error)?;
    // A release that removes nothing is not a mutation (a foreign holder, or already
    // released ids), so it writes no audit entry.
    if released > 0 {
        state
            .store
            .append_audit(&AuditEntry {
                id: 0,
                project: project.clone(),
                at: now_seconds(),
                actor: body.holder.clone(),
                action: "lock.release".to_string(),
                subject: body.ids.join(","),
                detail: format!("released {} lock(s)", released),
            })
            .map_err(map_store_error)?;
    }
    Ok(Json(json!({ "released": released })))
}
