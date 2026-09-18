# Slice 2 (tranche 1) - Repository service: store and prove OKF commits

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (- [ ]) syntax for tracking.

**Goal:** turn the engine into a service a team can run on: a Rust HTTP service that stores OKF models as content-addressed commits, serves them, and runs the fidelity gate server-side with the evidence recorded.

**Architecture:** a new `server/` crate group using axum on tokio. A `Store` trait owns persistence; tranche 1 implements it over SQLite (a single file, zero configuration, works on a controlled network) and the trait boundary is what a PostgreSQL backend will implement in a later tranche. Models are stored as content-addressed blobs keyed by the sha256 of their canonical OKF bytes, and commits are content-addressed too, so the same inputs always produce the same commit hash and the same evidence - the determinism discipline the engine already follows.

This is tranche 1 of Slice 2. It deliberately does NOT include branching and merge, locks, identity and roles, the audit log, the CLI, or container deployment: those are later tranches of the same slice, each with their own plan written when it starts. What this tranche must deliver is a working, tested service that a team could already put real models into and gate.

**Tech Stack:** Rust stable (edition 2021, rust-version 1.75), axum 0.7, tokio, serde/serde_json, rusqlite (bundled SQLite), sha2/hex, anyhow, tower (dev, for in-process requests), tempfile (dev).

**Spec:** docs/superpowers/specs/2026-09-17-modelwrite-platform-design.md (sections 4.1 to 4.3, and Slice 2 of 4.3). Roadmap: docs/superpowers/plans/2026-09-17-modelwrite-capability-roadmap.md.

## Global Constraints

- Rust stable; edition 2021; rust-version 1.75; new dependencies pinned in the workspace root Cargo.toml under [workspace.dependencies].
- Every source file begins with: // SPDX-License-Identifier: AGPL-3.0-or-later
- The server is AGPL-3.0-or-later like the engine.
- Cargo is not on PATH in fresh shells: begin every shell command sequence with $env:Path = "$env:USERPROFILE\.cargo\bin;" + $env:Path
- Use --no-fail-fast when running a workspace suite.
- Stage and commit only the paths a task owns, plus Cargo.lock when that task changes it. Never amend or rewrite a commit.
- The engine's contracts are fixed: this tranche consumes them, it never changes engine/ or the corpus. OKF documents are validated with okf::validate, diffed with okf::diff, and gated with gate::run.
- Determinism: commit hashes, blob hashes and gate evidence are sha256 over canonical bytes; no clocks inside hashed payloads. A creation timestamp is stored beside a commit, never inside its hash.
- The server must never panic on malformed input: every request path returns a structured HTTP error (400 for a malformed body, 404 for a missing project or commit, 409 for a conflict, 422 for an OKF document that fails validation).
- Tests use an in-process router (tower oneshot) against a temporary SQLite file; no network ports are bound in tests.

---

### Task 1: The service skeleton

**Files:**
- Create: server/Cargo.toml
- Create: server/src/lib.rs
- Create: server/src/main.rs
- Create: server/src/error.rs
- Test: server/tests/http.rs
- Modify: Cargo.toml (workspace members and shared dependencies)

**Interfaces:**
- Consumes: nothing.
- Produces:
  - server::app(state: AppState) -> axum::Router - the whole HTTP surface as a router, so tests can drive it in-process.
  - server::AppState { pub store: std::sync::Arc<dyn Store>, pub evidence_dir: std::path::PathBuf } (Store arrives in Task 2; in this task AppState carries only evidence_dir and the router serves the two static routes).
  - server::error::ApiError with ApiError::bad_request(msg) -> (StatusCode, Json<Value>) style responses.
  - Binary mw-server, listening on 127.0.0.1:8080 by default, overridable with MW_PORT.

- [ ] **Step 1: Add the workspace dependencies and member**

In the root Cargo.toml, extend members with "server", and add to [workspace.dependencies]:

```toml
axum = "0.7"
tokio = { version = "1", features = ["rt-multi-thread", "macros", "net"] }
rusqlite = { version = "0.31", features = ["bundled"] }
tower = { version = "0.4", features = ["util"] }
http-body-util = "0.1"
tempfile = "3"
```

- [ ] **Step 2: Write server/Cargo.toml**

```toml
[package]
name = "mw-server"
version = "0.1.0"
edition.workspace = true
rust-version.workspace = true
license.workspace = true
publish = false

[lib]
name = "server"

[[bin]]
name = "mw-server"
path = "src/main.rs"

[dependencies]
mw-okf = { path = "../engine/okf" }
mw-gate = { path = "../engine/gate" }
axum.workspace = true
tokio.workspace = true
rusqlite.workspace = true
serde.workspace = true
serde_json.workspace = true
sha2.workspace = true
hex.workspace = true
anyhow.workspace = true

[dev-dependencies]
mw-test-support = { path = "../engine/test-support" }
tower.workspace = true
http-body-util.workspace = true
tempfile.workspace = true
serde_json.workspace = true
```

- [ ] **Step 3: Write server/src/error.rs**

```rust
// SPDX-License-Identifier: AGPL-3.0-or-later
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

/// Every failure path returns this: a status code and a JSON body, never a panic.
#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub message: String,
}

impl ApiError {
    pub fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: message.into(),
        }
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: message.into(),
        }
    }

    pub fn conflict(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::CONFLICT,
            message: message.into(),
        }
    }

    /// A well-formed OKF document that fails validation: the model is the problem,
    /// not the request, so the code is 422 and the report travels with it.
    pub fn unprocessable(message: impl Into<String>, errors: Vec<String>) -> Self {
        Self {
            status: StatusCode::UNPROCESSABLE_ENTITY,
            message: format!("{}: {}", message.into(), errors.join("; ")),
        }
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: message.into(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(json!({ "error": self.message }))).into_response()
    }
}
```

- [ ] **Step 4: Write server/src/lib.rs**

```rust
// SPDX-License-Identifier: AGPL-3.0-or-later
pub mod error;

use std::path::PathBuf;
use std::sync::Arc;

use axum::routing::get;
use axum::{Json, Router};
use serde_json::json;

pub use error::ApiError;

#[derive(Clone)]
pub struct AppState {
    pub evidence_dir: PathBuf,
}

/// The whole HTTP surface as a router, so tests drive it in-process and never bind a port.
pub fn app(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/version", get(version))
        .with_state(Arc::new(state))
}

async fn health() -> Json<serde_json::Value> {
    Json(json!({ "status": "ok" }))
}

async fn version() -> Json<serde_json::Value> {
    Json(json!({
        "server": env!("CARGO_PKG_VERSION"),
        "service": "mw-server"
    }))
}
```

- [ ] **Step 5: Write server/src/main.rs**

```rust
// SPDX-License-Identifier: AGPL-3.0-or-later
use std::net::SocketAddr;
use std::path::PathBuf;

use server::{app, AppState};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let port: u16 = std::env::var("MW_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(8080);
    let evidence_dir = std::env::var("MW_EVIDENCE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("docs/evidence"));

    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    println!("mw-server listening on http://{}", addr);
    axum::serve(listener, app(AppState { evidence_dir })).await?;
    Ok(())
}
```

- [ ] **Step 6: Write server/tests/http.rs**

```rust
// SPDX-License-Identifier: AGPL-3.0-or-later
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

fn state(dir: &std::path::Path) -> server::AppState {
    server::AppState {
        evidence_dir: dir.to_path_buf(),
    }
}

#[tokio::test]
async fn health_answers_ok() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    let response = router
        .oneshot(Request::builder().uri("/health").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(value["status"], "ok");
}

#[tokio::test]
async fn version_names_the_service() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    let response = router
        .oneshot(Request::builder().uri("/version").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(value["service"], "mw-server");
}
```

http-body-util supplies BodyExt, which the tests use to read response bodies; it is declared in Step 1 as a workspace dependency and in the manifest above.

- [ ] **Step 7: Run the tests**

Run: cargo test -p mw-server --no-fail-fast
Expected: 2 tests pass.

- [ ] **Step 8: Format, lint and commit**

```powershell
cargo fmt --all
cargo clippy -p mw-server --all-targets -- -D warnings
git add Cargo.toml Cargo.lock server
git commit -m "feat: add the repository service skeleton"
```

---
### Task 2: The model store (trait plus SQLite backend)

**Files:**
- Create: server/src/store/mod.rs
- Create: server/src/store/sqlite.rs
- Modify: server/src/lib.rs (declare pub mod store)
- Test: server/tests/store.rs

**Interfaces:**
- Consumes: sha2/hex for content addressing; rusqlite for persistence.
- Produces:
  - store::Store trait: create_project, project, list_projects, put_blob, blob, commit_hash, append_commit, commit, commits_on, branch_tip, create_branch, list_branches, record_gate_run, gate_runs.
  - store::SqliteStore::open(path: &std::path::Path) -> anyhow::Result<SqliteStore> which applies the schema.
  - store::{Project, Commit, GateRun, StoreError}.

- [ ] **Step 1: Write server/src/store/mod.rs**

```rust
// SPDX-License-Identifier: AGPL-3.0-or-later
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
    fn append_commit(&self, commit: &Commit) -> Result<(), StoreError>;
    fn commit(&self, project: &str, hash: &str) -> Result<Option<Commit>, StoreError>;
    fn commits_on(&self, project: &str, branch: &str) -> Result<Vec<Commit>, StoreError>;
    fn branch_tip(&self, project: &str, branch: &str) -> Result<Option<String>, StoreError>;
    fn create_branch(&self, project: &str, name: &str, from: &str) -> Result<(), StoreError>;
    fn list_branches(&self, project: &str) -> Result<Vec<(String, String)>, StoreError>;
    fn record_gate_run(&self, run: &GateRun) -> Result<(), StoreError>;
    fn gate_runs(&self, project: &str) -> Result<Vec<GateRun>, StoreError>;
}
```

- [ ] **Step 2: Write server/src/store/sqlite.rs**

```rust
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

impl Store for SqliteStore {
    fn create_project(&self, name: &str) -> Result<Project, StoreError> {
        if self.project(name)?.is_some() {
            return Err(StoreError::Conflict(format!("project {} already exists", name)));
        }
        let created_at = now_epoch();
        self.with(|c| c.execute("INSERT INTO projects (name, created_at) VALUES (?1, ?2)", params![name, created_at]))?;
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
        self.with(|c| {
            c.execute(
                "INSERT OR IGNORE INTO commits (hash, project, branch, parents, okf_hash, author, message, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![commit.hash, commit.project, commit.branch, parents, commit.okf_hash, commit.author, commit.message, commit.created_at],
            )
        })?;
        self.with(|c| {
            c.execute(
                "INSERT INTO branches (project, name, tip) VALUES (?1, ?2, ?3) ON CONFLICT(project, name) DO UPDATE SET tip = ?3",
                params![commit.project, commit.branch, commit.hash],
            )
        })?;
        Ok(())
    }

    fn commit(&self, project: &str, hash: &str) -> Result<Option<Commit>, StoreError> {
        self.with(|c| {
            let mut stmt = c.prepare(
                "SELECT hash, project, branch, parents, okf_hash, author, message, created_at FROM commits WHERE project = ?1 AND hash = ?2",
            )?;
            let mut rows = stmt.query(params![project, hash])?;
            match rows.next()? {
                Some(row) => {
                    let parents: String = row.get(3)?;
                    Ok(Some(Commit {
                        hash: row.get(0)?,
                        project: row.get(1)?,
                        branch: row.get(2)?,
                        parents: serde_json::from_str(&parents).unwrap_or_default(),
                        okf_hash: row.get(4)?,
                        author: row.get(5)?,
                        message: row.get(6)?,
                        created_at: row.get(7)?,
                    }))
                }
                None => Ok(None),
            }
        })
    }

    fn commits_on(&self, project: &str, branch: &str) -> Result<Vec<Commit>, StoreError> {
        self.with(|c| {
            let mut stmt = c.prepare(
                "SELECT hash, project, branch, parents, okf_hash, author, message, created_at FROM commits WHERE project = ?1 AND branch = ?2 ORDER BY rowid",
            )?;
            let rows = stmt.query_map(params![project, branch], |row| {
                let parents: String = row.get(3)?;
                Ok(Commit {
                    hash: row.get(0)?,
                    project: row.get(1)?,
                    branch: row.get(2)?,
                    parents: serde_json::from_str(&parents).unwrap_or_default(),
                    okf_hash: row.get(4)?,
                    author: row.get(5)?,
                    message: row.get(6)?,
                    created_at: row.get(7)?,
                })
            })?;
            rows.collect()
        })
    }

    fn branch_tip(&self, project: &str, branch: &str) -> Result<Option<String>, StoreError> {
        self.with(|c| {
            let mut stmt = c.prepare("SELECT tip FROM branches WHERE project = ?1 AND name = ?2")?;
            let mut rows = stmt.query(params![project, branch])?;
            match rows.next()? {
                Some(row) => Ok(Some(row.get(0)?)),
                None => Ok(None),
            }
        })
    }

    fn create_branch(&self, project: &str, name: &str, from: &str) -> Result<(), StoreError> {
        if self.branch_tip(project, name)?.is_some() {
            return Err(StoreError::Conflict(format!("branch {} already exists", name)));
        }
        if self.commit(project, from)?.is_none() {
            return Err(StoreError::NotFound(format!("commit {}", from)));
        }
        self.with(|c| {
            c.execute(
                "INSERT INTO branches (project, name, tip) VALUES (?1, ?2, ?3)",
                params![project, name, from],
            )
        })?;
        Ok(())
    }

    fn list_branches(&self, project: &str) -> Result<Vec<(String, String)>, StoreError> {
        self.with(|c| {
            let mut stmt = c.prepare("SELECT name, tip FROM branches WHERE project = ?1 ORDER BY name")?;
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
```

- [ ] **Step 3: Declare the module**

Add `pub mod store;` to server/src/lib.rs.

- [ ] **Step 4: Write server/tests/store.rs**

```rust
// SPDX-License-Identifier: AGPL-3.0-or-later
use server::store::{commit_hash, sqlite::SqliteStore, Commit, Store, StoreError};

fn store() -> (SqliteStore, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::open(&dir.path().join("mw.db")).unwrap();
    (store, dir)
}

fn commit_for(project: &str, branch: &str, okf_hash: &str) -> Commit {
    let parents: Vec<String> = vec![];
    let hash = commit_hash(project, branch, &parents, okf_hash, "alex", "first commit");
    Commit {
        hash,
        project: project.to_string(),
        branch: branch.to_string(),
        parents,
        okf_hash: okf_hash.to_string(),
        author: "alex".to_string(),
        message: "first commit".to_string(),
        created_at: "0".to_string(),
    }
}

#[test]
fn blobs_are_content_addressed() {
    let (store, _dir) = store();
    let a = store.put_blob(b"{\"model\":1}").unwrap();
    let b = store.put_blob(b"{\"model\":1}").unwrap();
    let c = store.put_blob(b"{\"model\":2}").unwrap();
    assert_eq!(a, b, "identical bytes must share an address");
    assert_ne!(a, c);
    assert_eq!(store.blob(&a).unwrap().unwrap(), b"{\"model\":1}");
}

#[test]
fn a_commit_hash_ignores_time_but_not_content() {
    let parents = vec!["p1".to_string()];
    let a = commit_hash("p", "main", &parents, "okf1", "alex", "m");
    let b = commit_hash("p", "main", &parents, "okf1", "alex", "m");
    let c = commit_hash("p", "main", &parents, "okf2", "alex", "m");
    assert_eq!(a, b);
    assert_ne!(a, c);
}

#[test]
fn commits_land_on_a_branch_and_move_its_tip() {
    let (store, _dir) = store();
    store.create_project("coffee").unwrap();
    let first = commit_for("coffee", "main", "okf1");
    store.append_commit(&first).unwrap();
    assert_eq!(store.branch_tip("coffee", "main").unwrap().unwrap(), first.hash);

    let second = commit_for("coffee", "main", "okf2");
    store.append_commit(&second).unwrap();
    assert_eq!(store.branch_tip("coffee", "main").unwrap().unwrap(), second.hash);
    assert_eq!(store.commits_on("coffee", "main").unwrap().len(), 2);
}

#[test]
fn duplicate_projects_and_branches_are_conflicts() {
    let (store, _dir) = store();
    store.create_project("coffee").unwrap();
    match store.create_project("coffee") {
        Err(StoreError::Conflict(_)) => {}
        other => panic!("expected a conflict, got {:?}", other),
    }
    let commit = commit_for("coffee", "main", "okf1");
    store.append_commit(&commit).unwrap();
    store.create_branch("coffee", "review", &commit.hash).unwrap();
    match store.create_branch("coffee", "review", &commit.hash) {
        Err(StoreError::Conflict(_)) => {}
        other => panic!("expected a conflict, got {:?}", other),
    }
    match store.create_branch("coffee", "other", "missing-commit") {
        Err(StoreError::NotFound(_)) => {}
        other => panic!("expected not found, got {:?}", other),
    }
}
```

- [ ] **Step 5: Run the tests**

Run: cargo test -p mw-server --no-fail-fast
Expected: 6 tests pass (2 from Task 1 plus 4 here).

- [ ] **Step 6: Format, lint and commit**

```powershell
cargo fmt --all
cargo clippy -p mw-server --all-targets -- -D warnings
git add Cargo.toml Cargo.lock server
git commit -m "feat: add the content-addressed model store"
```

---
ROUTE SYNTAX: the workspace pins axum 0.7, which matches routes as :param, not {param}. The router lines above use :project and :hash accordingly; axum 0.8 syntax silently 404s on 0.7, which is how this was found. Prose endpoint descriptions elsewhere in this plan keep the readable {project} form.

### Task 3: The project and commit API

**Files:**
- Create: server/src/api.rs
- Modify: server/src/lib.rs (AppState gains the store; the router gains the routes)
- Create: server/tests/api.rs

**Interfaces:**
- Consumes: store::{Store, SqliteStore, commit_hash, now_epoch, Commit}; okf::{types::OkfRoot, validate}.
- Produces these routes:
  - POST /projects {"name": string} -> 201 {"name", "createdAt"}; 409 if it exists.
  - GET /projects -> 200 [{"name", "createdAt"}]
  - POST /projects/{project}/commits {"branch", "author", "message", "okf": object} -> 201 {"hash", "okfHash", "branch", "parents"}; 422 with the validation errors if the OKF document is invalid; 404 if the project is missing.
  - GET /projects/{project}/commits?branch=main -> 200 [commit]
  - GET /projects/{project}/commits/{hash} -> 200 the stored OKF document, byte for byte; 404 if unknown.
  - POST /projects/{project}/branches {"name", "from"} -> 201; 409 if the branch exists; 404 if the commit is unknown.

- [ ] **Step 1: Write server/src/api.rs**

```rust
// SPDX-License-Identifier: AGPL-3.0-or-later
use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::error::ApiError;
use crate::store::{commit_hash, now_epoch, Commit, Store, StoreError};
use okf::types::OkfRoot;

#[derive(Clone)]
pub struct ApiState {
    pub store: Arc<dyn Store>,
    pub evidence_dir: std::path::PathBuf,
}

pub fn map_store_error(e: StoreError) -> ApiError {
    match e {
        StoreError::NotFound(m) => ApiError::not_found(m),
        StoreError::Conflict(m) => ApiError::conflict(m),
        StoreError::Backend(m) => ApiError::internal(m),
    }
}

fn commit_json(commit: &Commit) -> Value {
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
    if body.name.trim().is_empty() {
        return Err(ApiError::bad_request("project name must not be empty"));
    }
    let project = state.store.create_project(&body.name).map_err(map_store_error)?;
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
}

pub async fn create_commit(
    State(state): State<ApiState>,
    Path(project): Path<String>,
    Json(body): Json<CreateCommit>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    if state.store.project(&project).map_err(map_store_error)?.is_none() {
        return Err(ApiError::not_found(format!("project {}", project)));
    }
    if body.branch.trim().is_empty() {
        return Err(ApiError::bad_request("branch must not be empty"));
    }

    // The document must be a valid OKF model before it is stored: a repository that
    // accepts invalid models cannot be gated meaningfully.
    let bytes = serde_json::to_vec(&body.okf).map_err(|e| ApiError::bad_request(e.to_string()))?;
    let root: okf::types::OkfRoot =
        serde_json::from_slice(&bytes).map_err(|e| ApiError::bad_request(format!("not an OKF document: {}", e)))?;
    let report = okf::validate::validate(&root);
    if !report.valid {
        return Err(ApiError::unprocessable("the model failed validation", report.errors));
    }

    let okf_hash = state.store.put_blob(&bytes).map_err(map_store_error)?;
    let parents: Vec<String> = state
        .store
        .branch_tip(&project, &body.branch)
        .map_err(map_store_error)?
        .into_iter()
        .collect();
    let hash = commit_hash(
        &project,
        &body.branch,
        &parents,
        &okf_hash,
        &body.author,
        &body.message,
    );
    let commit = Commit {
        hash: hash.clone(),
        project: project.clone(),
        branch: body.branch.clone(),
        parents: parents.clone(),
        okf_hash,
        author: body.author,
        message: body.message,
        created_at: now_epoch(),
    };
    state.store.append_commit(&commit).map_err(map_store_error)?;
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
    let commits = state.store.commits_on(&project, &branch).map_err(map_store_error)?;
    Ok(Json(Value::Array(commits.iter().map(commit_json).collect())))
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
        .ok_or_else(|| ApiError::internal(format!("missing blob {}", commit.okf_hash)))?;
    let value: Value =
        serde_json::from_slice(&bytes).map_err(|e| ApiError::internal(e.to_string()))?;
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
    state
        .store
        .create_branch(&project, &body.name, &body.from)
        .map_err(map_store_error)?;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "name": body.name, "tip": body.from })),
    ))
}
```

- [ ] **Step 2: Wire the routes and the store into server/src/lib.rs**

```rust
// SPDX-License-Identifier: AGPL-3.0-or-later
pub mod api;
pub mod error;
pub mod store;

pub use error::ApiError;

use std::sync::Arc;

use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::json;

#[derive(Clone)]
pub struct AppState {
    pub store: Arc<dyn store::Store>,
    pub evidence_dir: std::path::PathBuf,
}

impl AppState {
    fn api(&self) -> api::ApiState {
        api::ApiState {
            store: self.store.clone(),
            evidence_dir: self.evidence_dir.clone(),
        }
    }
}

pub fn app(state: AppState) -> Router {
    let api_state = state.api();
    Router::new()
        .route("/health", get(health))
        .route("/version", get(version))
        .route("/projects", post(api::create_project).get(api::list_projects))
        .route(
            "/projects/:project/commits",
            post(api::create_commit).get(api::list_commits),
        )
        .route("/projects/:project/commits/:hash", get(api::get_commit))
        .route("/projects/:project/branches", post(api::create_branch))
        .with_state(api_state)
}

async fn health() -> Json<serde_json::Value> {
    Json(json!({ "status": "ok" }))
}

async fn version() -> Json<serde_json::Value> {
    Json(json!({
        "server": env!("CARGO_PKG_VERSION"),
        "service": "mw-server"
    }))
}
```

Inside this crate the engine library is reachable as `okf` because its [lib] name is okf, so api.rs and gate_api.rs refer to okf::types and okf::validate directly.

- [ ] **Step 3: Update server/src/main.rs and the Task 1 tests for the new state**

main.rs opens the store and passes it:

```rust
// SPDX-License-Identifier: AGPL-3.0-or-later
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use server::store::sqlite::SqliteStore;
use server::{app, AppState};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let port: u16 = std::env::var("MW_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(8080);
    let db_path = std::env::var("MW_DB").unwrap_or_else(|_| "modelwrite.db".to_string());
    let evidence_dir = std::env::var("MW_EVIDENCE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("docs/evidence"));

    let store = Arc::new(SqliteStore::open(std::path::Path::new(&db_path))?);
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    println!("mw-server listening on http://{} (db {})", addr, db_path);
    axum::serve(listener, app(AppState { store, evidence_dir })).await?;
    Ok(())
}
```

server/tests/http.rs builds state with a temporary store:

```rust
fn state(dir: &std::path::Path) -> server::AppState {
    let store = server::store::sqlite::SqliteStore::open(&dir.join("mw.db")).unwrap();
    server::AppState {
        store: std::sync::Arc::new(store),
        evidence_dir: dir.to_path_buf(),
    }
}
```

- [ ] **Step 4: Write server/tests/api.rs**

```rust
// SPDX-License-Identifier: AGPL-3.0-or-later
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

fn state(dir: &std::path::Path) -> server::AppState {
    let store = server::store::sqlite::SqliteStore::open(&dir.join("mw.db")).unwrap();
    server::AppState {
        store: std::sync::Arc::new(store),
        evidence_dir: dir.to_path_buf(),
    }
}

fn post(uri: &str, body: serde_json::Value) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

async fn json_body(response: axum::response::Response) -> serde_json::Value {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

fn tiny_okf() -> serde_json::Value {
    serde_json::json!({
        "project": "tiny",
        "exportedAt": "2026-09-17T00:00:00Z",
        "summary": {},
        "stateMachine": { "name": "tiny sm", "regions": [] },
        "graph": { "nodes": [{ "id": "b1", "kind": "block", "name": "B1" }], "edges": [] }
    })
}

#[tokio::test]
async fn a_project_can_be_created_listed_and_is_not_duplicated() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));

    let created = router
        .clone()
        .oneshot(post("/projects", serde_json::json!({ "name": "coffee" })))
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::CREATED);

    let duplicate = router
        .clone()
        .oneshot(post("/projects", serde_json::json!({ "name": "coffee" })))
        .await
        .unwrap();
    assert_eq!(duplicate.status(), StatusCode::CONFLICT);

    let listed = router
        .oneshot(
            Request::builder()
                .uri("/projects")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let value = json_body(listed).await;
    assert_eq!(value[0]["name"], "coffee");
}

#[tokio::test]
async fn a_commit_stores_the_model_and_moves_the_branch() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    router
        .clone()
        .oneshot(post("/projects", serde_json::json!({ "name": "coffee" })))
        .await
        .unwrap();

    let committed = router
        .clone()
        .oneshot(post(
            "/projects/coffee/commits",
            serde_json::json!({
                "branch": "main",
                "author": "alex",
                "message": "import the model",
                "okf": tiny_okf()
            }),
        ))
        .await
        .unwrap();
    assert_eq!(committed.status(), StatusCode::CREATED);
    let commit = json_body(committed).await;
    assert!(commit["hash"].as_str().unwrap().len() == 64);
    assert!(commit["parents"].as_array().unwrap().is_empty());

    let hash = commit["hash"].as_str().unwrap().to_string();
    let fetched = router
        .oneshot(
            Request::builder()
                .uri(format!("/projects/coffee/commits/{}", hash))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(fetched.status(), StatusCode::OK);
    assert_eq!(json_body(fetched).await, tiny_okf());
}

#[tokio::test]
async fn an_invalid_model_is_rejected_with_its_errors() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    router
        .clone()
        .oneshot(post("/projects", serde_json::json!({ "name": "coffee" })))
        .await
        .unwrap();

    let mut okf = tiny_okf();
    okf["graph"] = serde_json::Value::Null;
    let rejected = router
        .oneshot(post(
            "/projects/coffee/commits",
            serde_json::json!({
                "branch": "main",
                "author": "alex",
                "message": "broken",
                "okf": okf
            }),
        ))
        .await
        .unwrap();
    assert_eq!(rejected.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body = json_body(rejected).await;
    assert!(body["error"].as_str().unwrap().contains("graph section is missing"));
}

#[tokio::test]
async fn committing_an_unknown_project_is_not_found() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    let response = router
        .oneshot(post(
            "/projects/missing/commits",
            serde_json::json!({
                "branch": "main",
                "author": "alex",
                "message": "nothing",
                "okf": tiny_okf()
            }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}
```

- [ ] **Step 5: Run the tests**

Run: cargo test -p mw-server --no-fail-fast
Expected: 10 tests pass (2 skeleton, 4 store, 4 here).

- [ ] **Step 6: Format, lint and commit**

```powershell
cargo fmt --all
cargo clippy -p mw-server --all-targets -- -D warnings
git add Cargo.toml Cargo.lock server
git commit -m "feat: add the project and commit API"
```

---
### Task 4: The gate endpoint and server-side evidence

**Files:**
- Create: server/src/gate_api.rs
- Modify: server/src/lib.rs (declare the module and add the two routes)
- Create: server/tests/gate_api.rs

**Interfaces:**
- Consumes: store::{Store, GateRun, now_epoch}; gate::{run, write_evidence}; okf::types::OkfRoot.
- Produces:
  - POST /projects/{project}/gate {"reference": commit-hash, "candidate": commit-hash} -> 200 the gate evidence JSON; the run is recorded in the store and its evidence is written to the evidence directory. The HTTP status is 200 whether the gate passed or failed: the body carries "passed", and a failed gate is a successful gate run, not a transport error.
  - GET /projects/{project}/gate-runs -> 200 [{project, branch, referenceHash, candidateHash, passed, evidence, createdAt}]

- [ ] **Step 1: Write server/src/gate_api.rs**

```rust
// SPDX-License-Identifier: AGPL-3.0-or-later
use axum::extract::{Path, State};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::api::{map_store_error, ApiState};
use crate::error::ApiError;
use crate::store::{now_epoch, GateRun};

/// Load the OKF document behind a commit hash.
fn load_model(state: &ApiState, project: &str, hash: &str) -> Result<okf::types::OkfRoot, ApiError> {
    let commit = state
        .store
        .commit(project, hash)
        .map_err(map_store_error)?
        .ok_or_else(|| ApiError::not_found(format!("commit {}", hash)))?;
    let bytes = state
        .store
        .blob(&commit.okf_hash)
        .map_err(map_store_error)?
        .ok_or_else(|| ApiError::internal(format!("missing blob {}", commit.okf_hash)))?;
    serde_json::from_slice(&bytes).map_err(|e| ApiError::internal(e.to_string()))
}

fn short(hash: &str) -> &str {
    let end = 12.min(hash.len());
    &hash[..end]
}

#[derive(Deserialize)]
pub struct GateRequest {
    pub reference: String,
    pub candidate: String,
}

pub async fn run_gate(
    State(state): State<ApiState>,
    Path(project): Path<String>,
    Json(body): Json<GateRequest>,
) -> Result<Json<Value>, ApiError> {
    let reference = load_model(&state, &project, &body.reference)?;
    let candidate = load_model(&state, &project, &body.candidate)?;

    let outcome = gate::run(&reference, &candidate, false);
    let branch = state
        .store
        .commit(&project, &body.candidate)
        .map_err(map_store_error)?
        .map(|c| c.branch)
        .unwrap_or_else(|| "main".to_string());

    // Evidence lands beside the engine's records under a deterministic name: the same
    // two commits always produce the same file, so a run is reproducible and citable.
    let file_name = format!(
        "server-{}-{}-{}.json",
        project,
        short(&body.reference),
        short(&body.candidate)
    );
    let path = state.evidence_dir.join(file_name);
    gate::write_evidence(&path, &outcome.evidence).map_err(|e| ApiError::internal(e.to_string()))?;

    let run = GateRun {
        project: project.clone(),
        branch,
        reference_hash: body.reference.clone(),
        candidate_hash: body.candidate.clone(),
        passed: outcome.passed,
        evidence: outcome.evidence.to_string(),
        created_at: now_epoch(),
    };
    state.store.record_gate_run(&run).map_err(map_store_error)?;

    Ok(Json(outcome.evidence))
}

pub async fn list_gate_runs(
    State(state): State<ApiState>,
    Path(project): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let runs = state.store.gate_runs(&project).map_err(map_store_error)?;
    let out: Vec<Value> = runs
        .iter()
        .map(|r| {
            json!({
                "project": r.project,
                "branch": r.branch,
                "referenceHash": r.reference_hash,
                "candidateHash": r.candidate_hash,
                "passed": r.passed,
                "evidence": serde_json::from_str::<Value>(&r.evidence).unwrap_or(Value::Null),
                "createdAt": r.created_at
            })
        })
        .collect();
    Ok(Json(Value::Array(out)))
}
```

Both map_store_error and ApiState are public in server/src/api.rs, so this module reuses them rather than duplicating the mapping.

- [ ] **Step 2: Add the routes**

In server/src/lib.rs add `pub mod gate_api;` and, before `.with_state(api_state)`:

```rust
        .route("/projects/:project/gate", post(gate_api::run_gate))
        .route("/projects/:project/gate-runs", get(gate_api::list_gate_runs))
```

- [ ] **Step 3: Write server/tests/gate_api.rs**

```rust
// SPDX-License-Identifier: AGPL-3.0-or-later
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

fn state(dir: &std::path::Path) -> server::AppState {
    let store = server::store::sqlite::SqliteStore::open(&dir.join("mw.db")).unwrap();
    server::AppState {
        store: std::sync::Arc::new(store),
        evidence_dir: dir.to_path_buf(),
    }
}

fn post(uri: &str, body: serde_json::Value) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

fn get(uri: &str) -> Request<Body> {
    Request::builder().uri(uri).body(Body::empty()).unwrap()
}

async fn json_body(response: axum::response::Response) -> serde_json::Value {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

fn model(name: &str) -> serde_json::Value {
    serde_json::json!({
        "project": name,
        "exportedAt": "2026-09-17T00:00:00Z",
        "summary": {},
        "stateMachine": { "name": "sm", "regions": [] },
        "graph": {
            "nodes": [
                { "id": "b1", "kind": "block", "name": "Block" },
                { "id": "r1", "kind": "requirement", "name": "Req" }
            ],
            "edges": [
                { "source": "b1", "target": "r1", "kind": "dependency", "label": "Satisfy" }
            ]
        },
        "requirements": [
            { "id": "r1", "name": "Req", "kind": "requirement", "stereotypes": ["Requirement"], "attributes": [], "documentation": "", "reqId": "1.1", "reqText": "shall satisfy" }
        ]
    })
}

async fn seed_project(router: &axum::Router, dir: &std::path::Path) -> (String, String) {
    router
        .clone()
        .oneshot(post("/projects", serde_json::json!({ "name": "coffee" })))
        .await
        .unwrap();

    let first = router
        .clone()
        .oneshot(post(
            "/projects/coffee/commits",
            serde_json::json!({ "branch": "main", "author": "alex", "message": "model", "okf": model("coffee") }),
        ))
        .await
        .unwrap();
    assert_eq!(first.status(), StatusCode::CREATED);
    let first = json_body(first).await;
    let reference = first["hash"].as_str().unwrap().to_string();

    let mut changed = model("coffee");
    changed["project"] = serde_json::json!("coffee v2");
    let second = router
        .clone()
        .oneshot(post(
            "/projects/coffee/commits",
            serde_json::json!({ "branch": "main", "author": "alex", "message": "changed", "okf": changed }),
        ))
        .await
        .unwrap();
    let second = json_body(second).await;
    let candidate = second["hash"].as_str().unwrap().to_string();

    let _ = dir;
    (reference, candidate)
}

#[tokio::test]
async fn gating_a_commit_against_itself_passes_and_records_evidence() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    let (reference, _candidate) = seed_project(&router, dir.path()).await;

    let response = router
        .clone()
        .oneshot(post(
            "/projects/coffee/gate",
            serde_json::json!({ "reference": reference, "candidate": reference }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let evidence = json_body(response).await;
    assert_eq!(evidence["passed"], true);
    assert_eq!(evidence["roundtrip"]["equal"], true);

    let files: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.starts_with("server-"))
        .collect();
    assert_eq!(files.len(), 1, "expected one evidence file, got {:?}", files);
}

#[tokio::test]
async fn gating_a_changed_model_fails_and_is_still_a_successful_run() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    let (reference, candidate) = seed_project(&router, dir.path()).await;

    let response = router
        .clone()
        .oneshot(post(
            "/projects/coffee/gate",
            serde_json::json!({ "reference": reference, "candidate": candidate }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let evidence = json_body(response).await;
    assert_eq!(evidence["passed"], false);
    assert!(!evidence["roundtrip"]["equal"].as_bool().unwrap());
    assert!(evidence["failures"]
        .as_array()
        .unwrap()
        .iter()
        .any(|f| f.as_str().unwrap().contains("roundtrip")));

    let runs = router
        .oneshot(get("/projects/coffee/gate-runs"))
        .await
        .unwrap();
    let recorded = json_body(runs).await;
    assert_eq!(recorded.as_array().unwrap().len(), 1);
    assert_eq!(recorded[0]["passed"], false);
}

#[tokio::test]
async fn gating_an_unknown_commit_is_not_found() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    let (reference, _candidate) = seed_project(&router, dir.path()).await;
    let response = router
        .oneshot(post(
            "/projects/coffee/gate",
            serde_json::json!({ "reference": reference, "candidate": "missing" }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}
```

A changed model must report roundtrip inequality, which the assertion above checks.

- [ ] **Step 4: Run the tests**

Run: cargo test -p mw-server --no-fail-fast
Expected: 13 tests pass.

- [ ] **Step 5: Format, lint and commit**

```powershell
cargo fmt --all
cargo clippy -p mw-server --all-targets -- -D warnings
git add Cargo.toml Cargo.lock server
git commit -m "feat: add the server-side gate endpoint and evidence recording"
```

---

### Task 5: The corpus end-to-end test

**Files:**
- Create: server/tests/corpus.rs

**Interfaces:**
- Consumes: test_support::{load_okf_expected, load_okf_broken} and everything the earlier tasks built.
- Produces: the proof that a real model can be committed to the service, branched, and gated, using the same corpus and the same corrupted fixture the engine tests use.

- [ ] **Step 1: Write server/tests/corpus.rs**

```rust
// SPDX-License-Identifier: AGPL-3.0-or-later
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

fn state(dir: &std::path::Path) -> server::AppState {
    let store = server::store::sqlite::SqliteStore::open(&dir.join("mw.db")).unwrap();
    server::AppState {
        store: std::sync::Arc::new(store),
        evidence_dir: dir.to_path_buf(),
    }
}

fn post(uri: &str, body: serde_json::Value) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

async fn json_body(response: axum::response::Response) -> serde_json::Value {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

async fn commit(router: &axum::Router, branch: &str, message: &str, okf: serde_json::Value) -> String {
    let response = router
        .clone()
        .oneshot(post(
            "/projects/coffee/commits",
            serde_json::json!({ "branch": branch, "author": "alex", "message": message, "okf": okf }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    json_body(response).await["hash"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn the_corpus_can_be_committed_branched_and_gated() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));

    router
        .clone()
        .oneshot(post("/projects", serde_json::json!({ "name": "coffee" })))
        .await
        .unwrap();

    let expected: serde_json::Value = serde_json::from_str(&test_support::load_okf_expected()).unwrap();
    let broken: serde_json::Value = serde_json::from_str(&test_support::load_okf_broken()).unwrap();

    let imported = commit(&router, "main", "import the exported model", expected).await;
    router
        .clone()
        .oneshot(post(
            "/projects/coffee/branches",
            serde_json::json!({ "name": "corrupted", "from": imported }),
        ))
        .await
        .unwrap();
    let corrupted = commit(&router, "corrupted", "drop a requirement", broken).await;

    let clean = router
        .clone()
        .oneshot(post(
            "/projects/coffee/gate",
            serde_json::json!({ "reference": imported, "candidate": imported }),
        ))
        .await
        .unwrap();
    let clean = json_body(clean).await;
    assert_eq!(clean["passed"], true);
    assert_eq!(clean["integration"]["componentCount"], 1);
    assert_eq!(clean["coverage"]["total"], 25);

    let lossy = router
        .clone()
        .oneshot(post(
            "/projects/coffee/gate",
            serde_json::json!({ "reference": imported, "candidate": corrupted }),
        ))
        .await
        .unwrap();
    let lossy = json_body(lossy).await;
    assert_eq!(lossy["passed"], false);
    let failures: Vec<String> = lossy["failures"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f.as_str().unwrap().to_string())
        .collect();
    assert!(failures.iter().any(|f| f.contains("missing elements")));
    assert!(failures.iter().any(|f| f.contains("isolated")));

    let runs = router
        .oneshot(Request::builder()
            .uri("/projects/coffee/gate-runs")
            .body(Body::empty())
            .unwrap())
        .await
        .unwrap();
    let runs = json_body(runs).await;
    assert_eq!(runs.as_array().unwrap().len(), 2);
}
```

- [ ] **Step 2: Run the tests**

Run: cargo test -p mw-server --no-fail-fast
Expected: 14 tests pass.

- [ ] **Step 3: Run the whole workspace, format, lint and commit**

```powershell
cargo test --workspace --no-fail-fast
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
git add Cargo.toml Cargo.lock server
git commit -m "test: prove the corpus can be committed, branched and gated through the service"
```

---

## Completion criteria

- [ ] cargo test --workspace --no-fail-fast passes, including the 14 service tests.
- [ ] cargo fmt --all -- --check and cargo clippy --workspace --all-targets -- -D warnings are clean.
- [ ] The service stores the corpus as a commit, serves it back byte for byte, creates a branch from it, and gates a branch against main: the clean comparison passes and the corrupted comparison fails with the loss named.
- [ ] A gate run writes an evidence file whose name is derived from the two commit hashes, and records the run in the store.
- [ ] No request path panics: malformed JSON, a missing project, a missing commit and an invalid model each return a structured status code.

## What tranche 2 of Slice 2 must add (not in this plan)

- Branch listing and deletion, three-way merge with conflict reporting at element granularity, and revert.
- Element-level locks with leases, and optimistic check-and-set on commit.
- Identity (OIDC and LDAP), roles, and per-project scoping; a token for CLI and CI use.
- An append-only audit log for commits, locks, permission changes and gate runs.
- A PostgreSQL backend implementing the same Store trait, selecting SQLite for single-user installs.
- The modelwrite CLI, and the docker compose and Helm deployment skeleton.

