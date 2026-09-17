// SPDX-License-Identifier: AGPL-3.0-or-later
use axum::extract::{Path, Query, State};
use axum::Json;
use serde::Deserialize;

use crate::api::{map_store_error, ApiState};
use crate::auth::{Identity, Permission};
use crate::error::ApiError;
use crate::store::AuditEntry;

#[derive(Deserialize)]
pub struct AuditQuery {
    pub limit: Option<i64>,
}

/// The append-only audit log for a project, newest first. The log records what was
/// attempted as well as what succeeded, and can never be edited: the store exposes only
/// an append and this read, never an update or a delete.
pub async fn list_audit(
    identity: Identity,
    State(state): State<ApiState>,
    Path(project): Path<String>,
    Query(query): Query<AuditQuery>,
) -> Result<Json<Vec<AuditEntry>>, ApiError> {
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
    let limit = query.limit.unwrap_or(1000);
    if limit < 1 {
        return Err(ApiError::bad_request("limit must be at least 1"));
    }
    let entries = state
        .store
        .audit(&project, limit)
        .map_err(map_store_error)?;
    Ok(Json(entries))
}
