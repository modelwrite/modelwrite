// SPDX-License-Identifier: AGPL-3.0-or-later
use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::audit::{
    BRANCH_CREATE, BRANCH_DELETE, BRANCH_RESET, COMMIT_CREATE, COMMIT_REFUSED, PROJECT_CREATE,
};
use crate::auth::{AuthConfig, Identity, Permission};
use crate::error::ApiError;
use crate::store::{
    is_lock_refusal, now_epoch, AcceptanceProvenance, AuditEntry, Commit, CommitGuard,
    ImportProvenance, Project, Store, StoreError,
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

/// Validate and deduplicate the element list for a lock acquisition, so the HTTP handler and
/// the offline CLI enforce the SAME rules from one place. A repeated element would otherwise
/// acquire one lease but report two. Returns the deduplicated list in its original order.
pub fn prepare_lock_elements(elements: Vec<String>) -> Result<Vec<String>, ApiError> {
    if elements.is_empty() {
        return Err(ApiError::bad_request("at least one element is required"));
    }
    for element in &elements {
        validate_element_name(element)?;
    }
    let mut seen = std::collections::BTreeSet::new();
    Ok(elements
        .into_iter()
        .filter(|e| seen.insert(e.clone()))
        .collect())
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
        } => ApiError::conflict(lock_refusal_message(&element, &holder, expires_at)),
        StoreError::Backend(m) => {
            eprintln!("storage error: {}", m);
            ApiError::internal("internal storage error")
        }
    }
}

/// The single, human-facing wording for a write refused by a live lease held by another
/// holder. Used by the HTTP layer's error mapping AND by the offline CLI, so the message the
/// CLI prints is, by construction, the exact message the server returns.
pub fn lock_refusal_message(element: &str, holder: &str, expires_at: i64) -> String {
    format!(
        "{} is locked by {} until {}; a caller who holds this lease must supply the holder field to proceed",
        element, holder, expires_at
    )
}

/// Load the OKF document behind a commit hash. Shared by the HTTP handlers and the offline
/// CLI: it takes the store, not the full request state, so the two paths cannot drift.
pub fn load_model(
    store: &dyn Store,
    project: &str,
    hash: &str,
) -> Result<okf::types::OkfRoot, StoreError> {
    let commit = store
        .commit(project, hash)?
        .ok_or_else(|| StoreError::NotFound(format!("commit {}", hash)))?;
    let bytes = store.blob(&commit.okf_hash)?.ok_or_else(|| {
        eprintln!("missing blob {}", commit.okf_hash);
        StoreError::Backend("the stored model is missing".to_string())
    })?;
    serde_json::from_slice(&bytes).map_err(|e| {
        eprintln!("stored model is not readable: {}", e);
        StoreError::Backend("the stored model could not be read".to_string())
    })
}

/// The real clock in seconds, as the store requires it. The store itself never reads the
/// clock: time is passed in so lock expiry is testable without sleeping.
fn now_seconds() -> i64 {
    now_epoch().parse().unwrap_or(0)
}

/// The author a commit records. In configured mode an empty (or absent) author defaults to
/// the verified subject, and a non-empty author must equal it or the request is refused with
/// 403 BEFORE anything is written - so a caller can never put another person's name into the
/// record, and a blank author can never contradict the audit's verified subject. In open mode
/// nobody was authenticated, so no name can be "someone else's" and the claimed name passes
/// through unchanged.
pub fn resolve_author(
    auth: &AuthConfig,
    identity: &Identity,
    claimed: &str,
) -> Result<String, ApiError> {
    if matches!(auth, AuthConfig::Open) {
        return Ok(claimed.to_string());
    }
    if claimed.is_empty() {
        return Ok(identity.subject.clone());
    }
    if claimed == identity.subject.as_str() {
        return Ok(claimed.to_string());
    }
    Err(ApiError::forbidden(
        "the request names an actor other than the authenticated caller",
    ))
}

/// A body may name an actor in a `holder` field. Once authentication is configured that
/// claimed name must agree with the verified identity, or the request is refused with 403
/// BEFORE anything is written, so a caller can never put another person's name into the
/// record. A missing or empty name proceeds - it names nobody. In open mode nobody was
/// authenticated, so no name can be "someone else's" and the check is skipped.
pub fn verify_actor(
    auth: &AuthConfig,
    identity: &Identity,
    claimed: Option<&str>,
) -> Result<(), ApiError> {
    match claimed {
        Some(name) => resolve_author(auth, identity, name).map(|_| ()),
        None => Ok(()),
    }
}

/// Record a refusal - an action that was ATTEMPTED but refused - in the audit log. A
/// refusal is not a mutation, so it appends directly rather than riding a transaction; the
/// event matters even though nothing changed.
#[allow(clippy::too_many_arguments)]
pub fn record_refusal(
    store: &dyn Store,
    project: &str,
    actor: &str,
    mechanism: &str,
    authorizer: &str,
    action: &str,
    subject: &str,
    detail: &str,
) -> Result<(), StoreError> {
    store
        .append_audit(&AuditEntry {
            id: 0,
            project: project.to_string(),
            at: now_seconds(),
            actor: actor.to_string(),
            mechanism: mechanism.to_string(),
            authorizer: authorizer.to_string(),
            action: action.to_string(),
            subject: subject.to_string(),
            detail: detail.to_string(),
        })
        .map(|_| ())
}

/// Call a commit-producing store method and, if it is refused because an element is
/// locked, record commit.refused before surfacing the 409. A lock refusal is the
/// highest-value event this feature produces: it is the overwrite the lock prevented. The
/// `actor` is the verified identity's subject - a refusal is an audit entry and must carry
/// the same verified actor as every other entry.
///
/// This six-argument form is kept for the offline CLI, which has no authorizer: the refusal
/// is attributed to the actor alone. Server write paths call
/// [`commit_refusal_guard_attributed`] so an agent's refusal also names the human it acted
/// for.
pub fn commit_refusal_guard(
    store: &dyn Store,
    project: &str,
    branch: &str,
    actor: &str,
    mechanism: &str,
    result: Result<Commit, StoreError>,
) -> Result<Commit, StoreError> {
    commit_refusal_guard_attributed(store, project, branch, actor, mechanism, "", result)
}

/// The attributed form of [`commit_refusal_guard`]: the authorizer (empty for a human) is
/// recorded on the refusal, so a reader can tell an agent's refused action from a human's
/// and know who the agent acted on behalf of.
pub fn commit_refusal_guard_attributed(
    store: &dyn Store,
    project: &str,
    branch: &str,
    actor: &str,
    mechanism: &str,
    authorizer: &str,
    result: Result<Commit, StoreError>,
) -> Result<Commit, StoreError> {
    match result {
        Ok(commit) => Ok(commit),
        Err(error) if is_lock_refusal(&error) => {
            let detail = error.to_string();
            // Recording the refusal must never change the answer. If the log cannot be
            // written, that is a problem with the log, not with the caller: the write was
            // still correctly refused, and reporting 500 would tell the caller their commit
            // failed for an unrelated reason and invite a retry that cannot succeed.
            if let Err(recording) = record_refusal(
                store,
                project,
                actor,
                mechanism,
                authorizer,
                COMMIT_REFUSED,
                branch,
                &detail,
            ) {
                eprintln!("could not record the refusal: {:?}", recording);
            }
            Err(error)
        }
        Err(error) => Err(error),
    }
}

/// Every element an incoming document names, used when a branch has no tip yet.
///
/// A first commit has nothing to diff against, and diffing a document against itself
/// yields an EMPTY touched set - so a lease taken before the branch existed would not be
/// enforced. Treating "no tip" as "an empty document" gives the honest answer: every
/// element this commit introduces is an element it changes.
pub fn all_touched(root: &okf::types::OkfRoot) -> Vec<String> {
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
        "createdAt": commit.created_at,
        "provenance": commit.provenance
    })
}

/// The elements a commit to this base would touch. A first commit (`reference` is `None`)
/// has nothing to diff against, so it touches every element the document names; a later
/// commit touches the difference between the tip model and the candidate. This is the ONE
/// place a commit decides what "changed" means, shared by the JSON handler, the editor, and
/// any future write path.
pub fn commit_touched(
    reference: Option<&okf::types::OkfRoot>,
    candidate: &okf::types::OkfRoot,
) -> Vec<String> {
    match reference {
        Some(reference) => touched_elements(reference, candidate),
        None => all_touched(candidate),
    }
}

/// What a commit attempt produced. A validation failure is NOT a storage failure: the JSON
/// handler renders it as 422 and the editor as its form errors, but the decision of what
/// counts as an invalid document lives in one place.
pub enum CommitFailure {
    Invalid { errors: Vec<String> },
    Store(StoreError),
}

/// Everything the commit core needs that the caller resolved upstream. `author` and
/// `actor` are already resolved against the identity; `bytes` are the exact bytes to
/// store (the JSON handler stores the client's document, the editor stores its re-serialised
/// model); `tip` and `reference` are the tip the candidate was computed against and that
/// tip's model, both `None` for a first commit; `import` is the provenance an import
/// commit carries, `None` for every other commit.
pub struct CommitCore<'a> {
    pub project: &'a str,
    pub branch: &'a str,
    pub author: &'a str,
    pub message: &'a str,
    pub actor: &'a str,
    pub mechanism: &'a str,
    pub authorizer: &'a str,
    pub candidate: &'a okf::types::OkfRoot,
    pub bytes: &'a [u8],
    pub import: Option<&'a ImportProvenance>,
    /// Some when this commit is a human's acceptance of an agent's MODEL-CHANGE proposal.
    /// Mutually exclusive with `import`: a commit is either an import (with its retained
    /// artifact and binding) or an acceptance of a model change (with its proposal), never
    /// both. The commit's provenance is then [CommitProvenance::Accepted], and the proposal
    /// record is updated inside the same store transaction as the commit row.
    pub acceptance: Option<&'a AcceptanceProvenance>,
    /// The guard's holder: the identity that may change the touched elements. An empty
    /// string means the caller holds no lease, so any live lease on a touched element
    /// refuses the commit. The editor passes its verified subject because it acquired a
    /// request-duration lease on the element before committing; a plain commit passes the
    /// optional holder field (or an empty string).
    pub holder: &'a str,
    pub now: i64,
    pub tip: Option<&'a str>,
    pub reference: Option<&'a okf::types::OkfRoot>,
}

/// The ONE implementation of a commit: validate the document, store its bytes, compute the
/// touched set, build the lock guard against the expected tip, and write the commit and its
/// audit row in one store transaction, recording a refusal if the guard refuses. Both
/// [create_commit] and the editor's `perform_edit` call this, so they cannot diverge on
/// the sequence, the guard or the audit entry.
pub fn commit_core(store: &dyn Store, input: &CommitCore<'_>) -> Result<Commit, CommitFailure> {
    // Every commit path validates before it stores. A future rule that produced a
    // self-contradicting document turns a silent bad write into a loud refusal here.
    let report = okf::validate::validate(input.candidate);
    if !report.valid {
        return Err(CommitFailure::Invalid {
            errors: report.errors,
        });
    }

    let touched = commit_touched(input.reference, input.candidate);
    let guard = CommitGuard {
        holder: input.holder,
        elements: &touched,
        now: input.now,
        expected_tip: input.tip,
    };

    let okf_hash = store.put_blob(input.bytes).map_err(CommitFailure::Store)?;
    let audit = AuditEntry {
        id: 0,
        project: input.project.to_string(),
        at: input.now,
        actor: input.actor.to_string(),
        mechanism: input.mechanism.to_string(),
        authorizer: input.authorizer.to_string(),
        action: COMMIT_CREATE.to_string(),
        subject: input.branch.to_string(),
        detail: input.message.to_string(),
    };
    // The store write is selected from the ONE provenance input: an import commits through
    // commit_model (whose transaction substantiates the import), an acceptance of a model
    // change commits through commit_accepted (whose transaction substantiates the proposal),
    // and an ordinary commit commits through commit_model with no import. The two are
    // mutually exclusive by construction; a caller that supplies both is a bug, refused here
    // rather than silently labelled.
    let commit_result = match (input.import, input.acceptance) {
        (Some(_), Some(_)) => Err(StoreError::Backend(
            "a commit cannot be both an import and a model-change acceptance".to_string(),
        )),
        (Some(import), None) => store.commit_model(
            input.project,
            input.branch,
            &okf_hash,
            input.author,
            input.message,
            Some(guard),
            Some(&audit),
            Some(import),
        ),
        (None, Some(acceptance)) => store.commit_accepted(
            input.project,
            input.branch,
            &okf_hash,
            input.author,
            input.message,
            Some(guard),
            Some(&audit),
            acceptance,
        ),
        (None, None) => store.commit_model(
            input.project,
            input.branch,
            &okf_hash,
            input.author,
            input.message,
            Some(guard),
            Some(&audit),
            None,
        ),
    };
    commit_refusal_guard_attributed(
        store,
        input.project,
        input.branch,
        input.actor,
        input.mechanism,
        input.authorizer,
        commit_result,
    )
    .map_err(CommitFailure::Store)
}

#[derive(Deserialize)]
pub struct CreateProject {
    pub name: String,
}

/// The ONE implementation of creating a project: build the audit entry and call the store,
/// shared by the JSON handler and the workbench form so they cannot diverge on the sequence
/// or the audit record. The permission and project-scope decisions stay in the callers, in
/// the same order; this core only does what every create must do.
pub fn create_project_core(
    store: &dyn Store,
    actor: &str,
    mechanism: &str,
    authorizer: &str,
    name: &str,
) -> Result<Project, StoreError> {
    let audit = AuditEntry {
        id: 0,
        project: name.to_string(),
        at: now_seconds(),
        actor: actor.to_string(),
        mechanism: mechanism.to_string(),
        authorizer: authorizer.to_string(),
        action: PROJECT_CREATE.to_string(),
        subject: name.to_string(),
        detail: "project created".to_string(),
    };
    store.create_project(name, Some(&audit))
}

pub async fn create_project(
    identity: Identity,
    State(state): State<ApiState>,
    Json(body): Json<CreateProject>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    if !identity.may(Permission::Administer) {
        return Err(ApiError::forbidden("admin permission required"));
    }
    if !identity.may_reach(&body.name) {
        return Err(ApiError::forbidden("project not in scope"));
    }
    validate_name("project name", &body.name)?;
    let project = create_project_core(
        state.store.as_ref(),
        &identity.subject,
        state.auth.mechanism(),
        state.auth.authorizer().unwrap_or(""),
        &body.name,
    )
    .map_err(map_store_error)?;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "name": project.name, "createdAt": project.created_at })),
    ))
}

pub async fn list_projects(
    identity: Identity,
    State(state): State<ApiState>,
) -> Result<Json<Value>, ApiError> {
    if !identity.may(Permission::Read) {
        return Err(ApiError::forbidden("read permission required"));
    }
    let projects = state.store.list_projects().map_err(map_store_error)?;
    // A caller scoped to particular projects must not learn the names of the others. The
    // listing is filtered rather than refused: an identity that reaches nothing gets an
    // empty list, which is the truthful answer to "what may I see".
    let out: Vec<Value> = projects
        .iter()
        .filter(|p| identity.may_reach(&p.name))
        .map(|p| json!({ "name": p.name, "createdAt": p.created_at }))
        .collect();
    Ok(Json(Value::Array(out)))
}

#[derive(Deserialize)]
pub struct CreateCommit {
    pub branch: String,
    #[serde(default)]
    pub author: String,
    pub message: String,
    pub okf: Value,
    pub holder: Option<String>,
}

pub async fn create_commit(
    identity: Identity,
    State(state): State<ApiState>,
    Path(project): Path<String>,
    Json(body): Json<CreateCommit>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    if !identity.may(Permission::Write) {
        return Err(ApiError::forbidden("write permission required"));
    }
    if !identity.may_reach(&project) {
        return Err(ApiError::forbidden("project not in scope"));
    }
    // In configured mode an empty author is the verified subject, never a blank string that
    // would contradict the audit's actor.
    let author = resolve_author(&state.auth, &identity, &body.author)?;
    verify_actor(&state.auth, &identity, body.holder.as_deref())?;
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
    let reference: Option<okf::types::OkfRoot> = match tip_hash.as_deref() {
        Some(tip) => {
            Some(load_model(state.store.as_ref(), &project, tip).map_err(map_store_error)?)
        }
        None => None,
    };

    // Cheap refusal BEFORE anything is stored, so a rejected commit leaves no orphaned
    // blob behind. The store checks again inside its transaction, and THAT check is the
    // authority: this one exists to avoid writing bytes we already know will be refused.
    if let Some(holder) = body.holder.as_deref() {
        let touched = commit_touched(reference.as_ref(), &root);
        let held = state
            .store
            .holders_of(&project, &touched, now_seconds())
            .map_err(map_store_error)?;
        if let Some(blocked) = held.iter().find(|l| l.holder != holder) {
            return Err(ApiError::conflict(lock_refusal_message(
                &blocked.element,
                &blocked.holder,
                blocked.expires_at,
            )));
        }
    }

    // The commit itself is the SAME sequence the editor runs: validate, store the bytes,
    // compute the touched set, guard against the expected tip, and write the commit and its
    // audit row in one transaction. Keeping the validation above is deliberate: it refuses
    // an invalid document BEFORE any store read, exactly as before.
    let commit = commit_core(
        state.store.as_ref(),
        &CommitCore {
            project: &project,
            branch: &body.branch,
            author: &author,
            message: &body.message,
            actor: &identity.subject,
            mechanism: state.auth.mechanism(),
            authorizer: state.auth.authorizer().unwrap_or(""),
            candidate: &root,
            bytes: &bytes,
            import: None,
            acceptance: None,
            holder: body.holder.as_deref().unwrap_or(""),
            now,
            tip: tip_hash.as_deref(),
            reference: reference.as_ref(),
        },
    )
    .map_err(|failure| match failure {
        CommitFailure::Invalid { errors } => {
            ApiError::unprocessable("the model failed validation", errors)
        }
        CommitFailure::Store(error) => map_store_error(error),
    })?;
    Ok((StatusCode::CREATED, Json(commit_json(&commit))))
}

#[derive(Deserialize)]
pub struct BranchQuery {
    pub branch: Option<String>,
}

pub async fn list_commits(
    identity: Identity,
    State(state): State<ApiState>,
    Path(project): Path<String>,
    Query(query): Query<BranchQuery>,
) -> Result<Json<Value>, ApiError> {
    if !identity.may(Permission::Read) {
        return Err(ApiError::forbidden("read permission required"));
    }
    if !identity.may_reach(&project) {
        return Err(ApiError::forbidden("project not in scope"));
    }
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
    identity: Identity,
    State(state): State<ApiState>,
    Path((project, hash)): Path<(String, String)>,
) -> Result<Json<Value>, ApiError> {
    if !identity.may(Permission::Read) {
        return Err(ApiError::forbidden("read permission required"));
    }
    if !identity.may_reach(&project) {
        return Err(ApiError::forbidden("project not in scope"));
    }
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

/// GET /projects/:project/commits/:hash/record - the commit RECORD for one hash, including
/// its provenance.
///
/// get_commit returns the OKF model (the document), which is what editors and clients
/// consume; it does not carry provenance. This route returns the commit row itself - hash,
/// parents, author, message and the provenance saying authored, imported or unknown - so a
/// reader can check a commit's provenance FROM THE HASH ALONE without listing a whole branch
/// to find it. It is additive: the model response is unchanged.
pub async fn get_commit_record(
    identity: Identity,
    State(state): State<ApiState>,
    Path((project, hash)): Path<(String, String)>,
) -> Result<Json<Value>, ApiError> {
    if !identity.may(Permission::Read) {
        return Err(ApiError::forbidden("read permission required"));
    }
    if !identity.may_reach(&project) {
        return Err(ApiError::forbidden("project not in scope"));
    }
    let commit = state
        .store
        .commit(&project, &hash)
        .map_err(map_store_error)?
        .ok_or_else(|| ApiError::not_found(format!("commit {}", hash)))?;
    Ok(Json(commit_json(&commit)))
}

#[derive(Deserialize)]
pub struct CreateBranch {
    pub name: String,
    pub from: String,
}

/// The ONE implementation of creating a branch: build the audit entry and call the store,
/// shared by the JSON handler and the workbench form so they cannot diverge on the sequence
/// or the audit record. The permission and project-scope decisions stay in the callers, in
/// the same order; this core only does what every branch-create must do.
pub fn create_branch_core(
    store: &dyn Store,
    project: &str,
    actor: &str,
    mechanism: &str,
    authorizer: &str,
    name: &str,
    from: &str,
) -> Result<(), StoreError> {
    let audit = AuditEntry {
        id: 0,
        project: project.to_string(),
        at: now_seconds(),
        actor: actor.to_string(),
        mechanism: mechanism.to_string(),
        authorizer: authorizer.to_string(),
        action: BRANCH_CREATE.to_string(),
        subject: name.to_string(),
        detail: format!("from {}", from),
    };
    store.create_branch(project, name, from, Some(&audit))
}

pub async fn create_branch(
    identity: Identity,
    State(state): State<ApiState>,
    Path(project): Path<String>,
    Json(body): Json<CreateBranch>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    if !identity.may(Permission::Write) {
        return Err(ApiError::forbidden("write permission required"));
    }
    if !identity.may_reach(&project) {
        return Err(ApiError::forbidden("project not in scope"));
    }
    validate_name("branch name", &body.name)?;
    create_branch_core(
        state.store.as_ref(),
        &project,
        &identity.subject,
        state.auth.mechanism(),
        state.auth.authorizer().unwrap_or(""),
        &body.name,
        &body.from,
    )
    .map_err(map_store_error)?;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "name": body.name, "tip": body.from })),
    ))
}

pub async fn list_branches(
    identity: Identity,
    State(state): State<ApiState>,
    Path(project): Path<String>,
) -> Result<Json<Value>, ApiError> {
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
    identity: Identity,
    State(state): State<ApiState>,
    Path((project, name)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    if !identity.may(Permission::Administer) {
        return Err(ApiError::forbidden("admin permission required"));
    }
    if !identity.may_reach(&project) {
        return Err(ApiError::forbidden("project not in scope"));
    }
    validate_name("branch name", &name)?;
    let audit = AuditEntry {
        id: 0,
        project: project.clone(),
        at: now_seconds(),
        actor: identity.subject.clone(),
        mechanism: state.auth.mechanism().to_string(),
        authorizer: state.auth.authorizer().unwrap_or("").to_string(),
        action: BRANCH_DELETE.to_string(),
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
    #[serde(default)]
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
    identity: Identity,
    State(state): State<ApiState>,
    Path((project, name)): Path<(String, String)>,
    Json(body): Json<ResetBranch>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    if !identity.may(Permission::Write) {
        return Err(ApiError::forbidden("write permission required"));
    }
    if !identity.may_reach(&project) {
        return Err(ApiError::forbidden("project not in scope"));
    }
    let author = resolve_author(&state.auth, &identity, &body.author)?;
    verify_actor(&state.auth, &identity, body.holder.as_deref())?;
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
    let tip_model =
        load_model(state.store.as_ref(), &project, &tip_hash).map_err(map_store_error)?;
    let target_model =
        load_model(state.store.as_ref(), &project, &body.to).map_err(map_store_error)?;
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
        actor: identity.subject.clone(),
        mechanism: state.auth.mechanism().to_string(),
        authorizer: state.auth.authorizer().unwrap_or("").to_string(),
        action: BRANCH_RESET.to_string(),
        subject: name.clone(),
        detail: format!("reset to {}", body.to),
    };
    let commit = commit_refusal_guard_attributed(
        state.store.as_ref(),
        &project,
        &name,
        &identity.subject,
        state.auth.mechanism(),
        state.auth.authorizer().unwrap_or(""),
        state.store.commit_model(
            &project,
            &name,
            &target.okf_hash,
            &author,
            &body.message,
            Some(guard),
            Some(&audit),
            None,
        ),
    )
    .map_err(map_store_error)?;
    Ok((StatusCode::CREATED, Json(commit_json(&commit))))
}
