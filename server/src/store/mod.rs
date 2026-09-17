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

#[derive(Debug)]
pub enum StoreError {
    NotFound(String),
    Conflict(String),
    Backend(String),
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StoreError::NotFound(m) => write!(f, "not found: {}", m),
            StoreError::Conflict(m) => write!(f, "conflict: {}", m),
            StoreError::Backend(m) => write!(f, "storage error: {}", m),
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

pub trait Store: Send + Sync {
    fn create_project(&self, name: &str) -> Result<Project, StoreError>;
    fn project(&self, name: &str) -> Result<Option<Project>, StoreError>;
    fn list_projects(&self) -> Result<Vec<Project>, StoreError>;
    fn put_blob(&self, bytes: &[u8]) -> Result<String, StoreError>;
    fn blob(&self, hash: &str) -> Result<Option<Vec<u8>>, StoreError>;

    /// Commit a model onto a branch atomically: the tip is read, the parents and the
    /// commit hash are derived from it, and the commit row and the branch tip are written
    /// inside ONE lock and transaction. Resolving the parents a layer above would be a
    /// read-modify-write race: two concurrent commits would both read the same tip and
    /// fork the history, the later one orphaning the earlier while the branch lists both.
    fn commit_model(
        &self,
        project: &str,
        branch: &str,
        okf_hash: &str,
        author: &str,
        message: &str,
    ) -> Result<Commit, StoreError>;

    /// Write a commit with EXPLICIT parents and move the branch tip, in one transaction.
    /// A merge commit has two parents, so the parent list cannot be derived from the tip.
    fn commit_merge(
        &self,
        project: &str,
        branch: &str,
        parents: &[String],
        okf_hash: &str,
        author: &str,
        message: &str,
    ) -> Result<Commit, StoreError>;

    fn commit(&self, project: &str, hash: &str) -> Result<Option<Commit>, StoreError>;
    fn commits_on(&self, project: &str, branch: &str) -> Result<Vec<Commit>, StoreError>;
    fn branch_tip(&self, project: &str, branch: &str) -> Result<Option<String>, StoreError>;
    fn create_branch(&self, project: &str, name: &str, from: &str) -> Result<(), StoreError>;

    /// Remove a branch pointer. This never deletes commits: the objects a branch pointed
    /// at stay in the store, so a deleted branch can be recreated at the same hash and no
    /// history is ever lost.
    fn delete_branch(&self, project: &str, name: &str) -> Result<(), StoreError>;
    fn list_branches(&self, project: &str) -> Result<Vec<(String, String)>, StoreError>;
    fn record_gate_run(&self, run: &GateRun) -> Result<(), StoreError>;
    fn gate_runs(&self, project: &str) -> Result<Vec<GateRun>, StoreError>;

    /// Acquire a lease on each of `elements`, all or nothing. If any element is held by a
    /// live lease owned by a DIFFERENT holder, nothing is acquired and a Conflict is
    /// returned naming the holder and the expiry. Re-acquiring an element the same holder
    /// already holds extends the lease rather than failing.
    fn acquire_locks(
        &self,
        project: &str,
        branch: &str,
        elements: &[String],
        holder: &str,
        ttl_seconds: i64,
        now: i64,
    ) -> Result<Vec<Lock>, StoreError>;

    /// Release locks by id; only the listed holder may release them. Returns how many
    /// rows were actually removed.
    fn release_locks(
        &self,
        project: &str,
        holder: &str,
        ids: &[String],
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
}
