// SPDX-License-Identifier: AGPL-3.0-or-later
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::api::{
    commit_json, commit_refusal_guard, load_model, map_store_error, resolve_author,
    touched_elements, validate_name, verify_actor, ApiState,
};
use crate::audit::{MERGE_CLEAN, MERGE_CONFLICT};
use crate::auth::{Identity, Permission};
use crate::error::ApiError;
use crate::merge::merge;
use crate::store::{now_seconds, AuditEntry, Commit, CommitGuard, Store, StoreError};

#[derive(Deserialize)]
pub struct MergeRequest {
    pub branch: String,
    pub other: String,
    #[serde(default)]
    pub author: String,
    pub message: String,
    /// Who is merging. Supplying it lets a caller proceed on elements it holds a lease on;
    /// omitting it means the caller cannot be the holder, so any live lease on a changed
    /// element refuses the merge rather than overwriting somebody else's work.
    pub holder: Option<String>,
}

/// Every commit reachable from a tip, as a set. The walk stops at a commit already seen, so
/// a malformed cycle in stored data cannot loop forever, and membership is constant time.
pub fn ancestry_set(
    store: &dyn Store,
    project: &str,
    tip: &str,
) -> Result<std::collections::HashSet<String>, StoreError> {
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut stack: Vec<String> = vec![tip.to_string()];
    while let Some(hash) = stack.pop() {
        if !seen.insert(hash.clone()) {
            continue;
        }
        if let Some(commit) = store.commit(project, &hash)? {
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
pub fn common_ancestor(
    store: &dyn Store,
    project: &str,
    ours: &str,
    theirs: &str,
) -> Result<String, StoreError> {
    let our_side = ancestry_set(store, project, ours)?;
    let their_side = ancestry_set(store, project, theirs)?;
    let mut shared: Vec<String> = our_side.intersection(&their_side).cloned().collect();
    if shared.is_empty() {
        return Err(StoreError::Conflict(
            "the branches share no common ancestor".to_string(),
        ));
    }
    shared.sort();

    // Each candidate's ancestry is computed ONCE. The comparison below is quadratic in the
    // number of candidates, and recomputing a whole history walk inside that loop turned a
    // quadratic into something much worse for no reason.
    let mut ancestries: std::collections::HashMap<String, std::collections::HashSet<String>> =
        std::collections::HashMap::new();
    for candidate in &shared {
        ancestries.insert(candidate.clone(), ancestry_set(store, project, candidate)?);
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
        0 => Err(StoreError::Backend(
            "no shared commit survived the merge base search".to_string(),
        )),
        _ => Err(StoreError::Conflict(
            "the branches have more than one possible merge base, so the merge cannot be computed automatically".to_string(),
        )),
    }
}

pub async fn merge_branches(
    identity: Identity,
    State(state): State<ApiState>,
    Path(project): Path<String>,
    Json(body): Json<MergeRequest>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    if !identity.may(Permission::Write) {
        return Err(ApiError::forbidden("write permission required"));
    }
    if !identity.may_reach(&project) {
        return Err(ApiError::forbidden("project not in scope"));
    }
    let author = resolve_author(&state.auth, &identity, &body.author)?;
    verify_actor(&state.auth, &identity, body.holder.as_deref())?;
    validate_name("branch name", &body.branch)?;
    validate_name("branch name", &body.other)?;

    match merge_core(
        state.store.as_ref(),
        &project,
        &MergeCore {
            branch: &body.branch,
            other: &body.other,
            author: &author,
            message: &body.message,
            holder: body.holder.as_deref(),
            actor: &identity.subject,
            mechanism: state.auth.mechanism(),
        },
    )
    .map_err(map_store_error)?
    {
        MergeOutcome::Merged { commit, base } => Ok((
            StatusCode::CREATED,
            Json(json!({ "commit": commit_json(&commit), "base": base })),
        )),
        MergeOutcome::Conflict { base, conflicts } => Ok((
            StatusCode::CONFLICT,
            Json(json!({
                "base": base,
                "branch": body.branch,
                "other": body.other,
                "conflicts": conflicts
            })),
        )),
    }
}

/// The outcome of a merge, described without any HTTP or page rendering. The JSON handler
/// and the merge page each map this to their own surface, so the two cannot diverge on what
/// a merge actually did.
pub enum MergeOutcome {
    Merged {
        commit: Commit,
        base: String,
    },
    Conflict {
        base: String,
        conflicts: Vec<crate::merge::Conflict>,
    },
}

/// Everything the merge core needs that the caller resolved upstream. The author is already
/// resolved against the identity; `holder` is the optional holder (empty becomes `None`).
pub struct MergeCore<'a> {
    pub branch: &'a str,
    pub other: &'a str,
    pub author: &'a str,
    pub message: &'a str,
    pub holder: Option<&'a str>,
    pub actor: &'a str,
    pub mechanism: &'a str,
}

/// The ONE implementation of a merge: resolve the two branch tips and their lowest common
/// ancestor, run the three-way merge, then either record the conflict or validate, lock-guard
/// and commit the merged document with its audit row. Both [merge_branches] and the merge
/// page's `perform_merge` call this, so they cannot record different audit entries or apply
/// a different lock strategy.
pub fn merge_core(
    store: &dyn Store,
    project: &str,
    input: &MergeCore<'_>,
) -> Result<MergeOutcome, StoreError> {
    if store.project(project)?.is_none() {
        return Err(StoreError::NotFound(format!("project {}", project)));
    }
    let ours_tip = store
        .branch_tip(project, input.branch)?
        .ok_or_else(|| StoreError::NotFound(format!("branch {}", input.branch)))?;
    let theirs_tip = store
        .branch_tip(project, input.other)?
        .ok_or_else(|| StoreError::NotFound(format!("branch {}", input.other)))?;

    let base_hash = common_ancestor(store, project, &ours_tip, &theirs_tip)?;
    let base = load_model(store, project, &base_hash)?;
    let ours = load_model(store, project, &ours_tip)?;
    let theirs = load_model(store, project, &theirs_tip)?;

    let outcome = merge(&base, &ours, &theirs);
    if !outcome.conflicts.is_empty() {
        // A conflict is not a transport failure: nothing is written, and the caller gets
        // every conflicting subject with its base, ours and theirs values so a human or an
        // agent can resolve it deliberately rather than guess. The attempt is still
        // recorded: the audit log exists to show what was tried, not only what succeeded.
        store.append_audit(&AuditEntry {
            id: 0,
            project: project.to_string(),
            at: now_seconds(),
            actor: input.actor.to_string(),
            mechanism: input.mechanism.to_string(),
            action: MERGE_CONFLICT.to_string(),
            subject: input.branch.to_string(),
            detail: format!(
                "merge conflict between {} and {}",
                input.branch, input.other
            ),
        })?;
        return Ok(MergeOutcome::Conflict {
            base: base_hash,
            conflicts: outcome.conflicts,
        });
    }

    let merged = outcome.merged.ok_or_else(|| {
        StoreError::Backend("the merge reported no conflicts but produced nothing".to_string())
    })?;

    // Every commit path validates before it stores. A merge must not be the one that skips
    // it: if a future rule ever produced a document that contradicts itself, this turns a
    // silent bad write into a loud failure.
    let report = okf::validate::validate(&merged);
    if !report.valid {
        eprintln!("merged model failed validation: {:?}", report.errors);
        return Err(StoreError::Backend(
            "the merged model failed validation and was not stored".to_string(),
        ));
    }

    let touched = touched_elements(&ours, &merged);

    // Refuse before storing the merged document, so a lock-refused merge leaves no
    // orphaned blob. The store checks again inside its transaction, and that check is the
    // authority; this one avoids writing bytes we already know will be rejected.
    {
        let held = store.holders_of(project, &touched, now_seconds())?;
        let holder = input.holder.unwrap_or("");
        if let Some(blocked) = held.iter().find(|lock| lock.holder != holder) {
            return Err(StoreError::Locked {
                element: blocked.element.clone(),
                holder: blocked.holder.clone(),
                expires_at: blocked.expires_at,
            });
        }
    }

    let bytes = serde_json::to_vec(&merged).map_err(|e| {
        eprintln!("merged model could not be serialised: {}", e);
        StoreError::Backend("the merged model could not be stored".to_string())
    })?;
    let okf_hash = store.put_blob(&bytes)?;
    // A merge is a write path like any other: it refuses to change an element another
    // holder has locked. The touched set is the difference between OUR tip model and the
    // MERGED model. A caller that holds the leases passes its holder and proceeds.
    let parents = vec![ours_tip.clone(), theirs_tip.clone()];
    let guard = CommitGuard {
        holder: input.holder.unwrap_or(""),
        elements: &touched,
        now: now_seconds(),
        expected_tip: Some(&parents[0]),
    };
    let audit = AuditEntry {
        id: 0,
        project: project.to_string(),
        at: now_seconds(),
        actor: input.actor.to_string(),
        mechanism: input.mechanism.to_string(),
        action: MERGE_CLEAN.to_string(),
        subject: input.branch.to_string(),
        detail: format!("merged {} into {}", input.other, input.branch),
    };
    let commit = commit_refusal_guard(
        store,
        project,
        input.branch,
        input.actor,
        input.mechanism,
        store.commit_merge(
            project,
            input.branch,
            &parents,
            &okf_hash,
            input.author,
            input.message,
            Some(guard),
            Some(&audit),
        ),
    )?;
    Ok(MergeOutcome::Merged {
        commit,
        base: base_hash,
    })
}
