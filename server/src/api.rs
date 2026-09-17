// SPDX-License-Identifier: AGPL-3.0-or-later
use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::error::ApiError;
use crate::store::{commit_hash, now_epoch, Commit, Store, StoreError};

#[derive(Clone)]
pub struct ApiState {
    pub store: Arc<dyn Store>,
    pub evidence_dir: std::path::PathBuf,
}

pub fn map_store_error(e: StoreError) -> ApiError {
    match e {
        StoreError::NotFound(m) => ApiError::not_found(m),
        StoreError::Conflict(m) => ApiError::conflict(m),
        StoreError::Backend(m) => ApiError::internal(m),
    }
}

fn commit_json(commit: &Commit) -> Value {
    json!({
        "hash": commit.hash,
        "project": commit.project,
        "branch": commit.branch,
        "parents": commit.parents,
        "okfHash": commit.okf_hash,
        "author": commit.author,
        "message": commit.message,
        "createdAt": commit.created_at
    })
}

#[derive(Deserialize)]
pub struct CreateProject {
    pub name: String,
}

pub async fn create_project(
    State(state): State<ApiState>,
    Json(body): Json<CreateProject>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    if body.name.trim().is_empty() {
        return Err(ApiError::bad_request("project name must not be empty"));
    }
    let project = state
        .store
        .create_project(&body.name)
        .map_err(map_store_error)?;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "name": project.name, "createdAt": project.created_at })),
    ))
}

pub async fn list_projects(State(state): State<ApiState>) -> Result<Json<Value>, ApiError> {
    let projects = state.store.list_projects().map_err(map_store_error)?;
    let out: Vec<Value> = projects
        .iter()
        .map(|p| json!({ "name": p.name, "createdAt": p.created_at }))
        .collect();
    Ok(Json(Value::Array(out)))
}

#[derive(Deserialize)]
pub struct CreateCommit {
    pub branch: String,
    pub author: String,
    pub message: String,
    pub okf: Value,
}

pub async fn create_commit(
    State(state): State<ApiState>,
    Path(project): Path<String>,
    Json(body): Json<CreateCommit>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    if state
        .store
        .project(&project)
        .map_err(map_store_error)?
        .is_none()
    {
        return Err(ApiError::not_found(format!("project {}", project)));
    }
    if body.branch.trim().is_empty() {
        return Err(ApiError::bad_request("branch must not be empty"));
    }

    // The document must be a valid OKF model before it is stored: a repository that
    // accepts invalid models cannot be gated meaningfully.
    let bytes = serde_json::to_vec(&body.okf).map_err(|e| ApiError::bad_request(e.to_string()))?;
    let root: okf::types::OkfRoot = serde_json::from_slice(&bytes)
        .map_err(|e| ApiError::bad_request(format!("not an OKF document: {}", e)))?;
    let report = okf::validate::validate(&root);
    if !report.valid {
        return Err(ApiError::unprocessable(
            "the model failed validation",
            report.errors,
        ));
    }

    let okf_hash = state.store.put_blob(&bytes).map_err(map_store_error)?;
    let parents: Vec<String> = state
        .store
        .branch_tip(&project, &body.branch)
        .map_err(map_store_error)?
        .into_iter()
        .collect();
    let hash = commit_hash(
        &project,
        &body.branch,
        &parents,
        &okf_hash,
        &body.author,
        &body.message,
    );
    let commit = Commit {
        hash: hash.clone(),
        project: project.clone(),
        branch: body.branch.clone(),
        parents: parents.clone(),
        okf_hash,
        author: body.author,
        message: body.message,
        created_at: now_epoch(),
    };
    state
        .store
        .append_commit(&commit)
        .map_err(map_store_error)?;
    Ok((StatusCode::CREATED, Json(commit_json(&commit))))
}

#[derive(Deserialize)]
pub struct BranchQuery {
    pub branch: Option<String>,
}

pub async fn list_commits(
    State(state): State<ApiState>,
    Path(project): Path<String>,
    Query(query): Query<BranchQuery>,
) -> Result<Json<Value>, ApiError> {
    let branch = query.branch.unwrap_or_else(|| "main".to_string());
    let commits = state
        .store
        .commits_on(&project, &branch)
        .map_err(map_store_error)?;
    Ok(Json(Value::Array(
        commits.iter().map(commit_json).collect(),
    )))
}

pub async fn get_commit(
    State(state): State<ApiState>,
    Path((project, hash)): Path<(String, String)>,
) -> Result<Json<Value>, ApiError> {
    let commit = state
        .store
        .commit(&project, &hash)
        .map_err(map_store_error)?
        .ok_or_else(|| ApiError::not_found(format!("commit {}", hash)))?;
    let bytes = state
        .store
        .blob(&commit.okf_hash)
        .map_err(map_store_error)?
        .ok_or_else(|| ApiError::internal(format!("missing blob {}", commit.okf_hash)))?;
    let value: Value =
        serde_json::from_slice(&bytes).map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(Json(value))
}

#[derive(Deserialize)]
pub struct CreateBranch {
    pub name: String,
    pub from: String,
}

pub async fn create_branch(
    State(state): State<ApiState>,
    Path(project): Path<String>,
    Json(body): Json<CreateBranch>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    state
        .store
        .create_branch(&project, &body.name, &body.from)
        .map_err(map_store_error)?;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "name": body.name, "tip": body.from })),
    ))
}
