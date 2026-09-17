// SPDX-License-Identifier: AGPL-3.0-or-later
use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::auth::AuthConfig;
use crate::error::ApiError;
use crate::store::{
    is_lock_refusal, now_epoch, AuditEntry, Commit, CommitGuard, Store, StoreError,
};

#[derive(Clone)]
pub struct ApiState {
    pub store: Arc<dyn Store>,
    pub evidence_dir: std::path::PathBuf,
    pub auth: AuthConfig,
}

/// A name that is safe as a URL segment: letters, digits, dot, underscore and hyphen,
/// at most 64 characters, and at least one letter or digit so that "." and ".." are
/// rejected. The charset is deliberately narrow rather than merely path-safe, so names
/// stay predictable in URLs and logs; it does mean a Git-style slashed branch name is
/// not accepted, which is a deliberate restriction, not an oversight.
pub fn validate_name(kind: &str, name: &str) -> Result<(), ApiError> {
    if name.is_empty() {
        return Err(ApiError::bad_request(format!("{} must not be empty", kind)));
    }
    if name.len() > 64 {
        return Err(ApiError::bad_request(format!(
            "{} must be 64 characters or fewer",
            kind
        )));
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    {
        return Err(ApiError::bad_request(format!(
            "{} may contain only letters, digits, dot, underscore and hyphen",
            kind
        )));
    }
    // Dots are allowed (coffee-machine.v2), which means ".." and "." pass the charset
    // check while being the very thing a path must never contain. Requiring one letter or
    // digit excludes them without narrowing the usable names.
    if !name.chars().any(|c| c.is_ascii_alphanumeric()) {
        return Err(ApiError::bad_request(format!(
            "{} must contain at least one letter or digit",
            kind
        )));
    }
    Ok(())
}

/// Validate an element id taken from an OKF document. This is deliberately wider than
/// `validate_name`: element ids come from the model itself, and their charset may include
/// characters `validate_name` rejects. An element that can be CHANGED must also be LOCKED,
/// so the only requirements are non-empty, no control characters, and at most 256 bytes.
pub fn validate_element_name(name: &str) -> Result<(), ApiError> {
    if name.is_empty() {
        return Err(ApiError::bad_request("element name must not be empty"));
    }
    if name.len() > 256 {
        return Err(ApiError::bad_request(
            "element name must be 256 bytes or fewer",
        ));
    }
    if name.chars().any(|c| c.is_control()) {
        return Err(ApiError::bad_request(
            "element name must not contain control characters",
        ));
    }
    Ok(())
}

/// Storage failures are logged with their detail and reported to the caller as a generic
/// internal error: the detail names schema objects and hashes, which is not the client's
/// business once this stops binding to localhost.
pub fn map_store_error(e: StoreError) -> ApiError {
    match e {
        StoreError::NotFound(m) => ApiError::not_found(m),
        StoreError::Conflict(m) => ApiError::conflict(m),
        StoreError::Locked {
            element,
            holder,
            expires_at,
        } => ApiError::conflict(format!(
            "{} is locked by {} until {}; a caller who holds this lease must supply the holder field to proceed",
            element, holder, expires_at
        )),
        StoreError::Backend(m) => {
            eprintln!("storage error: {}", m);
            ApiError::internal("internal storage error")
        }
    }
}

/// Load the OKF document behind a commit hash.
pub fn load_model(
    state: &ApiState,
    project: &str,
    hash: &str,
) -> Result<okf::types::OkfRoot, ApiError> {
    let commit = state
        .store
        .commit(project, hash)
        .map_err(map_store_error)?
        .ok_or_else(|| ApiError::not_found(format!("commit {}", hash)))?;
    let bytes = state
        .store
        .blob(&commit.okf_hash)
        .map_err(map_store_error)?
        .ok_or_else(|| {
            eprintln!("missing blob {}", commit.okf_hash);
            ApiError::internal("the stored model is missing")
        })?;
    serde_json::from_slice(&bytes).map_err(|e| {
        eprintln!("stored model is not readable: {}", e);
        ApiError::internal("the stored model could not be read")
    })
}

/// The real clock in seconds, as the store requires it. The store itself never reads the
/// clock: time is passed in so lock expiry is testable without sleeping.
fn now_seconds() -> i64 {
    now_epoch().parse().unwrap_or(0)
}

/// Record a refusal - an action that was ATTEMPTED but refused - in the audit log. A
/// refusal is not a mutation, so it appends directly rather than riding a transaction; the
/// event matters even though nothing changed.
pub fn record_refusal(
    store: &dyn Store,
    project: &str,
    actor: &str,
    action: &str,
    subject: &str,
    detail: &str,
) -> Result<(), ApiError> {
    store
        .append_audit(&AuditEntry {
            id: 0,
            project: project.to_string(),
            at: now_seconds(),
            actor: actor.to_string(),
            action: action.to_string(),
            subject: subject.to_string(),
            detail: detail.to_string(),
        })
        .map(|_| ())
        .map_err(map_store_error)
}

/// Call a commit-producing store method and, if it is refused because an element is
/// locked, record commit.refused before surfacing the 409. A lock refusal is the
/// highest-value event this feature produces: it is the overwrite the lock prevented.
pub fn commit_refusal_guard(
    state: &ApiState,
    project: &str,
    branch: &str,
    author: &str,
    result: Result<Commit, StoreError>,
) -> Result<Commit, ApiError> {
    match result {
        Ok(commit) => Ok(commit),
        Err(error) if is_lock_refusal(&error) => {
            let detail = error.to_string();
            // Recording the refusal must never change the answer. If the log cannot be
            // written, that is a problem with the log, not with the caller: the write was
            // still correctly refused, and reporting 500 would tell the caller their commit
            // failed for an unrelated reason and invite a retry that cannot succeed.
            if let Err(recording) = record_refusal(
                state.store.as_ref(),
                project,
                author,
                "commit.refused",
                branch,
                &detail,
            ) {
                eprintln!("could not record the refusal: {:?}", recording);
            }
            Err(map_store_error(error))
        }
        Err(error) => Err(map_store_error(error)),
    }
}

/// Every element an incoming document names, used when a branch has no tip yet.
///
/// A first commit has nothing to diff against, and diffing a document against itself
/// yields an EMPTY touched set - so a lease taken before the branch existed would not be
/// enforced. Treating "no tip" as "an empty document" gives the honest answer: every
/// element this commit introduces is an element it changes.
fn all_touched(root: &okf::types::OkfRoot) -> Vec<String> {
    let mut ids = okf::diff::element_ids(root);
    if let Some(graph) = &root.graph {
        for edge in &graph.edges {
            ids.insert(edge.source.clone());
            ids.insert(edge.target.clone());
        }
    }
    ids.into_iter().collect()
}

/// The elements a commit changes: every diff entry that names an element, plus the
/// endpoints of changed edges. A lock protects an element from being CHANGED, so an
/// untouched element elsewhere in the document does not block the commit.
pub fn touched_elements(
    reference: &okf::types::OkfRoot,
    candidate: &okf::types::OkfRoot,
) -> Vec<String> {
    let report = okf::diff::diff(reference, candidate);
    let mut ids: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();

    // Element entries are keyed "<section>:<id>". Two kinds of key are skipped on purpose:
    // "doc:" keys name document fields rather than elements, and the activity keys are list
    // indices rather than ids, so neither could be the subject of a lock.
    for key in report
        .missing_elements
        .iter()
        .chain(report.extra_elements.iter())
        .chain(report.changed_attributes.iter())
    {
        if let Some((section, id)) = key.split_once(':') {
            if section != "doc" && section != "activity" {
                ids.insert(id.to_string());
            }
        }
    }

    // An edge is a JSON array of source, target, kind and label, so changing one touches
    // both of its endpoints.
    for key in report.missing_edges.iter().chain(report.extra_edges.iter()) {
        if let Ok(parts) = serde_json::from_str::<Vec<String>>(key) {
            if let Some(source) = parts.first() {
                ids.insert(source.clone());
            }
            if let Some(target) = parts.get(1) {
                ids.insert(target.clone());
            }
        }
    }

    ids.into_iter().collect()
}

pub fn commit_json(commit: &Commit) -> Value {
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
    validate_name("project name", &body.name)?;
    let audit = AuditEntry {
        id: 0,
        project: body.name.clone(),
        at: now_seconds(),
        actor: "unknown".to_string(),
        action: "project.create".to_string(),
        subject: body.name.clone(),
        detail: "project created".to_string(),
    };
    let project = state
        .store
        .create_project(&body.name, Some(&audit))
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
    pub holder: Option<String>,
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
    validate_name("branch name", &body.branch)?;

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

    // Locks are enforced by default, not by opt-in. Every commit computes the elements it
    // would change and refuses to change any element with a live lease held by a different
    // holder. A missing holder is still checked: it cannot be the holder, so a live lease
    // on a touched element refuses it too. The authoritative check runs inside the store's
    // commit transaction, so a lock taken after this point cannot be bypassed.
    let now = now_seconds();
    let tip_hash = state
        .store
        .branch_tip(&project, &body.branch)
        .map_err(map_store_error)?;
    let touched = match tip_hash.as_deref() {
        Some(tip) => {
            let tip_model = load_model(&state, &project, tip)?;
            touched_elements(&tip_model, &root)
        }
        None => all_touched(&root),
    };
    let guard = CommitGuard {
        holder: body.holder.as_deref().unwrap_or(""),
        elements: &touched,
        now,
        expected_tip: tip_hash.as_deref(),
    };

    // Cheap refusal BEFORE anything is stored, so a rejected commit leaves no orphaned
    // blob behind. The store checks again inside its transaction, and THAT check is the
    // authority: this one exists to avoid writing bytes we already know will be refused.
    if let Some(holder) = body.holder.as_deref() {
        let held = state
            .store
            .holders_of(&project, &touched, now_seconds())
            .map_err(map_store_error)?;
        if let Some(blocked) = held.iter().find(|l| l.holder != holder) {
            return Err(ApiError::conflict(format!(
                "{} is locked by {} until {}; a caller who holds this lease must supply the holder field to proceed",
                blocked.element, blocked.holder, blocked.expires_at
            )));
        }
    }

    let okf_hash = state.store.put_blob(&bytes).map_err(map_store_error)?;
    // One call, one transaction: the parents come from the tip the store reads inside the
    // same lock that writes the commit, so two concurrent commits to one branch chain
    // instead of forking the history. The audit row rides the SAME transaction, so the
    // commit and its record of who made it succeed or fail together.
    let audit = AuditEntry {
        id: 0,
        project: project.clone(),
        at: now,
        actor: body.author.clone(),
        action: "commit.create".to_string(),
        subject: body.branch.clone(),
        detail: body.message.clone(),
    };
    let commit = commit_refusal_guard(
        &state,
        &project,
        &body.branch,
        &body.author,
        state.store.commit_model(
            &project,
            &body.branch,
            &okf_hash,
            &body.author,
            &body.message,
            Some(guard),
            Some(&audit),
        ),
    )?;
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
        .ok_or_else(|| {
            eprintln!("missing blob {}", commit.okf_hash);
            ApiError::internal("stored model is missing")
        })?;
    let value: Value = serde_json::from_slice(&bytes).map_err(|e| {
        eprintln!("stored model is not valid JSON: {}", e);
        ApiError::internal("stored model could not be read")
    })?;
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
    validate_name("branch name", &body.name)?;
    let audit = AuditEntry {
        id: 0,
        project: project.clone(),
        at: now_seconds(),
        actor: "unknown".to_string(),
        action: "branch.create".to_string(),
        subject: body.name.clone(),
        detail: format!("from {}", body.from),
    };
    state
        .store
        .create_branch(&project, &body.name, &body.from, Some(&audit))
        .map_err(map_store_error)?;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "name": body.name, "tip": body.from })),
    ))
}

pub async fn list_branches(
    State(state): State<ApiState>,
    Path(project): Path<String>,
) -> Result<Json<Value>, ApiError> {
    if state
        .store
        .project(&project)
        .map_err(map_store_error)?
        .is_none()
    {
        return Err(ApiError::not_found(format!("project {}", project)));
    }
    let branches = state
        .store
        .list_branches(&project)
        .map_err(map_store_error)?;
    let out: Vec<Value> = branches
        .iter()
        .map(|(name, tip)| json!({ "name": name, "tip": tip }))
        .collect();
    Ok(Json(Value::Array(out)))
}

pub async fn delete_branch(
    State(state): State<ApiState>,
    Path((project, name)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    validate_name("branch name", &name)?;
    let audit = AuditEntry {
        id: 0,
        project: project.clone(),
        at: now_seconds(),
        actor: "unknown".to_string(),
        action: "branch.delete".to_string(),
        subject: name.clone(),
        detail: "branch deleted".to_string(),
    };
    state
        .store
        .delete_branch(&project, &name, Some(&audit))
        .map_err(map_store_error)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct ResetBranch {
    pub to: String,
    pub author: String,
    pub message: String,
    /// Who is reverting. As with a commit, supplying the holder lets the lease holder
    /// proceed; omitting it means any live lease on a changed element refuses the reset.
    pub holder: Option<String>,
}

/// Restore a branch to the CONTENT of an earlier commit by appending a new commit. The
/// history is never rewritten: the old tip stays reachable, and the revert is itself a
/// commit with an author and a message.
pub async fn reset_branch(
    State(state): State<ApiState>,
    Path((project, name)): Path<(String, String)>,
    Json(body): Json<ResetBranch>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    validate_name("branch name", &name)?;
    if state
        .store
        .project(&project)
        .map_err(map_store_error)?
        .is_none()
    {
        return Err(ApiError::not_found(format!("project {}", project)));
    }
    let tip_hash = state
        .store
        .branch_tip(&project, &name)
        .map_err(map_store_error)?
        .ok_or_else(|| ApiError::not_found(format!("branch {}", name)))?;
    let target = state
        .store
        .commit(&project, &body.to)
        .map_err(map_store_error)?
        .ok_or_else(|| ApiError::not_found(format!("commit {}", body.to)))?;
    // A reset is a write path like any other: it refuses to change an element another
    // holder has locked. The touched set is the difference between the CURRENT tip model
    // and the TARGET model. A caller that holds the leases passes its holder and proceeds;
    // a caller that does not is refused, because an absent holder cannot be the holder.
    let tip_model = load_model(&state, &project, &tip_hash)?;
    let target_model = load_model(&state, &project, &body.to)?;
    let touched = touched_elements(&tip_model, &target_model);
    let guard = CommitGuard {
        holder: body.holder.as_deref().unwrap_or(""),
        elements: &touched,
        now: now_seconds(),
        expected_tip: Some(&tip_hash),
    };
    // The target's model is already stored, so this reuses its blob rather than copying
    // the bytes: the restored content is byte-identical to the original by construction.
    let audit = AuditEntry {
        id: 0,
        project: project.clone(),
        at: now_seconds(),
        actor: body.author.clone(),
        action: "branch.reset".to_string(),
        subject: name.clone(),
        detail: format!("reset to {}", body.to),
    };
    let commit = commit_refusal_guard(
        &state,
        &project,
        &name,
        &body.author,
        state.store.commit_model(
            &project,
            &name,
            &target.okf_hash,
            &body.author,
            &body.message,
            Some(guard),
            Some(&audit),
        ),
    )?;
    Ok((StatusCode::CREATED, Json(commit_json(&commit))))
}
