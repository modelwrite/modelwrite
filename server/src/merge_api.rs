// SPDX-License-Identifier: AGPL-3.0-or-later
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::api::{
    commit_json, commit_refusal_guard, load_model, map_store_error, touched_elements,
    validate_name, ApiState,
};
use crate::error::ApiError;
use crate::merge::merge;
use crate::store::{now_seconds, AuditEntry, CommitGuard};

#[derive(Deserialize)]
pub struct MergeRequest {
    pub branch: String,
    pub other: String,
    pub author: String,
    pub message: String,
    /// Who is merging. Supplying it lets a caller proceed on elements it holds a lease on;
    /// omitting it means the caller cannot be the holder, so any live lease on a changed
    /// element refuses the merge rather than overwriting somebody else's work.
    pub holder: Option<String>,
}

/// Every commit reachable from a tip, as a set. The walk stops at a commit already seen, so
/// a malformed cycle in stored data cannot loop forever, and membership is constant time.
fn ancestry_set(
    state: &ApiState,
    project: &str,
    tip: &str,
) -> Result<std::collections::HashSet<String>, ApiError> {
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut stack: Vec<String> = vec![tip.to_string()];
    while let Some(hash) = stack.pop() {
        if !seen.insert(hash.clone()) {
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
    }
    Ok(seen)
}

/// The LOWEST commit both branches share: a shared commit that is not an ancestor of any
/// other shared commit.
///
/// Taking the first shared commit in walk order is not the same thing, and the difference
/// is not cosmetic. Once a merge commit exists, a walk can meet an older common ancestor
/// before the real one, and an older base silently DISCARDS a change: if theirs reverted a
/// value that ours already carries, then at the older base theirs looks unchanged, so ours
/// wins and the revert disappears with no conflict reported. When the search finds more
/// than one lowest candidate - a criss-cross history - it refuses with a conflict rather
/// than guessing, because either guess can lose work.
fn common_ancestor(
    state: &ApiState,
    project: &str,
    ours: &str,
    theirs: &str,
) -> Result<String, ApiError> {
    let our_side = ancestry_set(state, project, ours)?;
    let their_side = ancestry_set(state, project, theirs)?;
    let mut shared: Vec<String> = our_side.intersection(&their_side).cloned().collect();
    if shared.is_empty() {
        return Err(ApiError::conflict("the branches share no common ancestor"));
    }
    shared.sort();

    // Each candidate's ancestry is computed ONCE. The comparison below is quadratic in the
    // number of candidates, and recomputing a whole history walk inside that loop turned a
    // quadratic into something much worse for no reason.
    let mut ancestries: std::collections::HashMap<String, std::collections::HashSet<String>> =
        std::collections::HashMap::new();
    for candidate in &shared {
        ancestries.insert(candidate.clone(), ancestry_set(state, project, candidate)?);
    }

    let mut lowest: Vec<String> = Vec::new();
    for candidate in &shared {
        let is_ancestor_of_another = shared.iter().any(|other| {
            other != candidate
                && ancestries
                    .get(other)
                    .map(|side| side.contains(candidate))
                    .unwrap_or(false)
        });
        if !is_ancestor_of_another {
            lowest.push(candidate.clone());
        }
    }

    match lowest.len() {
        1 => Ok(lowest.remove(0)),
        // Unreachable in a finite history: the reachability relation is a partial order, so
        // a non-empty set of shared commits always has a maximal element. Kept as a refusal
        // rather than a panic, in case the data ever stops being a finite history.
        0 => Err(ApiError::internal(
            "no shared commit survived the merge base search",
        )),
        _ => Err(ApiError::conflict(
            "the branches have more than one possible merge base, so the merge cannot be computed automatically",
        )),
    }
}

pub async fn merge_branches(
    State(state): State<ApiState>,
    Path(project): Path<String>,
    Json(body): Json<MergeRequest>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    validate_name("branch name", &body.branch)?;
    validate_name("branch name", &body.other)?;
    if state
        .store
        .project(&project)
        .map_err(map_store_error)?
        .is_none()
    {
        return Err(ApiError::not_found(format!("project {}", project)));
    }

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
        // agent can resolve it deliberately rather than guess. The attempt is still
        // recorded: the audit log exists to show what was tried, not only what succeeded.
        state
            .store
            .append_audit(&AuditEntry {
                id: 0,
                project: project.clone(),
                at: now_seconds(),
                actor: body.author.clone(),
                action: "merge.conflict".to_string(),
                subject: body.branch.clone(),
                detail: format!("merge conflict between {} and {}", body.branch, body.other),
            })
            .map_err(map_store_error)?;
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

    // Every commit path validates before it stores. A merge must not be the one that skips
    // it: if a future rule ever produced a document that contradicts itself, this turns a
    // silent bad write into a loud failure.
    let report = okf::validate::validate(&merged);
    if !report.valid {
        eprintln!("merged model failed validation: {:?}", report.errors);
        return Err(ApiError::internal(
            "the merged model failed validation and was not stored",
        ));
    }

    let bytes = serde_json::to_vec(&merged).map_err(|e| {
        eprintln!("merged model could not be serialised: {}", e);
        ApiError::internal("the merged model could not be stored")
    })?;
    let okf_hash = state.store.put_blob(&bytes).map_err(map_store_error)?;
    // A merge is a write path like any other: it refuses to change an element another
    // holder has locked. The touched set is the difference between OUR tip model and the
    // MERGED model. A caller that holds the leases passes its holder and proceeds.
    let touched = touched_elements(&ours, &merged);
    let parents = vec![ours_tip, theirs_tip];
    let guard = CommitGuard {
        holder: body.holder.as_deref().unwrap_or(""),
        elements: &touched,
        now: now_seconds(),
        expected_tip: Some(&parents[0]),
    };
    let audit = AuditEntry {
        id: 0,
        project: project.clone(),
        at: now_seconds(),
        actor: body.author.clone(),
        action: "merge.clean".to_string(),
        subject: body.branch.clone(),
        detail: format!("merged {} into {}", body.other, body.branch),
    };
    let commit = commit_refusal_guard(
        &state,
        &project,
        &body.branch,
        &body.author,
        state.store.commit_merge(
            &project,
            &body.branch,
            &parents,
            &okf_hash,
            &body.author,
            &body.message,
            Some(guard),
            Some(&audit),
        ),
    )?;

    Ok((
        StatusCode::CREATED,
        Json(json!({ "commit": commit_json(&commit), "base": base_hash })),
    ))
}
