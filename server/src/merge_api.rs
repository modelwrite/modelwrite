// SPDX-License-Identifier: AGPL-3.0-or-later
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::api::{commit_json, load_model, map_store_error, validate_name, ApiState};
use crate::error::ApiError;
use crate::merge::merge;

#[derive(Deserialize)]
pub struct MergeRequest {
    pub branch: String,
    pub other: String,
    pub author: String,
    pub message: String,
}

/// Every commit reachable from a tip, tip first. The walk stops at a commit already seen,
/// so a malformed cycle in stored data cannot loop forever.
fn ancestry(state: &ApiState, project: &str, tip: &str) -> Result<Vec<String>, ApiError> {
    let mut seen: Vec<String> = Vec::new();
    let mut stack: Vec<String> = vec![tip.to_string()];
    while let Some(hash) = stack.pop() {
        if seen.contains(&hash) {
            continue;
        }
        if let Some(commit) = state
            .store
            .commit(project, &hash)
            .map_err(map_store_error)?
        {
            for parent in &commit.parents {
                stack.push(parent.clone());
            }
        }
        seen.push(hash);
    }
    Ok(seen)
}

/// The nearest commit both branches share, found by walking our ancestry and taking the
/// first hash that theirs also reaches. With a linear history that is exactly the fork
/// point; with merge commits it is an approximation, which is documented rather than
/// hidden: a general lowest-common-ancestor is a later refinement.
fn common_ancestor(
    state: &ApiState,
    project: &str,
    ours: &str,
    theirs: &str,
) -> Result<String, ApiError> {
    let our_side = ancestry(state, project, ours)?;
    let their_side = ancestry(state, project, theirs)?;
    our_side
        .iter()
        .find(|hash| their_side.contains(*hash))
        .cloned()
        .ok_or_else(|| ApiError::conflict("the branches share no common ancestor"))
}

pub async fn merge_branches(
    State(state): State<ApiState>,
    Path(project): Path<String>,
    Json(body): Json<MergeRequest>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    validate_name("branch name", &body.branch)?;
    validate_name("branch name", &body.other)?;

    let ours_tip = state
        .store
        .branch_tip(&project, &body.branch)
        .map_err(map_store_error)?
        .ok_or_else(|| ApiError::not_found(format!("branch {}", body.branch)))?;
    let theirs_tip = state
        .store
        .branch_tip(&project, &body.other)
        .map_err(map_store_error)?
        .ok_or_else(|| ApiError::not_found(format!("branch {}", body.other)))?;

    let base_hash = common_ancestor(&state, &project, &ours_tip, &theirs_tip)?;
    let base = load_model(&state, &project, &base_hash)?;
    let ours = load_model(&state, &project, &ours_tip)?;
    let theirs = load_model(&state, &project, &theirs_tip)?;

    let outcome = merge(&base, &ours, &theirs);
    if !outcome.conflicts.is_empty() {
        // A conflict is not a transport failure: nothing is written, and the caller gets
        // every conflicting subject with its base, ours and theirs values so a human or an
        // agent can resolve it deliberately rather than guess.
        return Ok((
            StatusCode::CONFLICT,
            Json(json!({
                "base": base_hash,
                "branch": body.branch,
                "other": body.other,
                "conflicts": outcome.conflicts
            })),
        ));
    }

    let merged = outcome.merged.ok_or_else(|| {
        ApiError::internal("the merge reported no conflicts but produced nothing")
    })?;
    let bytes = serde_json::to_vec(&merged).map_err(|e| {
        eprintln!("merged model could not be serialised: {}", e);
        ApiError::internal("the merged model could not be stored")
    })?;
    let okf_hash = state.store.put_blob(&bytes).map_err(map_store_error)?;
    let commit = state
        .store
        .commit_merge(
            &project,
            &body.branch,
            &[ours_tip, theirs_tip],
            &okf_hash,
            &body.author,
            &body.message,
        )
        .map_err(map_store_error)?;

    Ok((
        StatusCode::CREATED,
        Json(json!({ "commit": commit_json(&commit), "base": base_hash })),
    ))
}
