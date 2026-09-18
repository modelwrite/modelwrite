// SPDX-License-Identifier: AGPL-3.0-or-later
use std::path::Path;

use rusqlite::{params, Connection};

use super::{
    now_epoch, AuditEntry, Commit, CommitGuard, CommitProvenance, GateRun, ImportProvenance,
    ImportRecord, Lock, Project, Store, StoreError,
};

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS projects (
    name TEXT PRIMARY KEY,
    created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS blobs (
    hash TEXT PRIMARY KEY,
    bytes BLOB NOT NULL
);
CREATE TABLE IF NOT EXISTS commits (
    hash TEXT PRIMARY KEY,
    project TEXT NOT NULL,
    branch TEXT NOT NULL,
    parents TEXT NOT NULL,
    okf_hash TEXT NOT NULL,
    author TEXT NOT NULL,
    message TEXT NOT NULL,
    created_at TEXT NOT NULL,
    provenance TEXT
);
CREATE INDEX IF NOT EXISTS commits_by_project_branch ON commits (project, branch);
CREATE TABLE IF NOT EXISTS branches (
    project TEXT NOT NULL,
    name TEXT NOT NULL,
    tip TEXT NOT NULL,
    PRIMARY KEY (project, name)
);
CREATE TABLE IF NOT EXISTS gate_runs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project TEXT NOT NULL,
    branch TEXT NOT NULL,
    reference_hash TEXT NOT NULL,
    candidate_hash TEXT NOT NULL,
    passed INTEGER NOT NULL,
    evidence TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS imports (
    artifact_hash TEXT NOT NULL,
    project TEXT NOT NULL,
    binding_id TEXT NOT NULL,
    binding_version TEXT NOT NULL,
    loss_report TEXT NOT NULL,
    fidelity_diff TEXT NOT NULL,
    commit_hash TEXT,
    accepted_losses TEXT NOT NULL,
    created_at TEXT NOT NULL,
    PRIMARY KEY (project, artifact_hash)
);
CREATE INDEX IF NOT EXISTS imports_by_commit ON imports (commit_hash);
CREATE TABLE IF NOT EXISTS locks (
    id TEXT PRIMARY KEY,
    project TEXT NOT NULL,
    branch TEXT NOT NULL,
    element TEXT NOT NULL,
    holder TEXT NOT NULL,
    acquired_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS locks_element ON locks(project, element);
CREATE TABLE IF NOT EXISTS audit (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project TEXT NOT NULL,
    at INTEGER NOT NULL,
    actor TEXT NOT NULL,
    mechanism TEXT NOT NULL,
    authorizer TEXT NOT NULL DEFAULT '',
    action TEXT NOT NULL,
    subject TEXT NOT NULL,
    detail TEXT NOT NULL,
    -- An entry that names nobody or nothing is not a record of anything. The constraint is
    -- also how a test can make an audit write fail on purpose: if the entry cannot be
    -- written, the mutation it describes must not survive either, which is the atomicity
    -- the audit trail promises.
    CHECK (length(actor) > 0),
    -- The mechanism must always be recorded: without it, a shared token and a named
    -- individual become byte-identical once the subject matches, which is exactly the
    -- per-person attribution the log must not imply.
    CHECK (length(mechanism) > 0),
    CHECK (length(action) > 0)
);
-- Append-only is a property of the DATABASE, not a convention of the trait: these
-- triggers make UPDATE and DELETE on the audit table fail outright, so a written entry
-- can never be rewritten or removed by any code path that reaches SQLite.
CREATE TRIGGER IF NOT EXISTS audit_no_update
BEFORE UPDATE ON audit
BEGIN
    SELECT RAISE(ABORT, 'audit entries are append-only');
END;
CREATE TRIGGER IF NOT EXISTS audit_no_delete
BEFORE DELETE ON audit
BEGIN
    SELECT RAISE(ABORT, 'audit entries are append-only');
END;
";

/// Bring a database created by an earlier version up to the current schema.
///
/// `CREATE TABLE IF NOT EXISTS` is not a migration: a table that already exists is left
/// exactly as it was, so a column added to the CREATE statement never reaches an existing
/// database. Without this, an installation that upgraded would keep an audit table with no
/// `mechanism` column and EVERY write would fail - the kind of failure that is discovered
/// during a customer upgrade rather than in a test. Migrations run in order and are
/// idempotent, so starting the service twice is harmless.
fn migrate(connection: &Connection) -> anyhow::Result<()> {
    let has_mechanism: bool = connection
        .prepare("SELECT 1 FROM pragma_table_info('audit') WHERE name = 'mechanism'")?
        .exists([])?;
    if !has_mechanism {
        // The default is the honest one for rows written before the field existed: those
        // rows came from a build that had no authentication, so nobody was verified.
        connection
            .execute_batch("ALTER TABLE audit ADD COLUMN mechanism TEXT NOT NULL DEFAULT 'open'")?;
    }
    let has_authorizer: bool = connection
        .prepare("SELECT 1 FROM pragma_table_info('audit') WHERE name = 'authorizer'")?
        .exists([])?;
    if !has_authorizer {
        // Rows written before the field existed were human actions, so the empty default is
        // the honest value: no agent authorizer existed to record.
        connection
            .execute_batch("ALTER TABLE audit ADD COLUMN authorizer TEXT NOT NULL DEFAULT ''")?;
    }
    let has_provenance: bool = connection
        .prepare("SELECT 1 FROM pragma_table_info('commits') WHERE name = 'provenance'")?
        .exists([])?;
    if !has_provenance {
        // A commit written before provenance existed has no record of how it was produced.
        // The column stays NULL for those rows, and NULL reads as Unknown: absence is never
        // upgraded to a claim that the commit was authored.
        connection.execute_batch("ALTER TABLE commits ADD COLUMN provenance TEXT")?;
    }
    Ok(())
}

pub struct SqliteStore {
    connection: std::sync::Mutex<Connection>,
}

impl SqliteStore {
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        let connection = Connection::open(path)?;
        connection.execute_batch(SCHEMA)?;
        migrate(&connection)?;
        Ok(Self {
            connection: std::sync::Mutex::new(connection),
        })
    }

    fn with<T>(&self, f: impl FnOnce(&Connection) -> rusqlite::Result<T>) -> Result<T, StoreError> {
        let guard = self
            .connection
            .lock()
            .map_err(|_| StoreError::Backend("connection lock poisoned".to_string()))?;
        f(&guard).map_err(|e| StoreError::Backend(e.to_string()))
    }
}

/// One commit row exactly as stored: hash, project, branch, parents, okf_hash, author,
/// message, created_at, provenance. Named so the query helpers stay readable and
/// clippy-clean.
type CommitRow = (
    String,
    String,
    String,
    String,
    String,
    String,
    String,
    String,
    Option<String>,
);

/// One import row exactly as stored: artifact_hash, project, binding_id, binding_version,
/// loss_report, fidelity_diff, commit_hash, accepted_losses, created_at.
type ImportRow = (
    String,
    String,
    String,
    String,
    String,
    String,
    Option<String>,
    String,
    String,
);

/// Parse the parents column, reporting corruption rather than hiding it: an empty list
/// silently substituted here would change what commit_hash covers.
fn parse_parents(hash: &str, raw: &str) -> Result<Vec<String>, StoreError> {
    serde_json::from_str(raw).map_err(|e| {
        StoreError::Backend(format!("corrupt parents column for commit {}: {}", hash, e))
    })
}

/// Insert one audit row and return its AUTOINCREMENT id. The caller's `id` field is
/// ignored: the store assigns ids, and no update or delete path ever reuses or rewrites one.
/// Shared by `append_audit` and by the transactions that must write the audit row
/// atomically with the mutation it describes.
fn insert_audit(c: &Connection, entry: &AuditEntry) -> rusqlite::Result<i64> {
    c.execute(
        "INSERT INTO audit (project, at, actor, mechanism, authorizer, action, subject, detail) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![entry.project, entry.at, entry.actor, entry.mechanism, entry.authorizer, entry.action, entry.subject, entry.detail],
    )?;
    Ok(c.last_insert_rowid())
}

/// Run a closure inside ONE write transaction, committing only when it succeeds. On
/// failure the transaction is dropped, which rolls back every statement the closure ran.
/// This is what makes a mutation and its audit row land together or not at all.
fn with_tx<T>(
    connection: &std::sync::Mutex<Connection>,
    f: impl FnOnce(&rusqlite::Transaction<'_>) -> Result<T, StoreError>,
) -> Result<T, StoreError> {
    let guard = connection
        .lock()
        .map_err(|_| StoreError::Backend("connection lock poisoned".to_string()))?;
    let tx = guard
        .unchecked_transaction()
        .map_err(|e| StoreError::Backend(e.to_string()))?;
    let result = f(&tx);
    if result.is_ok() {
        tx.commit()
            .map_err(|e| StoreError::Backend(e.to_string()))?;
    }
    result
}

/// Refuse a guarded write when any element carries a live lease owned by a different
/// holder. Every write path — commit, merge and reset — funnels through this one rule, so
/// a lock protects an element from being changed no matter which path is used. An empty
/// holder cannot own a lease, so any live lease on a touched element refuses it: a request
/// without a holder is checked, not exempted.
fn enforce_guard(
    tx: &rusqlite::Transaction<'_>,
    project: &str,
    guard: &CommitGuard<'_>,
) -> Result<(), StoreError> {
    for element in guard.elements {
        let held: Option<(String, i64)> = (|| -> rusqlite::Result<Option<(String, i64)>> {
            let mut stmt = tx.prepare(
                "SELECT holder, expires_at FROM locks WHERE project = ?1 AND element = ?2 AND expires_at > ?3 AND holder != ?4 LIMIT 1",
            )?;
            let mut rows = stmt.query(params![project, element, guard.now, guard.holder])?;
            match rows.next()? {
                Some(row) => Ok(Some((row.get(0)?, row.get(1)?))),
                None => Ok(None),
            }
        })()
        .map_err(|e| StoreError::Backend(e.to_string()))?;
        if let Some((holder, expires_at)) = held {
            return Err(super::lock_refusal(element, &holder, expires_at));
        }
    }
    Ok(())
}

impl Store for SqliteStore {
    fn create_project(
        &self,
        name: &str,
        audit: Option<&AuditEntry>,
    ) -> Result<Project, StoreError> {
        // Check-then-insert would be a time-of-check window: two concurrent callers could
        // both pass the check. The primary key settles it instead, and an insert that
        // changed nothing is reported as the conflict it is. The audit row, when supplied,
        // rides the same transaction, so a project and its record land together.
        let created_at = now_epoch();
        with_tx(&self.connection, |tx| {
            let inserted = tx
                .execute(
                    "INSERT OR IGNORE INTO projects (name, created_at) VALUES (?1, ?2)",
                    params![name, created_at],
                )
                .map_err(|e| StoreError::Backend(e.to_string()))?;
            if inserted == 0 {
                return Err(StoreError::Conflict(format!(
                    "project {} already exists",
                    name
                )));
            }
            if let Some(audit) = audit {
                insert_audit(tx, audit).map_err(|e| StoreError::Backend(e.to_string()))?;
            }
            Ok(())
        })?;
        Ok(Project {
            name: name.to_string(),
            created_at,
        })
    }

    fn project(&self, name: &str) -> Result<Option<Project>, StoreError> {
        self.with(|c| {
            let mut stmt = c.prepare("SELECT name, created_at FROM projects WHERE name = ?1")?;
            let mut rows = stmt.query(params![name])?;
            match rows.next()? {
                Some(row) => Ok(Some(Project {
                    name: row.get(0)?,
                    created_at: row.get(1)?,
                })),
                None => Ok(None),
            }
        })
    }

    fn list_projects(&self) -> Result<Vec<Project>, StoreError> {
        self.with(|c| {
            let mut stmt = c.prepare("SELECT name, created_at FROM projects ORDER BY name")?;
            let rows = stmt.query_map([], |row| {
                Ok(Project {
                    name: row.get(0)?,
                    created_at: row.get(1)?,
                })
            })?;
            rows.collect()
        })
    }

    fn put_blob(&self, bytes: &[u8]) -> Result<String, StoreError> {
        let hash = super::blob_hash(bytes);
        self.with(|c| {
            c.execute(
                "INSERT OR IGNORE INTO blobs (hash, bytes) VALUES (?1, ?2)",
                params![hash, bytes],
            )
        })?;
        Ok(hash)
    }

    fn blob(&self, hash: &str) -> Result<Option<Vec<u8>>, StoreError> {
        self.with(|c| {
            let mut stmt = c.prepare("SELECT bytes FROM blobs WHERE hash = ?1")?;
            let mut rows = stmt.query(params![hash])?;
            match rows.next()? {
                Some(row) => Ok(Some(row.get(0)?)),
                None => Ok(None),
            }
        })
    }

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
    ) -> Result<Commit, StoreError> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| StoreError::Backend("connection lock poisoned".to_string()))?;
        let tx = connection
            .unchecked_transaction()
            .map_err(|e| StoreError::Backend(e.to_string()))?;

        // The lock check happens HERE, inside the transaction that writes the commit. A
        // check performed by the caller before this call would leave a window in which
        // another holder takes the lock and the guarded commit lands regardless.
        if let Some(guard) = guard.as_ref() {
            enforce_guard(&tx, project, guard)?;
        }

        // The tip is read inside the same lock and transaction that writes the commit, so
        // two concurrent commits to one branch chain rather than fork.
        let tip: Option<String> = (|| -> rusqlite::Result<Option<String>> {
            let mut stmt =
                tx.prepare("SELECT tip FROM branches WHERE project = ?1 AND name = ?2")?;
            let mut rows = stmt.query(params![project, branch])?;
            match rows.next()? {
                Some(row) => Ok(Some(row.get(0)?)),
                None => Ok(None),
            }
        })()
        .map_err(|e| StoreError::Backend(e.to_string()))?;
        // A guard computed against a tip that has since moved describes a document that is
        // no longer the branch's: enforcing it would check the wrong elements. Refuse, so
        // the caller re-reads and retries rather than being told a stale answer.
        if let Some(guard) = guard.as_ref() {
            if guard.expected_tip != tip.as_deref() {
                return Err(StoreError::Conflict(format!(
                    "branch {} moved while the commit was being prepared; re-read and retry",
                    branch
                )));
            }
        }

        let parents: Vec<String> = tip.into_iter().collect();
        let created_at = now_epoch();
        let hash = super::commit_hash(project, branch, &parents, okf_hash, author, message);
        let parents_json =
            serde_json::to_string(&parents).map_err(|e| StoreError::Backend(e.to_string()))?;

        // Provenance is decided HERE, from the same import the caller supplied, and written in
        // this transaction: an authored commit (import None) is labelled authored, an import
        // is labelled imported with its retained artifact, binding and accepted losses.
        // Nothing patches it afterwards, so the commit can never disagree with how it was
        // written.
        let provenance = match import {
            Some(import) => CommitProvenance::Imported {
                artifact_hash: import.artifact_hash.clone(),
                binding_id: import.binding_id.clone(),
                binding_version: import.binding_version.clone(),
                accepted_losses: import.accepted_losses.clone(),
            },
            None => CommitProvenance::Authored,
        };
        let provenance_col = provenance.column_value();

        // Plain INSERT, not OR IGNORE: a constraint failure must abort this transaction
        // rather than move a branch tip to a hash that has no commit row.
        tx.execute(
            "INSERT INTO commits (hash, project, branch, parents, okf_hash, author, message, created_at, provenance) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![hash, project, branch, parents_json, okf_hash, author, message, created_at, provenance_col],
        )
        .map_err(|e| StoreError::Backend(e.to_string()))?;
        tx.execute(
            "INSERT INTO branches (project, name, tip) VALUES (?1, ?2, ?3) ON CONFLICT(project, name) DO UPDATE SET tip = ?3",
            params![project, branch, hash],
        )
        .map_err(|e| StoreError::Backend(e.to_string()))?;
        if let Some(audit) = audit {
            insert_audit(&tx, audit).map_err(|e| StoreError::Backend(e.to_string()))?;
        }
        // I2: the import link is written INSIDE the commit transaction. If the import record
        // it names is missing, the UPDATE affects no rows and the whole transaction rolls back,
        // so a commit can never land without its provenance (and vice versa).
        if let Some(import) = import {
            let accepted_json = serde_json::to_string(&import.accepted_losses)
                .map_err(|e| StoreError::Backend(e.to_string()))?;
            let updated = tx
                .execute(
                    "UPDATE imports SET commit_hash = ?1, accepted_losses = ?2 WHERE project = ?3 AND artifact_hash = ?4",
                    params![hash, accepted_json, project, import.artifact_hash],
                )
                .map_err(|e| StoreError::Backend(e.to_string()))?;
            if updated == 0 {
                return Err(StoreError::NotFound(format!(
                    "import {} for project {}",
                    import.artifact_hash, project
                )));
            }
        }
        tx.commit()
            .map_err(|e| StoreError::Backend(e.to_string()))?;

        Ok(Commit {
            hash,
            project: project.to_string(),
            branch: branch.to_string(),
            parents,
            okf_hash: okf_hash.to_string(),
            author: author.to_string(),
            message: message.to_string(),
            created_at,
            provenance,
        })
    }

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
    ) -> Result<Commit, StoreError> {
        // A merge commit has exactly two parents. Anything else is a caller mistake, and it
        // would quietly write a root commit or an ordinary single-parent commit under the
        // name of a merge.
        if parents.len() != 2 {
            return Err(StoreError::Backend(format!(
                "a merge commit needs exactly two parents, got {}",
                parents.len()
            )));
        }
        let connection = self
            .connection
            .lock()
            .map_err(|_| StoreError::Backend("connection lock poisoned".to_string()))?;
        let tx = connection
            .unchecked_transaction()
            .map_err(|e| StoreError::Backend(e.to_string()))?;

        // The first parent must still BE the branch tip. A merge is computed from tips read
        // BEFORE this call, so if anything landed in between - another commit, a merge or a
        // reset - writing this commit would move the branch off that work and leave it
        // unreachable: a stored commit no branch explains, which is silent loss. Refusing
        // with a conflict is the same check-then-write discipline commit_model applies.
        let current_tip: Option<String> = (|| -> rusqlite::Result<Option<String>> {
            let mut stmt =
                tx.prepare("SELECT tip FROM branches WHERE project = ?1 AND name = ?2")?;
            let mut rows = stmt.query(params![project, branch])?;
            match rows.next()? {
                Some(row) => Ok(Some(row.get(0)?)),
                None => Ok(None),
            }
        })()
        .map_err(|e| StoreError::Backend(e.to_string()))?;
        if current_tip.as_deref() != Some(parents[0].as_str()) {
            return Err(StoreError::Conflict(format!(
                "branch {} moved while the merge was being prepared (expected {}, found {})",
                branch,
                parents[0],
                current_tip.as_deref().unwrap_or("nothing")
            )));
        }

        // The merge's guard runs inside the transaction that writes it, so a lock taken
        // between the caller's read and this write cannot be bypassed.
        if let Some(guard) = guard.as_ref() {
            enforce_guard(&tx, project, guard)?;
        }

        // Every parent must already exist: a merge commit can only cite ancestry that is
        // really there, never invent it.
        for parent in parents {
            let exists: bool = (|| -> rusqlite::Result<bool> {
                let mut stmt =
                    tx.prepare("SELECT 1 FROM commits WHERE project = ?1 AND hash = ?2")?;
                let mut rows = stmt.query(params![project, parent])?;
                Ok(rows.next()?.is_some())
            })()
            .map_err(|e| StoreError::Backend(e.to_string()))?;
            if !exists {
                return Err(StoreError::NotFound(format!("parent commit {}", parent)));
            }
        }

        let parents: Vec<String> = parents.to_vec();
        let created_at = now_epoch();
        let hash = super::commit_hash(project, branch, &parents, okf_hash, author, message);
        let parents_json =
            serde_json::to_string(&parents).map_err(|e| StoreError::Backend(e.to_string()))?;

        // A merge is a write path, not a migration: it is labelled authored, never left for a
        // reader to guess.
        let provenance = CommitProvenance::Authored;
        let provenance_col = provenance.column_value();

        // Plain INSERT, not OR IGNORE: a constraint failure must abort this transaction
        // rather than move a branch tip to a hash that has no commit row.
        tx.execute(
            "INSERT INTO commits (hash, project, branch, parents, okf_hash, author, message, created_at, provenance) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![hash, project, branch, parents_json, okf_hash, author, message, created_at, provenance_col],
        )
        .map_err(|e| StoreError::Backend(e.to_string()))?;
        tx.execute(
            "INSERT INTO branches (project, name, tip) VALUES (?1, ?2, ?3) ON CONFLICT(project, name) DO UPDATE SET tip = ?3",
            params![project, branch, hash],
        )
        .map_err(|e| StoreError::Backend(e.to_string()))?;
        if let Some(audit) = audit {
            insert_audit(&tx, audit).map_err(|e| StoreError::Backend(e.to_string()))?;
        }
        tx.commit()
            .map_err(|e| StoreError::Backend(e.to_string()))?;

        Ok(Commit {
            hash,
            project: project.to_string(),
            branch: branch.to_string(),
            parents,
            okf_hash: okf_hash.to_string(),
            author: author.to_string(),
            message: message.to_string(),
            created_at,
            provenance,
        })
    }

    fn commit(&self, project: &str, hash: &str) -> Result<Option<Commit>, StoreError> {
        // The row is read inside the lock and the parents and provenance columns are parsed
        // outside it, so that corruption can surface as a storage error rather than a query
        // error.
        let row: Option<CommitRow> = self.with(|c| {
                let mut stmt = c.prepare(
                    "SELECT hash, project, branch, parents, okf_hash, author, message, created_at, provenance FROM commits WHERE project = ?1 AND hash = ?2",
                )?;
                let mut rows = stmt.query(params![project, hash])?;
                match rows.next()? {
                    Some(row) => Ok(Some((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                        row.get(7)?,
                        row.get(8)?,
                    ))),
                    None => Ok(None),
                }
            })?;

        match row {
            None => Ok(None),
            Some((
                hash,
                project,
                branch,
                parents,
                okf_hash,
                author,
                message,
                created_at,
                provenance,
            )) => Ok(Some(Commit {
                parents: parse_parents(&hash, &parents)?,
                provenance: CommitProvenance::parse_column(provenance.as_deref())?,
                hash,
                project,
                branch,
                okf_hash,
                author,
                message,
                created_at,
            })),
        }
    }

    fn commits_on(&self, project: &str, branch: &str) -> Result<Vec<Commit>, StoreError> {
        // Rows are collected first and the parents and provenance columns are parsed
        // afterwards, because a parse failure inside the query closure could only be reported
        // as a rusqlite error; corruption must surface as a storage error instead.
        let rows: Vec<CommitRow> = self.with(|c| {
            let mut stmt = c.prepare(
                "SELECT hash, project, branch, parents, okf_hash, author, message, created_at, provenance FROM commits WHERE project = ?1 AND branch = ?2 ORDER BY rowid",
            )?;
            let rows = stmt.query_map(params![project, branch], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7)?,
                    row.get(8)?,
                ))
            })?;
            rows.collect()
        })?;

        let mut commits = Vec::with_capacity(rows.len());
        for (hash, project, branch, parents, okf_hash, author, message, created_at, provenance) in
            rows
        {
            commits.push(Commit {
                parents: parse_parents(&hash, &parents)?,
                provenance: CommitProvenance::parse_column(provenance.as_deref())?,
                hash,
                project,
                branch,
                okf_hash,
                author,
                message,
                created_at,
            });
        }
        Ok(commits)
    }

    fn branch_tip(&self, project: &str, branch: &str) -> Result<Option<String>, StoreError> {
        self.with(|c| {
            let mut stmt =
                c.prepare("SELECT tip FROM branches WHERE project = ?1 AND name = ?2")?;
            let mut rows = stmt.query(params![project, branch])?;
            match rows.next()? {
                Some(row) => Ok(Some(row.get(0)?)),
                None => Ok(None),
            }
        })
    }

    fn create_branch(
        &self,
        project: &str,
        name: &str,
        from: &str,
        audit: Option<&AuditEntry>,
    ) -> Result<(), StoreError> {
        if self.commit(project, from)?.is_none() {
            return Err(StoreError::NotFound(format!("commit {}", from)));
        }
        // The composite primary key decides a duplicate branch, not a prior read. The
        // audit row rides the same transaction as the branch insert.
        with_tx(&self.connection, |tx| {
            let inserted = tx
                .execute(
                    "INSERT OR IGNORE INTO branches (project, name, tip) VALUES (?1, ?2, ?3)",
                    params![project, name, from],
                )
                .map_err(|e| StoreError::Backend(e.to_string()))?;
            if inserted == 0 {
                return Err(StoreError::Conflict(format!(
                    "branch {} already exists",
                    name
                )));
            }
            if let Some(audit) = audit {
                insert_audit(tx, audit).map_err(|e| StoreError::Backend(e.to_string()))?;
            }
            Ok(())
        })
    }

    fn delete_branch(
        &self,
        project: &str,
        name: &str,
        audit: Option<&AuditEntry>,
    ) -> Result<(), StoreError> {
        // Ask which thing is missing, so the 404 says so: a delete on an unknown project
        // would otherwise report a branch that was never the problem.
        if self.project(project)?.is_none() {
            return Err(StoreError::NotFound(format!("project {}", project)));
        }
        with_tx(&self.connection, |tx| {
            let removed = tx
                .execute(
                    "DELETE FROM branches WHERE project = ?1 AND name = ?2",
                    params![project, name],
                )
                .map_err(|e| StoreError::Backend(e.to_string()))?;
            if removed == 0 {
                return Err(StoreError::NotFound(format!("branch {}", name)));
            }
            if let Some(audit) = audit {
                insert_audit(tx, audit).map_err(|e| StoreError::Backend(e.to_string()))?;
            }
            Ok(())
        })
    }

    fn list_branches(&self, project: &str) -> Result<Vec<(String, String)>, StoreError> {
        self.with(|c| {
            let mut stmt =
                c.prepare("SELECT name, tip FROM branches WHERE project = ?1 ORDER BY name")?;
            let rows = stmt.query_map(params![project], |row| Ok((row.get(0)?, row.get(1)?)))?;
            rows.collect()
        })
    }

    fn record_gate_run(&self, run: &GateRun, audit: Option<&AuditEntry>) -> Result<(), StoreError> {
        with_tx(&self.connection, |tx| {
            tx.execute(
                "INSERT INTO gate_runs (project, branch, reference_hash, candidate_hash, passed, evidence, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![run.project, run.branch, run.reference_hash, run.candidate_hash, run.passed as i64, run.evidence, run.created_at],
            )
            .map_err(|e| StoreError::Backend(e.to_string()))?;
            if let Some(audit) = audit {
                insert_audit(tx, audit).map_err(|e| StoreError::Backend(e.to_string()))?;
            }
            Ok(())
        })
    }

    fn gate_runs(&self, project: &str) -> Result<Vec<GateRun>, StoreError> {
        self.with(|c| {
            let mut stmt = c.prepare(
                "SELECT project, branch, reference_hash, candidate_hash, passed, evidence, created_at FROM gate_runs WHERE project = ?1 ORDER BY id",
            )?;
            let rows = stmt.query_map(params![project], |row| {
                Ok(GateRun {
                    project: row.get(0)?,
                    branch: row.get(1)?,
                    reference_hash: row.get(2)?,
                    candidate_hash: row.get(3)?,
                    passed: row.get::<_, i64>(4)? != 0,
                    evidence: row.get(5)?,
                    created_at: row.get(6)?,
                })
            })?;
            rows.collect()
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn record_import(
        &self,
        project: &str,
        artifact_hash: &str,
        binding_id: &str,
        binding_version: &str,
        loss_report: &str,
        fidelity_diff: &str,
    ) -> Result<(), StoreError> {
        // The same bytes always produce the same report, so re-recording the same
        // (project, artifact) is a no-op. A record must exist BEFORE the commit is attempted,
        // so a refused import still has a retrievable report.
        self.with(|c| {
            c.execute(
                "INSERT OR IGNORE INTO imports (artifact_hash, project, binding_id, binding_version, loss_report, fidelity_diff, commit_hash, accepted_losses, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, '[]', ?7)",
                params![artifact_hash, project, binding_id, binding_version, loss_report, fidelity_diff, now_epoch()],
            )
        })?;
        Ok(())
    }

    fn import_report(
        &self,
        project: &str,
        artifact_hash: &str,
    ) -> Result<Option<ImportRecord>, StoreError> {
        // The row is read under the lock and the two JSON columns are parsed outside it, so
        // corruption surfaces as a storage error rather than a query error.
        let row: Option<ImportRow> = self.with(|c| {
            let mut stmt = c.prepare(
                "SELECT artifact_hash, project, binding_id, binding_version, loss_report, fidelity_diff, commit_hash, accepted_losses, created_at FROM imports WHERE project = ?1 AND artifact_hash = ?2",
            )?;
            let mut rows = stmt.query(params![project, artifact_hash])?;
            match rows.next()? {
                Some(row) => Ok(Some((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7)?,
                    row.get(8)?,
                ))),
                None => Ok(None),
            }
        })?;

        match row {
            None => Ok(None),
            Some((
                artifact_hash,
                project,
                binding_id,
                binding_version,
                loss_report,
                fidelity_diff,
                commit_hash,
                accepted_losses,
                created_at,
            )) => {
                let accepted_losses: Vec<String> =
                    serde_json::from_str(&accepted_losses).map_err(|e| {
                        StoreError::Backend(format!(
                            "corrupt accepted_losses column for import {}: {}",
                            artifact_hash, e
                        ))
                    })?;
                Ok(Some(ImportRecord {
                    artifact_hash,
                    project,
                    binding_id,
                    binding_version,
                    loss_report,
                    fidelity_diff,
                    commit_hash,
                    accepted_losses,
                    created_at,
                }))
            }
        }
    }

    fn append_audit(&self, entry: &AuditEntry) -> Result<i64, StoreError> {
        self.with(|c| insert_audit(c, entry))
    }

    fn audit(&self, project: &str, limit: i64) -> Result<Vec<AuditEntry>, StoreError> {
        let limit = limit.clamp(1, 1000);
        self.with(|c| {
            let mut stmt = c.prepare(
                "SELECT id, project, at, actor, mechanism, authorizer, action, subject, detail FROM audit WHERE project = ?1 ORDER BY id DESC LIMIT ?2",
            )?;
            let rows = stmt.query_map(params![project, limit], |row| {
                Ok(AuditEntry {
                    id: row.get(0)?,
                    project: row.get(1)?,
                    at: row.get(2)?,
                    actor: row.get(3)?,
                    mechanism: row.get(4)?,
                    authorizer: row.get(5)?,
                    action: row.get(6)?,
                    subject: row.get(7)?,
                    detail: row.get(8)?,
                })
            })?;
            rows.collect()
        })
    }

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
    ) -> Result<Vec<Lock>, StoreError> {
        let guard = self
            .connection
            .lock()
            .map_err(|_| StoreError::Backend("connection lock poisoned".to_string()))?;
        let tx = guard
            .unchecked_transaction()
            .map_err(|e| StoreError::Backend(e.to_string()))?;

        // Sweep dead leases first, so an expired lock never blocks a new holder.
        tx.execute("DELETE FROM locks WHERE expires_at <= ?1", params![now])
            .map_err(|e| StoreError::Backend(e.to_string()))?;

        // All or nothing: refuse before writing anything if any requested element is
        // held by a live lease owned by a different holder.
        for element in elements {
            let conflict: Option<(String, i64)> =
                (|| -> rusqlite::Result<Option<(String, i64)>> {
                    let mut stmt = tx.prepare(
                        "SELECT holder, expires_at FROM locks WHERE project = ?1 AND element = ?2 AND holder != ?3",
                    )?;
                    let mut rows = stmt.query(params![project, element.as_str(), holder])?;
                    match rows.next()? {
                        Some(row) => Ok(Some((row.get(0)?, row.get(1)?))),
                        None => Ok(None),
                    }
                })()
                .map_err(|e| StoreError::Backend(e.to_string()))?;

            if let Some((other, expires_at)) = conflict {
                return Err(super::lock_refusal(element, &other, expires_at));
            }
        }

        // Upsert this holder's rows: extend an existing lease, insert a new one otherwise.
        let mut locks = Vec::with_capacity(elements.len());
        for element in elements {
            let existing: Option<(String, String, i64)> =
                (|| -> rusqlite::Result<Option<(String, String, i64)>> {
                    let mut stmt = tx.prepare(
                        "SELECT id, branch, acquired_at FROM locks WHERE project = ?1 AND element = ?2 AND holder = ?3",
                    )?;
                    let mut rows = stmt.query(params![project, element.as_str(), holder])?;
                    match rows.next()? {
                        Some(row) => Ok(Some((row.get(0)?, row.get(1)?, row.get(2)?))),
                        None => Ok(None),
                    }
                })()
                .map_err(|e| StoreError::Backend(e.to_string()))?;

            let expires_at = now + ttl_seconds;
            let (id, acquired_at) = match existing {
                Some((id, _branch, acquired_at)) => {
                    tx.execute(
                        "UPDATE locks SET branch = ?1, expires_at = ?2 WHERE id = ?3",
                        params![branch, expires_at, id],
                    )
                    .map_err(|e| StoreError::Backend(e.to_string()))?;
                    (id, acquired_at)
                }
                None => {
                    let id = super::lock_id(project, branch, element, holder, now);
                    tx.execute(
                        "INSERT INTO locks (id, project, branch, element, holder, acquired_at, expires_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                        params![id, project, branch, element.as_str(), holder, now, expires_at],
                    )
                    .map_err(|e| StoreError::Backend(e.to_string()))?;
                    (id, now)
                }
            };

            locks.push(Lock {
                id,
                project: project.to_string(),
                branch: branch.to_string(),
                element: element.clone(),
                holder: holder.to_string(),
                acquired_at,
                expires_at,
            });
        }

        if let Some(audit) = audit {
            insert_audit(&tx, audit).map_err(|e| StoreError::Backend(e.to_string()))?;
        }
        tx.commit()
            .map_err(|e| StoreError::Backend(e.to_string()))?;

        Ok(locks)
    }

    fn release_locks(
        &self,
        project: &str,
        holder: &str,
        ids: &[String],
        audit: Option<&AuditEntry>,
    ) -> Result<usize, StoreError> {
        let guard = self
            .connection
            .lock()
            .map_err(|_| StoreError::Backend("connection lock poisoned".to_string()))?;
        let tx = guard
            .unchecked_transaction()
            .map_err(|e| StoreError::Backend(e.to_string()))?;

        // Only the listed holder may release, so holder and id are both pinned in the
        // WHERE clause; any other holder's attempt deletes nothing.
        let mut released = 0usize;
        for id in ids {
            released += tx
                .execute(
                    "DELETE FROM locks WHERE project = ?1 AND holder = ?2 AND id = ?3",
                    params![project, holder, id],
                )
                .map_err(|e| StoreError::Backend(e.to_string()))?;
        }

        // A release that removes nothing is not a mutation and writes no audit entry. The
        // entry, when written, rides this transaction so it cannot outlive its rows.
        if released > 0 {
            if let Some(audit) = audit {
                insert_audit(&tx, audit).map_err(|e| StoreError::Backend(e.to_string()))?;
            }
        }

        tx.commit()
            .map_err(|e| StoreError::Backend(e.to_string()))?;
        Ok(released)
    }

    fn locks(&self, project: &str, now: i64) -> Result<Vec<Lock>, StoreError> {
        let guard = self
            .connection
            .lock()
            .map_err(|_| StoreError::Backend("connection lock poisoned".to_string()))?;
        let tx = guard
            .unchecked_transaction()
            .map_err(|e| StoreError::Backend(e.to_string()))?;

        let locks: Vec<Lock> = (|| -> rusqlite::Result<Vec<Lock>> {
            let mut stmt = tx.prepare(
                "SELECT id, project, branch, element, holder, acquired_at, expires_at FROM locks WHERE project = ?1 AND expires_at > ?2 ORDER BY element",
            )?;
            let rows = stmt.query_map(params![project, now], |row| {
                Ok(Lock {
                    id: row.get(0)?,
                    project: row.get(1)?,
                    branch: row.get(2)?,
                    element: row.get(3)?,
                    holder: row.get(4)?,
                    acquired_at: row.get(5)?,
                    expires_at: row.get(6)?,
                })
            })?;
            rows.collect()
        })()
        .map_err(|e| StoreError::Backend(e.to_string()))?;

        tx.commit()
            .map_err(|e| StoreError::Backend(e.to_string()))?;
        Ok(locks)
    }

    fn holders_of(
        &self,
        project: &str,
        elements: &[String],
        now: i64,
    ) -> Result<Vec<Lock>, StoreError> {
        let guard = self
            .connection
            .lock()
            .map_err(|_| StoreError::Backend("connection lock poisoned".to_string()))?;
        let tx = guard
            .unchecked_transaction()
            .map_err(|e| StoreError::Backend(e.to_string()))?;

        let locks: Vec<Lock> = (|| -> rusqlite::Result<Vec<Lock>> {
            let mut locks = Vec::new();
            for element in elements {
                let mut stmt = tx.prepare(
                    "SELECT id, project, branch, element, holder, acquired_at, expires_at FROM locks WHERE project = ?1 AND element = ?2 AND expires_at > ?3",
                )?;
                let rows = stmt.query_map(params![project, element.as_str(), now], |row| {
                    Ok(Lock {
                        id: row.get(0)?,
                        project: row.get(1)?,
                        branch: row.get(2)?,
                        element: row.get(3)?,
                        holder: row.get(4)?,
                        acquired_at: row.get(5)?,
                        expires_at: row.get(6)?,
                    })
                })?;
                for row in rows {
                    locks.push(row?);
                }
            }
            Ok(locks)
        })()
        .map_err(|e| StoreError::Backend(e.to_string()))?;

        tx.commit()
            .map_err(|e| StoreError::Backend(e.to_string()))?;
        Ok(locks)
    }
}
