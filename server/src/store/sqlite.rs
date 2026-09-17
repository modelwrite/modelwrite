// SPDX-License-Identifier: AGPL-3.0-or-later
use std::path::Path;

use rusqlite::{params, Connection};

use super::{now_epoch, Commit, GateRun, Project, Store, StoreError};

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
    created_at TEXT NOT NULL
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
";

pub struct SqliteStore {
    connection: std::sync::Mutex<Connection>,
}

impl SqliteStore {
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        let connection = Connection::open(path)?;
        connection.execute_batch(SCHEMA)?;
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
/// message, created_at. Named so the query helpers stay readable and clippy-clean.
type CommitRow = (
    String,
    String,
    String,
    String,
    String,
    String,
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

impl Store for SqliteStore {
    fn create_project(&self, name: &str) -> Result<Project, StoreError> {
        // Check-then-insert would be a time-of-check window: two concurrent callers could
        // both pass the check. The primary key settles it instead, and an insert that
        // changed nothing is reported as the conflict it is.
        let created_at = now_epoch();
        let inserted = self.with(|c| {
            c.execute(
                "INSERT OR IGNORE INTO projects (name, created_at) VALUES (?1, ?2)",
                params![name, created_at],
            )
        })?;
        if inserted == 0 {
            return Err(StoreError::Conflict(format!(
                "project {} already exists",
                name
            )));
        }
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

    fn append_commit(&self, commit: &Commit) -> Result<(), StoreError> {
        let parents = serde_json::to_string(&commit.parents)
            .map_err(|e| StoreError::Backend(e.to_string()))?;
        // The commit row and the branch tip must move together. Two separate lock
        // acquisitions would let two concurrent appends interleave, leaving the tip
        // pointing at the earlier commit while the history lists both.
        let guard = self
            .connection
            .lock()
            .map_err(|_| StoreError::Backend("connection lock poisoned".to_string()))?;
        let tx = guard
            .unchecked_transaction()
            .map_err(|e| StoreError::Backend(e.to_string()))?;
        tx.execute(
            "INSERT OR IGNORE INTO commits (hash, project, branch, parents, okf_hash, author, message, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![commit.hash, commit.project, commit.branch, parents, commit.okf_hash, commit.author, commit.message, commit.created_at],
        )
        .map_err(|e| StoreError::Backend(e.to_string()))?;
        tx.execute(
            "INSERT INTO branches (project, name, tip) VALUES (?1, ?2, ?3) ON CONFLICT(project, name) DO UPDATE SET tip = ?3",
            params![commit.project, commit.branch, commit.hash],
        )
        .map_err(|e| StoreError::Backend(e.to_string()))?;
        tx.commit()
            .map_err(|e| StoreError::Backend(e.to_string()))?;
        Ok(())
    }

    fn commit_model(
        &self,
        project: &str,
        branch: &str,
        okf_hash: &str,
        author: &str,
        message: &str,
    ) -> Result<Commit, StoreError> {
        let guard = self
            .connection
            .lock()
            .map_err(|_| StoreError::Backend("connection lock poisoned".to_string()))?;
        let tx = guard
            .unchecked_transaction()
            .map_err(|e| StoreError::Backend(e.to_string()))?;

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
        let parents: Vec<String> = tip.into_iter().collect();
        let created_at = now_epoch();
        let hash = super::commit_hash(project, branch, &parents, okf_hash, author, message);
        let parents_json =
            serde_json::to_string(&parents).map_err(|e| StoreError::Backend(e.to_string()))?;

        tx.execute(
            "INSERT OR IGNORE INTO commits (hash, project, branch, parents, okf_hash, author, message, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![hash, project, branch, parents_json, okf_hash, author, message, created_at],
        )
        .map_err(|e| StoreError::Backend(e.to_string()))?;
        tx.execute(
            "INSERT INTO branches (project, name, tip) VALUES (?1, ?2, ?3) ON CONFLICT(project, name) DO UPDATE SET tip = ?3",
            params![project, branch, hash],
        )
        .map_err(|e| StoreError::Backend(e.to_string()))?;
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
        })
    }

    fn commit(&self, project: &str, hash: &str) -> Result<Option<Commit>, StoreError> {
        // The row is read inside the lock and the parents column is parsed outside it, so
        // that corruption can surface as a storage error rather than a query error.
        let row: Option<CommitRow> = self.with(|c| {
                let mut stmt = c.prepare(
                    "SELECT hash, project, branch, parents, okf_hash, author, message, created_at FROM commits WHERE project = ?1 AND hash = ?2",
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
                    ))),
                    None => Ok(None),
                }
            })?;

        match row {
            None => Ok(None),
            Some((hash, project, branch, parents, okf_hash, author, message, created_at)) => {
                Ok(Some(Commit {
                    parents: parse_parents(&hash, &parents)?,
                    hash,
                    project,
                    branch,
                    okf_hash,
                    author,
                    message,
                    created_at,
                }))
            }
        }
    }

    fn commits_on(&self, project: &str, branch: &str) -> Result<Vec<Commit>, StoreError> {
        // Rows are collected first and the parents column is parsed afterwards, because a
        // parse failure inside the query closure could only be reported as a rusqlite
        // error; corruption must surface as a storage error instead.
        let rows: Vec<CommitRow> = self.with(|c| {
            let mut stmt = c.prepare(
                "SELECT hash, project, branch, parents, okf_hash, author, message, created_at FROM commits WHERE project = ?1 AND branch = ?2 ORDER BY rowid",
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
                ))
            })?;
            rows.collect()
        })?;

        let mut commits = Vec::with_capacity(rows.len());
        for (hash, project, branch, parents, okf_hash, author, message, created_at) in rows {
            commits.push(Commit {
                parents: parse_parents(&hash, &parents)?,
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

    fn create_branch(&self, project: &str, name: &str, from: &str) -> Result<(), StoreError> {
        if self.commit(project, from)?.is_none() {
            return Err(StoreError::NotFound(format!("commit {}", from)));
        }
        // The composite primary key decides a duplicate branch, not a prior read.
        let inserted = self.with(|c| {
            c.execute(
                "INSERT OR IGNORE INTO branches (project, name, tip) VALUES (?1, ?2, ?3)",
                params![project, name, from],
            )
        })?;
        if inserted == 0 {
            return Err(StoreError::Conflict(format!(
                "branch {} already exists",
                name
            )));
        }
        Ok(())
    }

    fn list_branches(&self, project: &str) -> Result<Vec<(String, String)>, StoreError> {
        self.with(|c| {
            let mut stmt =
                c.prepare("SELECT name, tip FROM branches WHERE project = ?1 ORDER BY name")?;
            let rows = stmt.query_map(params![project], |row| Ok((row.get(0)?, row.get(1)?)))?;
            rows.collect()
        })
    }

    fn record_gate_run(&self, run: &GateRun) -> Result<(), StoreError> {
        self.with(|c| {
            c.execute(
                "INSERT INTO gate_runs (project, branch, reference_hash, candidate_hash, passed, evidence, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![run.project, run.branch, run.reference_hash, run.candidate_hash, run.passed as i64, run.evidence, run.created_at],
            )
        })?;
        Ok(())
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
}
