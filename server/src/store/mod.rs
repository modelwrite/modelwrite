// SPDX-License-Identifier: AGPL-3.0-or-later
use serde::Serialize;
use sha2::{Digest, Sha256};

pub mod sqlite;

#[derive(Debug, Clone, PartialEq)]
pub struct Project {
    pub name: String,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Commit {
    pub hash: String,
    pub project: String,
    pub branch: String,
    pub parents: Vec<String>,
    pub okf_hash: String,
    pub author: String,
    pub message: String,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GateRun {
    pub project: String,
    pub branch: String,
    pub reference_hash: String,
    pub candidate_hash: String,
    pub passed: bool,
    pub evidence: String,
    pub created_at: String,
}

/// A lease on one element. A lock always carries an expiry, so a crashed client cannot
/// block an element forever. The `branch` records where the holder intends to edit, while
/// the lock itself is keyed on the element: two branches cannot edit one element at once.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Lock {
    pub id: String,
    pub project: String,
    pub branch: String,
    pub element: String,
    pub holder: String,
    pub acquired_at: i64,
    pub expires_at: i64,
}

/// One entry in the append-only audit log. The `id` is assigned by the store on append
/// (callers pass 0); `at` is the wall-clock time supplied by the HTTP layer, never read
/// inside the store; `actor` is what the request SAID it was - its author or holder - not
/// a verified identity, which is a later tranche.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditEntry {
    pub id: i64,
    pub project: String,
    pub at: i64,
    pub actor: String,
    pub action: String,
    pub subject: String,
    pub detail: String,
}

#[derive(Debug)]
pub enum StoreError {
    NotFound(String),
    Conflict(String),
    Backend(String),
    /// A guarded write would change an element somebody else holds a live lease on.
    ///
    /// This is its own variant rather than a `Conflict` with particular wording, because
    /// the caller must tell a LOCKED refusal from a stale-tip refusal: the first is
    /// recorded as an attempt to overwrite someone's work, the second is an ordinary
    /// retry. Deciding that by searching the message for a phrase would make a record of
    /// who tried to overwrite whom depend on the wording of an error string.
    Locked {
        element: String,
        holder: String,
        expires_at: i64,
    },
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StoreError::NotFound(m) => write!(f, "not found: {}", m),
            StoreError::Conflict(m) => write!(f, "conflict: {}", m),
            StoreError::Backend(m) => write!(f, "storage error: {}", m),
            StoreError::Locked {
                element,
                holder,
                expires_at,
            } => write!(f, "{} is held by {} until {}", element, holder, expires_at),
        }
    }
}

impl std::error::Error for StoreError {}

/// Content address of a model: sha256 over the exact bytes stored. The same document
/// always has the same address, so a re-export that changed nothing is a no-op.
pub fn blob_hash(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

/// Content address of a commit: sha256 over a canonical JSON payload of everything
/// that defines it except time. Two identical commits made at different moments hash
/// the same, which is what makes a commit reproducible and its evidence citable.
pub fn commit_hash(
    project: &str,
    branch: &str,
    parents: &[String],
    okf_hash: &str,
    author: &str,
    message: &str,
) -> String {
    let mut sorted_parents: Vec<&str> = parents.iter().map(|p| p.as_str()).collect();
    sorted_parents.sort_unstable();
    let payload = serde_json::json!({
        "project": project,
        "branch": branch,
        "parents": sorted_parents,
        "okfHash": okf_hash,
        "author": author,
        "message": message
    });
    let bytes = serde_json::to_vec(&payload).expect("commit payload serializes");
    blob_hash(&bytes)
}

/// Deterministic identifier for a lock row: the project, branch, element and holder
/// scoped by the acquisition time, so tests can predict the id without a random source.
/// Truncated to 32 hex characters, ample for a lease measured in minutes.
fn lock_id(project: &str, branch: &str, element: &str, holder: &str, now: i64) -> String {
    let digest =
        Sha256::digest(format!("{}|{}|{}|{}|{}", project, branch, element, holder, now).as_bytes());
    let encoded = hex::encode(digest);
    encoded[..32].to_string()
}

/// Seconds since the epoch, stored beside a commit and never inside its hash.
pub fn now_epoch() -> String {
    match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => d.as_secs().to_string(),
        Err(_) => "0".to_string(),
    }
}

/// The real clock in seconds, for callers that pass time INTO the store (lock expiry,
/// audit timestamps). The store itself never calls this.
pub fn now_seconds() -> i64 {
    now_epoch().parse().unwrap_or(0)
}

/// What a guarded commit must not change. Any element in `elements` held by a DIFFERENT
/// holder with a live lease refuses the commit. The holder is the one asking; an empty
/// holder is a request that supplied none, so it can never be the holder and any live
/// lease on a touched element refuses it. Locks are therefore enforced by default: every
/// write path passes a guard, and a writer that omits the holder is refused, not let
/// through.
///
/// `expected_tip` is the branch tip the caller computed `elements` against. The guard is
/// only meaningful for that tip: if the branch moved since, the touched set describes a
/// document that is no longer there, and enforcing it would protect the wrong elements -
/// a race that a fresh check cannot catch from outside the transaction. The store refuses
/// instead, and the caller re-reads and retries.
///
/// Lock scope: a lock is keyed by project and element, NOT by branch. The branch is recorded
/// as context for whoever looks at the lock table, but the protection is deliberately
/// cross-branch - two people editing one element from two branches is exactly the overwrite
/// this exists to prevent.
pub struct CommitGuard<'a> {
    pub holder: &'a str,
    pub elements: &'a [String],
    pub now: i64,
    pub expected_tip: Option<&'a str>,
}

/// The refusal every guarded write path produces when it would change a locked element.
/// It names the element, the holder and the lease expiry, and tells a caller who IS the
/// holder to supply the holder field: a request without a holder cannot be the holder, so
/// a live lease on a touched element refuses it too.
pub fn lock_refusal(element: &str, holder: &str, expires_at: i64) -> StoreError {
    StoreError::Locked {
        element: element.to_string(),
        holder: holder.to_string(),
        expires_at,
    }
}

/// True when the store refused because somebody else holds the element.
pub fn is_lock_refusal(error: &StoreError) -> bool {
    matches!(error, StoreError::Locked { .. })
}

pub trait Store: Send + Sync {
    /// Create a project and, when `audit` is supplied, write its audit row inside the
    /// same transaction, so a durable project always has a record and vice versa.
    fn create_project(&self, name: &str, audit: Option<&AuditEntry>)
        -> Result<Project, StoreError>;
    fn project(&self, name: &str) -> Result<Option<Project>, StoreError>;
    fn list_projects(&self) -> Result<Vec<Project>, StoreError>;
    fn put_blob(&self, bytes: &[u8]) -> Result<String, StoreError>;
    fn blob(&self, hash: &str) -> Result<Option<Vec<u8>>, StoreError>;

    /// Commit a model onto a branch atomically: the tip is read, the parents and the
    /// commit hash are derived from it, and the commit row and the branch tip are written
    /// inside ONE lock and transaction. Resolving the parents a layer above would be a
    /// read-modify-write race: two concurrent commits would both read the same tip and
    /// fork the history, the later one orphaning the earlier while the branch lists both.
    ///
    /// An optional guard says "refuse this commit if any of these elements is locked by
    /// somebody else". The check runs INSIDE the transaction, not before it: checking first
    /// and writing afterwards leaves a window in which another holder acquires the lock and
    /// the guarded commit lands anyway, which would make a lock advisory in the worst way -
    /// it would look enforced and not be.
    #[allow(clippy::too_many_arguments)]
    fn commit_model(
        &self,
        project: &str,
        branch: &str,
        okf_hash: &str,
        author: &str,
        message: &str,
        guard: Option<CommitGuard<'_>>,
        audit: Option<&AuditEntry>,
    ) -> Result<Commit, StoreError>;

    /// Write a commit with EXPLICIT parents and move the branch tip, in one transaction.
    /// A merge commit has two parents, so the parent list cannot be derived from the tip.
    /// The optional guard is checked inside that transaction, exactly as `commit_model`
    /// does: a merge that would change a locked element is refused rather than overwriting
    /// the holder's work.
    #[allow(clippy::too_many_arguments)]
    fn commit_merge(
        &self,
        project: &str,
        branch: &str,
        parents: &[String],
        okf_hash: &str,
        author: &str,
        message: &str,
        guard: Option<CommitGuard<'_>>,
        audit: Option<&AuditEntry>,
    ) -> Result<Commit, StoreError>;

    fn commit(&self, project: &str, hash: &str) -> Result<Option<Commit>, StoreError>;
    fn commits_on(&self, project: &str, branch: &str) -> Result<Vec<Commit>, StoreError>;
    fn branch_tip(&self, project: &str, branch: &str) -> Result<Option<String>, StoreError>;
    fn create_branch(
        &self,
        project: &str,
        name: &str,
        from: &str,
        audit: Option<&AuditEntry>,
    ) -> Result<(), StoreError>;

    /// Remove a branch pointer. This never deletes commits: the objects a branch pointed
    /// at stay in the store, so a deleted branch can be recreated at the same hash and no
    /// history is ever lost. The audit row, when supplied, is written in the same
    /// transaction as the deletion.
    fn delete_branch(
        &self,
        project: &str,
        name: &str,
        audit: Option<&AuditEntry>,
    ) -> Result<(), StoreError>;
    fn list_branches(&self, project: &str) -> Result<Vec<(String, String)>, StoreError>;
    fn record_gate_run(&self, run: &GateRun, audit: Option<&AuditEntry>) -> Result<(), StoreError>;
    fn gate_runs(&self, project: &str) -> Result<Vec<GateRun>, StoreError>;

    /// Acquire a lease on each of `elements`, all or nothing. If any element is held by a
    /// live lease owned by a DIFFERENT holder, nothing is acquired and a Conflict is
    /// returned naming the holder and the expiry. Re-acquiring an element the same holder
    /// already holds extends the lease rather than failing.
    #[allow(clippy::too_many_arguments)]
    fn acquire_locks(
        &self,
        project: &str,
        branch: &str,
        elements: &[String],
        holder: &str,
        ttl_seconds: i64,
        now: i64,
        audit: Option<&AuditEntry>,
    ) -> Result<Vec<Lock>, StoreError>;

    /// Release locks by id; only the listed holder may release them. Returns how many
    /// rows were actually removed. The audit row, when supplied, is written inside the
    /// same transaction, and only when at least one row was actually removed: a no-op
    /// release is not a mutation and writes no entry.
    fn release_locks(
        &self,
        project: &str,
        holder: &str,
        ids: &[String],
        audit: Option<&AuditEntry>,
    ) -> Result<usize, StoreError>;

    /// Live locks on a project: expired rows are reported as gone.
    fn locks(&self, project: &str, now: i64) -> Result<Vec<Lock>, StoreError>;

    /// The live locks covering any of the given elements, used by the commit path to see
    /// who holds an element before changing it.
    fn holders_of(
        &self,
        project: &str,
        elements: &[String],
        now: i64,
    ) -> Result<Vec<Lock>, StoreError>;

    /// Append one entry to the audit log and return its row id. There is deliberately NO
    /// update, delete or truncate method anywhere in this trait: the log is append-only by
    /// construction rather than by convention, so a written entry can never be rewritten.
    fn append_audit(&self, entry: &AuditEntry) -> Result<i64, StoreError>;

    /// The audit entries for a project, newest first, at most `limit` rows (capped at 1000).
    fn audit(&self, project: &str, limit: i64) -> Result<Vec<AuditEntry>, StoreError>;
}
