// SPDX-License-Identifier: AGPL-3.0-or-later
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[cfg(feature = "postgres")]
pub mod postgres;
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
    /// How this commit was produced. Authored and Imported are written in the SAME
    /// transaction as the commit; Unknown is only ever read back from a commit whose
    /// provenance column was never written (a commit made before provenance existed), so
    /// absence is never upgraded to a claim that the commit was authored.
    pub provenance: CommitProvenance,
}

/// How a commit was produced. A reader must be able to tell, from the commit alone, whether
/// it was authored (a normal commit or edit), imported (a migration read from a retained
/// artifact through a binding), accepted (an agent's proposal a human accepted, naming both
/// parties), or unknown (a commit written before provenance existed). The four are
/// deliberately distinct, and absence is never a claim: a commit with no recorded provenance
/// reads as Unknown, never as Authored.
///
/// The serialized form is the JSON a reader sees - a kind field plus, for an import, the
/// artifact hash, binding id and version and the accepted losses, and for an acceptance the
/// proposal id, the agent and the accepting human - so the API's commit_json and the store's
/// commits.provenance column can never disagree on the shape.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum CommitProvenance {
    /// A normal commit or edit, made through a write path that is not an import.
    Authored,
    /// A migration read from a retained artifact through a binding.
    Imported {
        #[serde(rename = "artifactHash")]
        artifact_hash: String,
        #[serde(rename = "bindingId")]
        binding_id: String,
        #[serde(rename = "bindingVersion")]
        binding_version: String,
        #[serde(rename = "acceptedLosses")]
        accepted_losses: Vec<String>,
    },
    /// A migration whose blocking losses were accepted by a human acting on an agent's
    /// proposal. The record names both parties - the agent that proposed and the human that
    /// accepted - plus the accepted items, so a reader a year later can see that a machine
    /// drafted this and a person decided it, and which person.
    Accepted {
        #[serde(rename = "proposalId")]
        proposal_id: String,
        /// The agent whose proposal was accepted.
        agent: String,
        #[serde(rename = "acceptedBy")]
        accepted_by: String,
        #[serde(rename = "acceptedItems")]
        accepted_items: Vec<String>,
    },
    /// A commit written before provenance was recorded: how it arrived was never checked.
    Unknown,
}

impl CommitProvenance {
    /// Read the stored commits.provenance column. A NULL column - a commit written before
    /// provenance existed - reads as Unknown, so absence is never mistaken for a claim that
    /// the commit was authored. A column that cannot be parsed is corruption, reported as a
    /// storage error rather than silently read as Unknown.
    pub fn parse_column(raw: Option<&str>) -> Result<Self, StoreError> {
        match raw {
            None => Ok(CommitProvenance::Unknown),
            Some(raw) => serde_json::from_str(raw)
                .map_err(|e| StoreError::Backend(format!("corrupt provenance column: {}", e))),
        }
    }

    /// The value to store in commits.provenance when a commit is being written now. Unknown
    /// is never written - it exists only as the read-back of an absent value - so it maps to
    /// None (a NULL column), while authored and imported serialize to their JSON.
    pub fn column_value(&self) -> Option<String> {
        match self {
            CommitProvenance::Unknown => None,
            _ => Some(serde_json::to_string(self).expect("provenance serializes")),
        }
    }
}

/// The durable record of one import: the retained artifact's content address, the binding
/// that read it, the binding's own loss report and the engine's round-trip diff, plus - once
/// the import is committed - the commit and the losses the request accepted by name. The
/// artifact bytes themselves live in the blob store under `artifact_hash`, so the record
/// and the artifact are always addressable together.
#[derive(Debug, Clone, PartialEq)]
pub struct ImportRecord {
    pub artifact_hash: String,
    pub project: String,
    pub binding_id: String,
    pub binding_version: String,
    /// The binding's own claim, as JSON (a `binding::LossReport`) - the only account of the
    /// native XMI->OKF read, which is not independently measured.
    pub loss_report: String,
    /// The engine's diff of the binding's own OKF->XMI->OKF round trip, as JSON (an
    /// `okf::diff::DiffReport`). This measures the round trip only - the native XMI->OKF
    /// read is covered by `loss_report` (the binding's own claim), not by this field.
    pub fidelity_diff: String,
    /// The commit that landed this import, once it has been committed.
    pub commit_hash: Option<String>,
    /// The blocking loss subjects the request accepted by name, in request order.
    pub accepted_losses: Vec<String>,
    pub created_at: String,
}

/// The provenance an import commit carries: the retained artifact it was read from, the
/// binding that read it (id and version), the blocking losses the request accepted by name,
/// and - when the import is the result of a human accepting an agent's proposal - the
/// acceptance (which proposal, which agent, which human). It is written INSIDE the same
/// transaction as the commit row, so a commit can never land unlinked from the source
/// artifact or the binding it migrated through. The loss report and round-trip diff still
/// live on the import record itself (ImportRecord), written by record_import before the
/// commit; this carries the fields the commit's own provenance column must record.
///
/// The acceptance rides the import provenance rather than a parallel parameter so the
/// commit path's ONE provenance input can express an authored, imported or accepted commit
/// without changing the store's commit signature (which the offline CLI also calls).
#[derive(Debug, Clone, PartialEq)]
pub struct ImportProvenance {
    pub artifact_hash: String,
    pub binding_id: String,
    pub binding_version: String,
    pub accepted_losses: Vec<String>,
    /// Some when this import is a human's acceptance of an agent's proposal. The commit is
    /// then labelled [CommitProvenance::Accepted] rather than Imported, and the proposal
    /// record is updated (decided, decided-by, commit) inside the SAME transaction as the
    /// commit row, so an acceptance can never outlive or precede the commit it describes.
    pub acceptance: Option<ProposalAcceptance>,
}

/// Who accepted an agent's proposal, so an acceptance commit can name both parties. The
/// agent is read from the recorded proposal (the agent that proposed), and accepted_by is
/// the verified human who accepted - never a name taken from a request body. The accepted
/// items themselves travel as the import provenance's accepted_losses: the items a
/// loss-resolution proposal names are exactly the losses the import accepts.
#[derive(Debug, Clone, PartialEq)]
pub struct ProposalAcceptance {
    pub proposal_id: String,
    pub agent: String,
    pub accepted_by: String,
}

/// The provenance a commit carries when a human accepts an agent's MODEL-CHANGE proposal:
/// the proposal id, the agent that proposed, the human that accepted, and the accepted
/// items (the identities of the proposed changes). Unlike an import acceptance, there is no
/// retained source artifact, binding or loss report to substantiate: the candidate document
/// is the proposal's own material, and it is validated by the shared commit core rather than
/// by a binding. Written INSIDE the same transaction as the commit row, so an acceptance can
/// never outlive or precede the commit it describes.
#[derive(Debug, Clone, PartialEq)]
pub struct AcceptanceProvenance {
    pub proposal_id: String,
    pub agent: String,
    pub accepted_by: String,
    pub accepted_items: Vec<String>,
}

/// How a recorded proposal was decided. A proposal starts undecided (a NULL decision
/// column); accepting or refusing it records one of these. An undecided proposal is
/// addressable and actable; a decided one is a decision that can never be undone, because
/// the decision rides the same write that produced it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProposalDecision {
    Accepted,
    Refused,
}

impl ProposalDecision {
    pub fn as_str(&self) -> &'static str {
        match self {
            ProposalDecision::Accepted => "accepted",
            ProposalDecision::Refused => "refused",
        }
    }

    pub fn parse(raw: &str) -> Result<Self, StoreError> {
        match raw {
            "accepted" => Ok(ProposalDecision::Accepted),
            "refused" => Ok(ProposalDecision::Refused),
            other => Err(StoreError::Backend(format!(
                "corrupt proposal decision: {}",
                other
            ))),
        }
    }
}

/// The durable record of one agent proposal: who the agent is, the task it answered, the
/// retained artifact (when the proposal resolves a loss report) and the review artifact
/// itself, plus - once a human decides - the decision, the human who made it, the accepted
/// items and the commit the acceptance produced. The review artifact is stored as the JSON
/// the agent produced, so the human's decision is recorded against exactly what the agent
/// wrote, not a re-serialisation that could drift.
#[derive(Debug, Clone, PartialEq)]
pub struct ProposalRecord {
    pub id: String,
    pub project: String,
    pub agent: String,
    pub task_goal: String,
    /// The retained artifact the proposal's loss report is about, when the material is a
    /// loss report; None for a proposal over other material.
    pub artifact_hash: Option<String>,
    /// The binding selector (id@version) the proposal's loss report was produced by.
    pub binding: Option<String>,
    /// The review artifact, as the JSON the agent produced.
    pub review_artifact: String,
    pub created_at: String,
    pub decision: Option<ProposalDecision>,
    /// The verified subject who decided (the accepting or refusing human), empty until
    /// decided.
    pub decided_by: String,
    /// The accepted item identities, in the order the human named them.
    pub accepted_items: Vec<String>,
    /// The commit the acceptance produced, once accepted.
    pub commit_hash: Option<String>,
    /// Wall-clock time the decision was recorded, empty until decided.
    pub decided_at: String,
}

/// Deterministic, content-addressed identifier for a proposal: the project, the agent that
/// proposed and the review artifact itself. Recording the same proposal twice yields the
/// same id, so re-recording is a no-op and a test can predict the id without a random
/// source. Truncated to 32 hex characters, ample for a proposal.
pub fn proposal_id(project: &str, agent: &str, review_artifact: &str) -> String {
    let digest = Sha256::digest(format!("{}|{}|{}", project, agent, review_artifact).as_bytes());
    let encoded = hex::encode(digest);
    encoded[..32].to_string()
}

/// The blocking losses a recorded import's loss report says were NOT accepted by name. The
/// report is the binding's own account of its native read (not an independent measurement),
/// and an acceptance names a loss by its entry identity ("subject [verdict]"). Returns the
/// entry identities of the blocking losses missing from `accepted` - empty when the
/// provenance is honest. A report that cannot be parsed is corruption, reported as a storage
/// error rather than read as "no losses".
pub fn unaccepted_losses(
    loss_report: &str,
    accepted: &[String],
) -> Result<Vec<String>, StoreError> {
    let report: binding::LossReport = serde_json::from_str(loss_report)
        .map_err(|e| StoreError::Backend(format!("corrupt loss report: {}", e)))?;
    Ok(report
        .blocking()
        .into_iter()
        .map(agent::losses::entry_identity)
        .filter(|identity| !accepted.contains(identity))
        .collect())
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
/// inside the store; `actor` is the verified identity's subject - who actually performed
/// the action - never a name taken from the request body. In open mode that subject is
/// "anonymous": nobody was authenticated, which is the honest statement. `mechanism` records
/// HOW the caller authenticated (`open`, `static`, `jwt` or `agent`) so a shared token is
/// never recorded byte-identically to a named individual; it is the subject's companion,
/// not a substitute for it. `authorizer` is the human or service on whose behalf an agent
/// acted; it is empty for every human action, so a reader can tell an agent from a human
/// from the log alone.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditEntry {
    pub id: i64,
    pub project: String,
    pub at: i64,
    pub actor: String,
    pub mechanism: String,
    pub authorizer: String,
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
            } => write!(
                f,
                "{} is locked by {} until {}",
                element, holder, expires_at
            ),
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
    ///
    /// An optional `import` provenance is written INSIDE the same transaction as the commit
    /// row: the commit and its link to the source artifact succeed or fail together, so a
    /// committed import can never be permanently unlinked from its source. When the import
    /// record it names does not exist, the whole commit is refused with `NotFound` and no
    /// commit row is written.
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
        import: Option<&ImportProvenance>,
    ) -> Result<Commit, StoreError>;

    /// Commit a model-change acceptance: a commit whose provenance is
    /// [CommitProvenance::Accepted], naming the proposal, the agent that proposed it and the
    /// human that accepted it, plus the accepted items. There is no import to substantiate:
    /// the candidate document was validated by the shared commit core before this call, and
    /// the acceptance is substantiated HERE - the proposal must exist, be undecided, and have
    /// been recorded by the named agent - inside the same transaction that writes the commit
    /// row and marks the proposal accepted. Mirrors [commit_model]'s sequence (guard, tip,
    /// parents, hash, commit row, branch tip, audit row) without the import checks.
    #[allow(clippy::too_many_arguments)]
    fn commit_accepted(
        &self,
        project: &str,
        branch: &str,
        okf_hash: &str,
        author: &str,
        message: &str,
        guard: Option<CommitGuard<'_>>,
        audit: Option<&AuditEntry>,
        acceptance: &AcceptanceProvenance,
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

    /// The gate runs that checked THIS commit (the candidate), oldest first. The answer is
    /// scoped to a commit, never a branch or a project, so a later commit does not inherit
    /// an earlier verdict: a verdict that followed a branch would be a claim about a model
    /// that was never checked.
    fn gate_runs_for_commit(&self, project: &str, hash: &str) -> Result<Vec<GateRun>, StoreError>;

    /// Record an import's loss report and round-trip diff, keyed by (project, artifact_hash).
    /// This runs BEFORE the commit is attempted, so a refused import still has its report
    /// retrievable - the caller must be able to read exactly which losses to accept. The key
    /// is the PROJECT plus the retained artifact's content address, never the address alone:
    /// a content address is global, so two projects importing the same artifact each get their
    /// own record. Re-recording the same (project, artifact) is a no-op, since the same bytes
    /// always produce the same report.
    #[allow(clippy::too_many_arguments)]
    fn record_import(
        &self,
        project: &str,
        artifact_hash: &str,
        binding_id: &str,
        binding_version: &str,
        loss_report: &str,
        fidelity_diff: &str,
    ) -> Result<(), StoreError>;

    /// The import record for a retained artifact in this project, if one was recorded.
    fn import_report(
        &self,
        project: &str,
        artifact_hash: &str,
    ) -> Result<Option<ImportRecord>, StoreError>;

    /// Record one agent proposal and return its content-addressed id. The agent is the
    /// verified subject supplied by the caller (never a name from a request body), the task
    /// and review artifact are what the agent produced, and the retained artifact and binding
    /// are carried so a later acceptance can re-run the import the proposal resolves. The
    /// audit row, when supplied, rides the same transaction, so a durable proposal always has
    /// its record and vice versa. Re-recording the same proposal is a no-op that returns the
    /// same id.
    #[allow(clippy::too_many_arguments)]
    fn record_proposal(
        &self,
        project: &str,
        agent: &str,
        task_goal: &str,
        artifact_hash: Option<&str>,
        binding: Option<&str>,
        review_artifact: &str,
        audit: Option<&AuditEntry>,
    ) -> Result<String, StoreError>;

    /// The proposal record for this id in this project, if one was recorded.
    fn proposal(&self, project: &str, id: &str) -> Result<Option<ProposalRecord>, StoreError>;

    /// The proposals for this project, newest first (by insertion order, so a re-recorded
    /// proposal keeps its original position). The list is what the proposals page renders:
    /// every proposal, decided or not, with the decision attached.
    fn list_proposals(&self, project: &str) -> Result<Vec<ProposalRecord>, StoreError>;

    /// Record a human's decision to REFUSE a proposal. A refusal is a decision, not a failed
    /// action: the proposal is marked refused with the verified subject and the wall clock,
    /// and the audit row (when supplied) rides the same transaction. The proposal must exist
    /// and be undecided; accepting or refusing a decided proposal is a conflict.
    fn refuse_proposal(
        &self,
        project: &str,
        id: &str,
        decided_by: &str,
        audit: Option<&AuditEntry>,
    ) -> Result<(), StoreError>;

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

/// Which backend the store should open. `Sqlite` takes a filesystem path; `Postgres` takes
/// a libpq-style connection URL. Chosen from the environment by [`StoreConfig::from_env`]:
/// `MW_DATABASE_URL` (when set) selects Postgres, otherwise `MW_DB` (default `modelwrite.db`)
/// selects SQLite.
#[derive(Debug, Clone, PartialEq)]
pub enum StoreConfig {
    Sqlite(std::path::PathBuf),
    Postgres(String),
}

impl StoreConfig {
    /// Read the backend choice from the environment. `MW_DATABASE_URL` takes precedence and
    /// selects Postgres; otherwise `MW_DB` is a filesystem path for SQLite, defaulting to
    /// `modelwrite.db`.
    pub fn from_env() -> Result<Self, StoreError> {
        if let Ok(url) = std::env::var("MW_DATABASE_URL") {
            if !url.trim().is_empty() {
                return Ok(StoreConfig::Postgres(url));
            }
        }
        let path = std::env::var("MW_DB").unwrap_or_else(|_| "modelwrite.db".to_string());
        Ok(StoreConfig::Sqlite(std::path::PathBuf::from(path)))
    }
}

/// Open a store of the configured backend, behind the shared [`Store`] trait, so nothing
/// above the store changes when the backend does.
pub fn open_store(config: &StoreConfig) -> Result<Arc<dyn Store>, StoreError> {
    match config {
        StoreConfig::Sqlite(path) => sqlite::SqliteStore::open(path)
            .map(|store| Arc::new(store) as Arc<dyn Store>)
            .map_err(|e| StoreError::Backend(e.to_string())),
        #[cfg(feature = "postgres")]
        StoreConfig::Postgres(url) => postgres::PostgresStore::open(url)
            .map(|store| Arc::new(store) as Arc<dyn Store>),
        #[cfg(not(feature = "postgres"))]
        StoreConfig::Postgres(_) => Err(StoreError::Backend(
            "this build was compiled without the postgres backend; rebuild with the postgres feature"
                .to_string(),
        )),
    }
}
