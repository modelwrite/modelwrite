// SPDX-License-Identifier: AGPL-3.0-or-later
use postgres::config::SslMode;
use postgres::NoTls;
use r2d2_postgres::PostgresConnectionManager;
use tokio_postgres_rustls::MakeRustlsConnect;

use super::{
    now_epoch, AuditEntry, Commit, CommitGuard, GateRun, Lock, Project, Store, StoreError,
};

/// The schema, ported from the SQLite reference implementation. TEXT stays TEXT, the
/// INTEGER seconds columns become BIGINT, the audit id becomes BIGSERIAL (a monotonic
/// insertion key, so the log is ordered by insertion rather than wall-clock time), and the
/// implicit SQLite rowid that ordered commits becomes an explicit BIGSERIAL PRIMARY KEY id
/// for the same reason. Every CHECK constraint is carried over, and the append-only
/// property of the audit table is enforced by triggers exactly as it is on SQLite. The
/// locks table carries a UNIQUE(project, element) constraint so "one holder per element"
/// is a property of the DATABASE, not of the acquire code: two concurrent acquires by
/// different holders serialise on the constraint and only one can win.
const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS projects (
    name TEXT PRIMARY KEY,
    created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS blobs (
    hash TEXT PRIMARY KEY,
    bytes BYTEA NOT NULL
);
CREATE TABLE IF NOT EXISTS commits (
    id BIGSERIAL PRIMARY KEY,
    hash TEXT NOT NULL UNIQUE,
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
    id BIGSERIAL PRIMARY KEY,
    project TEXT NOT NULL,
    branch TEXT NOT NULL,
    reference_hash TEXT NOT NULL,
    candidate_hash TEXT NOT NULL,
    passed BOOLEAN NOT NULL,
    evidence TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS locks (
    id TEXT PRIMARY KEY,
    project TEXT NOT NULL,
    branch TEXT NOT NULL,
    element TEXT NOT NULL,
    holder TEXT NOT NULL,
    acquired_at BIGINT NOT NULL,
    expires_at BIGINT NOT NULL,
    UNIQUE (project, element)
);
CREATE TABLE IF NOT EXISTS audit (
    id BIGSERIAL PRIMARY KEY,
    project TEXT NOT NULL,
    at BIGINT NOT NULL,
    actor TEXT NOT NULL,
    mechanism TEXT NOT NULL,
    action TEXT NOT NULL,
    subject TEXT NOT NULL,
    detail TEXT NOT NULL,
    CHECK (length(actor) > 0),
    CHECK (length(mechanism) > 0),
    CHECK (length(action) > 0)
);
CREATE OR REPLACE FUNCTION audit_append_only() RETURNS trigger AS $function$
BEGIN
    RAISE EXCEPTION 'audit entries are append-only';
END;
$function$ LANGUAGE plpgsql;
DROP TRIGGER IF EXISTS audit_no_update ON audit;
CREATE TRIGGER audit_no_update
BEFORE UPDATE ON audit
FOR EACH ROW
EXECUTE FUNCTION audit_append_only();
DROP TRIGGER IF EXISTS audit_no_delete ON audit;
CREATE TRIGGER audit_no_delete
BEFORE DELETE ON audit
FOR EACH ROW
EXECUTE FUNCTION audit_append_only();
";

type PlainPool = r2d2::Pool<PostgresConnectionManager<NoTls>>;
type TlsPool = r2d2::Pool<PostgresConnectionManager<MakeRustlsConnect>>;

enum Pool {
    Plain(PlainPool),
    Tls(TlsPool),
}

impl Pool {
    fn get_conn(&self) -> Result<PooledClient, StoreError> {
        match self {
            Pool::Plain(pool) => pool.get().map(PooledClient::Plain).map_err(backend),
            Pool::Tls(pool) => pool.get().map(PooledClient::Tls).map_err(backend),
        }
    }
}

enum PooledClient {
    Plain(r2d2::PooledConnection<PostgresConnectionManager<NoTls>>),
    Tls(r2d2::PooledConnection<PostgresConnectionManager<MakeRustlsConnect>>),
}

impl PooledClient {
    fn client(&mut self) -> &mut postgres::Client {
        match self {
            PooledClient::Plain(conn) => conn,
            PooledClient::Tls(conn) => conn,
        }
    }
}

pub struct PostgresStore {
    pool: Pool,
}

fn backend<E: std::fmt::Display>(error: E) -> StoreError {
    StoreError::Backend(error.to_string())
}

fn rustls_connector() -> Result<MakeRustlsConnect, StoreError> {
    let mut roots = rustls::RootCertStore::empty();
    let loaded = rustls_native_certs::load_native_certs();
    for certificate in loaded.certs {
        roots.add(certificate).map_err(backend)?;
    }
    let config = rustls::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    Ok(MakeRustlsConnect::new(config))
}

impl PostgresStore {
    pub fn open(url: &str) -> Result<Self, StoreError> {
        let config: postgres::Config = url.parse().map_err(backend)?;
        let pool = match config.get_ssl_mode() {
            // `disable` is an explicit opt-out of transport security, so plaintext is
            // exactly what was asked for.
            SslMode::Disable => {
                let manager = PostgresConnectionManager::new(config, NoTls);
                Pool::Plain(r2d2::Pool::new(manager).map_err(backend)?)
            }
            // `prefer` asks for TLS with a plaintext fallback. This client cannot negotiate
            // that fallback, so it honours the preference by REQUIRING TLS instead of
            // silently downgrading a transport-security setting to plaintext.
            SslMode::Prefer | SslMode::Require => {
                let manager = PostgresConnectionManager::new(config, rustls_connector()?);
                Pool::Tls(r2d2::Pool::new(manager).map_err(backend)?)
            }
            _ => {
                return Err(StoreError::Backend(
                    "unsupported sslmode in the connection string".to_string(),
                ))
            }
        };
        {
            let mut conn = pool.get_conn()?;
            // Creating the schema is guarded by an advisory lock, because `CREATE TABLE IF
            // NOT EXISTS` is NOT safe against a concurrent creator: two connections can both
            // find the table missing, both attempt the create, and the loser fails with a
            // duplicate-key error on the system catalogue rather than doing nothing. That is
            // not a test artefact - two service replicas starting at the same moment against
            // one database hit it exactly the same way, and CI found it the first time these
            // tests ever ran concurrently (six of thirteen failed opening the store).
            //
            // The lock is transaction-scoped, so it is released when the DDL commits or
            // fails, and it is keyed by a constant chosen for this schema alone.
            let mut tx = conn.client().transaction().map_err(backend)?;
            tx.batch_execute("SELECT pg_advisory_xact_lock(7071175)")
                .map_err(backend)?;
            tx.batch_execute(SCHEMA).map_err(backend)?;
            tx.commit().map_err(backend)?;
        }
        Ok(Self { pool })
    }

    fn with_client<T>(
        &self,
        f: impl FnOnce(&mut postgres::Client) -> Result<T, StoreError>,
    ) -> Result<T, StoreError> {
        let mut conn = self.pool.get_conn()?;
        f(conn.client())
    }

    fn with_tx<T>(
        &self,
        f: impl FnOnce(&mut postgres::Transaction<'_>) -> Result<T, StoreError>,
    ) -> Result<T, StoreError> {
        let mut conn = self.pool.get_conn()?;
        let mut tx = conn.client().transaction().map_err(backend)?;
        let result = f(&mut tx);
        if result.is_ok() {
            tx.commit().map_err(backend)?;
        }
        result
    }
}

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

fn parse_parents(hash: &str, raw: &str) -> Result<Vec<String>, StoreError> {
    serde_json::from_str(raw).map_err(|e| {
        StoreError::Backend(format!("corrupt parents column for commit {}: {}", hash, e))
    })
}

fn insert_audit<G: postgres::GenericClient>(
    client: &mut G,
    entry: &AuditEntry,
) -> Result<i64, StoreError> {
    let row = client
        .query_one(
            "INSERT INTO audit (project, at, actor, mechanism, action, subject, detail) VALUES ($1, $2, $3, $4, $5, $6, $7) RETURNING id",
            &[
                &entry.project,
                &entry.at,
                &entry.actor,
                &entry.mechanism,
                &entry.action,
                &entry.subject,
                &entry.detail,
            ],
        )
        .map_err(backend)?;
    Ok(row.get(0))
}

fn enforce_guard<G: postgres::GenericClient>(
    client: &mut G,
    project: &str,
    guard: &CommitGuard<'_>,
) -> Result<(), StoreError> {
    for element in guard.elements {
        let held: Option<(String, i64)> = client
            .query_opt(
                "SELECT holder, expires_at FROM locks WHERE project = $1 AND element = $2 AND expires_at > $3 AND holder != $4 LIMIT 1",
                &[&project, &element, &guard.now, &guard.holder],
            )
            .map_err(backend)?
            .map(|row| (row.get(0), row.get(1)));
        if let Some((holder, expires_at)) = held {
            return Err(super::lock_refusal(element, &holder, expires_at));
        }
    }
    Ok(())
}

impl Store for PostgresStore {
    fn create_project(
        &self,
        name: &str,
        audit: Option<&AuditEntry>,
    ) -> Result<Project, StoreError> {
        let created_at = now_epoch();
        self.with_tx(|tx| {
            let inserted = tx
                .execute(
                    "INSERT INTO projects (name, created_at) VALUES ($1, $2) ON CONFLICT (name) DO NOTHING",
                    &[&name, &created_at],
                )
                .map_err(backend)?;
            if inserted == 0 {
                return Err(StoreError::Conflict(format!(
                    "project {} already exists",
                    name
                )));
            }
            if let Some(audit) = audit {
                insert_audit(tx, audit)?;
            }
            Ok(())
        })?;
        Ok(Project {
            name: name.to_string(),
            created_at,
        })
    }

    fn project(&self, name: &str) -> Result<Option<Project>, StoreError> {
        self.with_client(|client| {
            client
                .query_opt(
                    "SELECT name, created_at FROM projects WHERE name = $1",
                    &[&name],
                )
                .map_err(backend)
                .map(|row| {
                    row.map(|row| Project {
                        name: row.get(0),
                        created_at: row.get(1),
                    })
                })
        })
    }

    fn list_projects(&self) -> Result<Vec<Project>, StoreError> {
        self.with_client(|client| {
            let rows = client
                .query("SELECT name, created_at FROM projects ORDER BY name", &[])
                .map_err(backend)?;
            rows.into_iter()
                .map(|row| {
                    Ok(Project {
                        name: row.get(0),
                        created_at: row.get(1),
                    })
                })
                .collect()
        })
    }

    fn put_blob(&self, bytes: &[u8]) -> Result<String, StoreError> {
        let hash = super::blob_hash(bytes);
        self.with_client(|client| {
            client
                .execute(
                    "INSERT INTO blobs (hash, bytes) VALUES ($1, $2) ON CONFLICT (hash) DO NOTHING",
                    &[&hash, &bytes],
                )
                .map_err(backend)?;
            Ok(())
        })?;
        Ok(hash)
    }

    fn blob(&self, hash: &str) -> Result<Option<Vec<u8>>, StoreError> {
        self.with_client(|client| {
            match client
                .query_opt("SELECT bytes FROM blobs WHERE hash = $1", &[&hash])
                .map_err(backend)?
            {
                Some(row) => Ok(Some(row.get(0))),
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
    ) -> Result<Commit, StoreError> {
        self.with_tx(|tx| {
            if let Some(guard) = guard.as_ref() {
                enforce_guard(tx, project, guard)?;
            }

            tx.execute(
                "INSERT INTO branches (project, name, tip) VALUES ($1, $2, $3) ON CONFLICT (project, name) DO NOTHING",
                &[&project, &branch, &""],
            )
            .map_err(backend)?;

            let tip: Option<String> = tx
                .query_opt(
                    "SELECT tip FROM branches WHERE project = $1 AND name = $2 FOR UPDATE",
                    &[&project, &branch],
                )
                .map_err(backend)?
                .map(|row| row.get::<_, String>(0))
                .filter(|tip| !tip.is_empty());

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
            let parents_json = serde_json::to_string(&parents).map_err(backend)?;

            tx.execute(
                "INSERT INTO commits (hash, project, branch, parents, okf_hash, author, message, created_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
                &[&hash, &project, &branch, &parents_json, &okf_hash, &author, &message, &created_at],
            )
            .map_err(backend)?;
            tx.execute(
                "INSERT INTO branches (project, name, tip) VALUES ($1, $2, $3) ON CONFLICT (project, name) DO UPDATE SET tip = EXCLUDED.tip",
                &[&project, &branch, &hash],
            )
            .map_err(backend)?;
            if let Some(audit) = audit {
                insert_audit(tx, audit)?;
            }
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
        if parents.len() != 2 {
            return Err(StoreError::Backend(format!(
                "a merge commit needs exactly two parents, got {}",
                parents.len()
            )));
        }
        self.with_tx(|tx| {
            let current_tip: Option<String> = tx
                .query_opt(
                    "SELECT tip FROM branches WHERE project = $1 AND name = $2 FOR UPDATE",
                    &[&project, &branch],
                )
                .map_err(backend)?
                .map(|row| row.get(0));
            if current_tip.as_deref() != Some(parents[0].as_str()) {
                return Err(StoreError::Conflict(format!(
                    "branch {} moved while the merge was being prepared (expected {}, found {})",
                    branch,
                    parents[0],
                    current_tip.as_deref().unwrap_or("nothing")
                )));
            }

            if let Some(guard) = guard.as_ref() {
                enforce_guard(tx, project, guard)?;
            }

            for parent in parents {
                let exists: bool = tx
                    .query_opt(
                        "SELECT 1 FROM commits WHERE project = $1 AND hash = $2",
                        &[&project, &parent],
                    )
                    .map_err(backend)?
                    .is_some();
                if !exists {
                    return Err(StoreError::NotFound(format!("parent commit {}", parent)));
                }
            }

            let parents: Vec<String> = parents.to_vec();
            let created_at = now_epoch();
            let hash = super::commit_hash(project, branch, &parents, okf_hash, author, message);
            let parents_json = serde_json::to_string(&parents).map_err(backend)?;

            tx.execute(
                "INSERT INTO commits (hash, project, branch, parents, okf_hash, author, message, created_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
                &[&hash, &project, &branch, &parents_json, &okf_hash, &author, &message, &created_at],
            )
            .map_err(backend)?;
            tx.execute(
                "INSERT INTO branches (project, name, tip) VALUES ($1, $2, $3) ON CONFLICT (project, name) DO UPDATE SET tip = EXCLUDED.tip",
                &[&project, &branch, &hash],
            )
            .map_err(backend)?;
            if let Some(audit) = audit {
                insert_audit(tx, audit)?;
            }
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
        })
    }

    fn commit(&self, project: &str, hash: &str) -> Result<Option<Commit>, StoreError> {
        let row: Option<CommitRow> = self.with_client(|client| {
            client
                .query_opt(
                    "SELECT hash, project, branch, parents, okf_hash, author, message, created_at FROM commits WHERE project = $1 AND hash = $2",
                    &[&project, &hash],
                )
                .map_err(backend)
                .map(|row| {
                    row.map(|row| {
                        (
                            row.get(0),
                            row.get(1),
                            row.get(2),
                            row.get(3),
                            row.get(4),
                            row.get(5),
                            row.get(6),
                            row.get(7),
                        )
                    })
                })
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
        let rows: Vec<CommitRow> = self.with_client(|client| {
            let rows = client
                .query(
                    "SELECT hash, project, branch, parents, okf_hash, author, message, created_at FROM commits WHERE project = $1 AND branch = $2 ORDER BY id",
                    &[&project, &branch],
                )
                .map_err(backend)?;
            rows.into_iter()
                .map(|row| {
                    Ok((
                        row.get(0),
                        row.get(1),
                        row.get(2),
                        row.get(3),
                        row.get(4),
                        row.get(5),
                        row.get(6),
                        row.get(7),
                    ))
                })
                .collect()
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
        self.with_client(|client| {
            client
                .query_opt(
                    "SELECT tip FROM branches WHERE project = $1 AND name = $2",
                    &[&project, &branch],
                )
                .map_err(backend)
                .map(|row| row.map(|row| row.get(0)))
        })
    }

    fn create_branch(
        &self,
        project: &str,
        name: &str,
        from: &str,
        audit: Option<&AuditEntry>,
    ) -> Result<(), StoreError> {
        self.with_tx(|tx| {
            // The source commit must exist, checked inside the transaction that writes the
            // branch, so the check and the write see the same state.
            let from_exists: bool = tx
                .query_opt(
                    "SELECT 1 FROM commits WHERE project = $1 AND hash = $2",
                    &[&project, &from],
                )
                .map_err(backend)?
                .is_some();
            if !from_exists {
                return Err(StoreError::NotFound(format!("commit {}", from)));
            }

            let inserted = tx
                .execute(
                    "INSERT INTO branches (project, name, tip) VALUES ($1, $2, $3) ON CONFLICT (project, name) DO NOTHING",
                    &[&project, &name, &from],
                )
                .map_err(backend)?;
            if inserted == 0 {
                return Err(StoreError::Conflict(format!(
                    "branch {} already exists",
                    name
                )));
            }
            if let Some(audit) = audit {
                insert_audit(tx, audit)?;
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
        self.with_tx(|tx| {
            // Ask which thing is missing inside the transaction that deletes, so the check
            // and the delete see the same state.
            let project_exists: bool = tx
                .query_opt("SELECT 1 FROM projects WHERE name = $1", &[&project])
                .map_err(backend)?
                .is_some();
            if !project_exists {
                return Err(StoreError::NotFound(format!("project {}", project)));
            }

            let removed = tx
                .execute(
                    "DELETE FROM branches WHERE project = $1 AND name = $2",
                    &[&project, &name],
                )
                .map_err(backend)?;
            if removed == 0 {
                return Err(StoreError::NotFound(format!("branch {}", name)));
            }
            if let Some(audit) = audit {
                insert_audit(tx, audit)?;
            }
            Ok(())
        })
    }

    fn list_branches(&self, project: &str) -> Result<Vec<(String, String)>, StoreError> {
        self.with_client(|client| {
            let rows = client
                .query(
                    "SELECT name, tip FROM branches WHERE project = $1 ORDER BY name",
                    &[&project],
                )
                .map_err(backend)?;
            rows.into_iter()
                .map(|row| Ok((row.get(0), row.get(1))))
                .collect()
        })
    }

    fn record_gate_run(&self, run: &GateRun, audit: Option<&AuditEntry>) -> Result<(), StoreError> {
        self.with_tx(|tx| {
            tx.execute(
                "INSERT INTO gate_runs (project, branch, reference_hash, candidate_hash, passed, evidence, created_at) VALUES ($1, $2, $3, $4, $5, $6, $7)",
                &[
                    &run.project,
                    &run.branch,
                    &run.reference_hash,
                    &run.candidate_hash,
                    &run.passed,
                    &run.evidence,
                    &run.created_at,
                ],
            )
            .map_err(backend)?;
            if let Some(audit) = audit {
                insert_audit(tx, audit)?;
            }
            Ok(())
        })
    }

    fn gate_runs(&self, project: &str) -> Result<Vec<GateRun>, StoreError> {
        self.with_client(|client| {
            let rows = client
                .query(
                    "SELECT project, branch, reference_hash, candidate_hash, passed, evidence, created_at FROM gate_runs WHERE project = $1 ORDER BY id",
                    &[&project],
                )
                .map_err(backend)?;
            rows.into_iter()
                .map(|row| {
                    Ok(GateRun {
                        project: row.get(0),
                        branch: row.get(1),
                        reference_hash: row.get(2),
                        candidate_hash: row.get(3),
                        passed: row.get(4),
                        evidence: row.get(5),
                        created_at: row.get(6),
                    })
                })
                .collect()
        })
    }

    fn append_audit(&self, entry: &AuditEntry) -> Result<i64, StoreError> {
        self.with_client(|client| insert_audit(client, entry))
    }

    fn audit(&self, project: &str, limit: i64) -> Result<Vec<AuditEntry>, StoreError> {
        let limit = limit.clamp(1, 1000);
        self.with_client(|client| {
            let rows = client
                .query(
                    "SELECT id, project, at, actor, mechanism, action, subject, detail FROM audit WHERE project = $1 ORDER BY id DESC LIMIT $2",
                    &[&project, &limit],
                )
                .map_err(backend)?;
            rows.into_iter()
                .map(|row| {
                    Ok(AuditEntry {
                        id: row.get(0),
                        project: row.get(1),
                        at: row.get(2),
                        actor: row.get(3),
                        mechanism: row.get(4),
                        action: row.get(5),
                        subject: row.get(6),
                        detail: row.get(7),
                    })
                })
                .collect()
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
        self.with_tx(|tx| {
            // Sweep dead leases first, so an expired lock never blocks a new holder.
            tx.execute("DELETE FROM locks WHERE expires_at <= $1", &[&now])
                .map_err(backend)?;

            let expires_at = now + ttl_seconds;
            let mut locks = Vec::with_capacity(elements.len());
            for element in elements {
                // One statement, settled by the database: insert a fresh lease, or extend
                // this holder's existing lease. The UNIQUE(project, element) constraint is
                // the arbiter between two concurrent acquires by different holders - the
                // loser's DO UPDATE ... WHERE fails its holder test and returns no row, so
                // it is refused with the winner's identity instead of both holding it.
                let id = super::lock_id(project, branch, element, holder, now);
                let taken: Option<(String, i64)> = tx
                    .query_opt(
                        "INSERT INTO locks (id, project, branch, element, holder, acquired_at, expires_at) VALUES ($1, $2, $3, $4, $5, $6, $7) ON CONFLICT (project, element) DO UPDATE SET branch = EXCLUDED.branch, expires_at = EXCLUDED.expires_at WHERE locks.holder = EXCLUDED.holder RETURNING id, acquired_at",
                        &[&id, &project, &branch, &element, &holder, &now, &expires_at],
                    )
                    .map_err(backend)?
                    .map(|row| (row.get(0), row.get(1)));

                match taken {
                    Some((id, acquired_at)) => {
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
                    None => {
                        // A different holder owns a live lease on this element. Returning
                        // the error rolls back the whole transaction, so acquiring is all
                        // or nothing for the requested elements.
                        // Naming the winner is a courtesy, not part of the guarantee: the
                        // constraint already decided that this caller loses. The lease can
                        // even have vanished between the failed insert and this read, if a
                        // sweep expired it in that instant, so the lookup is optional and a
                        // missing row degrades to a retryable refusal rather than a backend
                        // error - an error a caller cannot act on, for a situation that is
                        // simply "somebody else has it, or just had it".
                        let winner: Option<(String, i64)> = tx
                            .query_opt(
                                "SELECT holder, expires_at FROM locks WHERE project = $1 AND element = $2",
                                &[&project, &element],
                            )
                            .map_err(backend)?
                            .map(|row| (row.get(0), row.get(1)));
                        return Err(match winner {
                            Some((holder, expires_at)) => {
                                super::lock_refusal(element, &holder, expires_at)
                            }
                            None => super::lock_refusal(
                                element,
                                "another holder",
                                now,
                            ),
                        });
                    }
                }
            }

            if let Some(audit) = audit {
                insert_audit(tx, audit)?;
            }
            Ok(locks)
        })
    }

    fn release_locks(
        &self,
        project: &str,
        holder: &str,
        ids: &[String],
        audit: Option<&AuditEntry>,
    ) -> Result<usize, StoreError> {
        self.with_tx(|tx| {
            let mut released = 0usize;
            for id in ids {
                released += tx
                    .execute(
                        "DELETE FROM locks WHERE project = $1 AND holder = $2 AND id = $3",
                        &[&project, &holder, &id],
                    )
                    .map_err(backend)? as usize;
            }

            if released > 0 {
                if let Some(audit) = audit {
                    insert_audit(tx, audit)?;
                }
            }
            Ok(released)
        })
    }

    fn locks(&self, project: &str, now: i64) -> Result<Vec<Lock>, StoreError> {
        self.with_client(|client| {
            let rows = client
                .query(
                    "SELECT id, project, branch, element, holder, acquired_at, expires_at FROM locks WHERE project = $1 AND expires_at > $2 ORDER BY element",
                    &[&project, &now],
                )
                .map_err(backend)?;
            rows.into_iter()
                .map(|row| {
                    Ok(Lock {
                        id: row.get(0),
                        project: row.get(1),
                        branch: row.get(2),
                        element: row.get(3),
                        holder: row.get(4),
                        acquired_at: row.get(5),
                        expires_at: row.get(6),
                    })
                })
                .collect()
        })
    }

    fn holders_of(
        &self,
        project: &str,
        elements: &[String],
        now: i64,
    ) -> Result<Vec<Lock>, StoreError> {
        self.with_client(|client| {
            let mut locks = Vec::new();
            for element in elements {
                let rows = client
                    .query(
                        "SELECT id, project, branch, element, holder, acquired_at, expires_at FROM locks WHERE project = $1 AND element = $2 AND expires_at > $3",
                        &[&project, &element, &now],
                    )
                    .map_err(backend)?;
                for row in rows {
                    locks.push(Lock {
                        id: row.get(0),
                        project: row.get(1),
                        branch: row.get(2),
                        element: row.get(3),
                        holder: row.get(4),
                        acquired_at: row.get(5),
                        expires_at: row.get(6),
                    });
                }
            }
            Ok(locks)
        })
    }
}
