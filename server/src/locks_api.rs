// SPDX-License-Identifier: AGPL-3.0-or-later
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::api::{map_store_error, validate_name, ApiState};
use crate::error::ApiError;
use crate::store::{now_epoch, Lock};

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
            &body.elements,
            &body.holder,
            body.ttl_seconds,
            now_seconds(),
        )
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
    Ok(Json(json!({ "released": released })))
}
