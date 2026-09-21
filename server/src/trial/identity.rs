// SPDX-License-Identifier: AGPL-3.0-or-later
//! The identity store: accounts, single-use login codes (hashed at rest), sessions (hashed at
//! rest) and per-trial metadata. One SQLite database holds every account, code, session and
//! trial row; each trial's models live in a separate per-trial database opened by the store's
//! TrialRegistry. No password ever exists: the code is the credential, and both code and
//! session token are stored only as their SHA-256 hex digests.

use std::path::Path;
use std::sync::Mutex;

use rusqlite::{params, Connection, OptionalExtension};

use crate::trial::TrialError;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS accounts (
    email TEXT PRIMARY KEY,
    trial_id TEXT NOT NULL,
    marketing_consent INTEGER NOT NULL DEFAULT 0,
    consent_changed_at INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    created_ip TEXT NOT NULL DEFAULT ''
);
CREATE TABLE IF NOT EXISTS login_codes (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    email TEXT NOT NULL,
    code_hash TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL,
    used_at INTEGER,
    ip TEXT NOT NULL DEFAULT ''
);
CREATE INDEX IF NOT EXISTS login_codes_by_email ON login_codes(email);
CREATE TABLE IF NOT EXISTS sessions (
    token_hash TEXT PRIMARY KEY,
    email TEXT NOT NULL,
    trial_id TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS sessions_by_email ON sessions(email);
CREATE TABLE IF NOT EXISTS trials (
    trial_id TEXT PRIMARY KEY,
    email TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    last_activity_at INTEGER NOT NULL,
    archived_at INTEGER,
    archive_path TEXT,
    checksum TEXT
);
";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    pub email: String,
    pub trial_id: String,
    pub marketing_consent: bool,
    pub consent_changed_at: i64,
    pub created_at: i64,
    pub created_ip: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrialRow {
    pub trial_id: String,
    pub email: String,
    pub created_at: i64,
    pub last_activity_at: i64,
    pub archived_at: Option<i64>,
    pub archive_path: Option<String>,
    pub checksum: Option<String>,
}

impl TrialRow {
    pub fn is_archived(&self) -> bool {
        self.archived_at.is_some()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRow {
    pub email: String,
    pub trial_id: String,
    pub created_at: i64,
    pub expires_at: i64,
}

pub struct IdentityStore {
    connection: Mutex<Connection>,
}

impl IdentityStore {
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        let connection = Connection::open(path)?;
        connection.execute_batch(SCHEMA)?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    fn with<T>(&self, f: impl FnOnce(&Connection) -> rusqlite::Result<T>) -> Result<T, TrialError> {
        let guard = self
            .connection
            .lock()
            .map_err(|_| TrialError::Backend("identity connection lock poisoned".to_string()))?;
        f(&guard).map_err(|e| TrialError::Backend(e.to_string()))
    }

    pub fn account(&self, email: &str) -> Result<Option<Account>, TrialError> {
        self.with(|c| {
            c.query_row(
                "SELECT email, trial_id, marketing_consent, consent_changed_at, created_at, created_ip
                 FROM accounts WHERE email = ?1",
                params![email],
                |row| Ok(Account {
                    email: row.get(0)?,
                    trial_id: row.get(1)?,
                    marketing_consent: row.get::<_, i64>(2)? != 0,
                    consent_changed_at: row.get(3)?,
                    created_at: row.get(4)?,
                    created_ip: row.get(5)?,
                }),
            ).optional()
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create_account(
        &self,
        email: &str,
        trial_id: &str,
        marketing_consent: bool,
        consent_changed_at: i64,
        now: i64,
        ip: &str,
    ) -> Result<(), TrialError> {
        self.with(|c| {
            c.execute(
                "INSERT INTO accounts (email, trial_id, marketing_consent, consent_changed_at, created_at, created_ip)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![email, trial_id, marketing_consent as i64, consent_changed_at, now, ip],
            ).map(|_| ())
        })
    }

    pub fn set_consent(&self, email: &str, consented: bool, now: i64) -> Result<(), TrialError> {
        self.with(|c| {
            c.execute(
                "UPDATE accounts SET marketing_consent = ?1, consent_changed_at = ?2 WHERE email = ?3",
                params![consented as i64, now, email],
            ).map(|_| ())
        })
    }

    pub fn create_trial(&self, trial_id: &str, email: &str, now: i64) -> Result<(), TrialError> {
        self.with(|c| {
            c.execute(
                "INSERT INTO trials (trial_id, email, created_at, last_activity_at) VALUES (?1, ?2, ?3, ?3)",
                params![trial_id, email, now],
            ).map(|_| ())
        })
    }

    pub fn trial(&self, trial_id: &str) -> Result<Option<TrialRow>, TrialError> {
        self.with(|c| {
            c.query_row(
                "SELECT trial_id, email, created_at, last_activity_at, archived_at, archive_path, checksum
                 FROM trials WHERE trial_id = ?1",
                params![trial_id],
                |row| Ok(TrialRow {
                    trial_id: row.get(0)?, email: row.get(1)?, created_at: row.get(2)?,
                    last_activity_at: row.get(3)?, archived_at: row.get(4)?,
                    archive_path: row.get(5)?, checksum: row.get(6)?,
                }),
            ).optional()
        })
    }

    pub fn list_trials(&self) -> Result<Vec<TrialRow>, TrialError> {
        self.with(|c| {
            let mut stmt = c.prepare(
                "SELECT trial_id, email, created_at, last_activity_at, archived_at, archive_path, checksum
                 FROM trials ORDER BY created_at",
            )?;
            let rows = stmt.query_map([], |row| Ok(TrialRow {
                trial_id: row.get(0)?, email: row.get(1)?, created_at: row.get(2)?,
                last_activity_at: row.get(3)?, archived_at: row.get(4)?,
                archive_path: row.get(5)?, checksum: row.get(6)?,
            }))?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
        })
    }

    pub fn list_live_trials(&self) -> Result<Vec<TrialRow>, TrialError> {
        Ok(self
            .list_trials()?
            .into_iter()
            .filter(|t| !t.is_archived())
            .collect())
    }

    pub fn live_trial_count(&self) -> Result<usize, TrialError> {
        self.with(|c| {
            c.query_row(
                "SELECT COUNT(*) FROM trials WHERE archived_at IS NULL",
                [],
                |row| row.get::<_, i64>(0),
            )
            .map(|n| n as usize)
        })
    }

    pub fn record_activity(&self, trial_id: &str, now: i64) -> Result<(), TrialError> {
        self.with(|c| {
            c.execute(
                "UPDATE trials SET last_activity_at = ?1 WHERE trial_id = ?2",
                params![now, trial_id],
            )
            .map(|_| ())
        })
    }

    pub fn archive_trial(
        &self,
        trial_id: &str,
        archived_at: i64,
        archive_path: &str,
        checksum: &str,
    ) -> Result<(), TrialError> {
        self.with(|c| {
            c.execute(
                "UPDATE trials SET archived_at = ?1, archive_path = ?2, checksum = ?3 WHERE trial_id = ?4",
                params![archived_at, archive_path, checksum, trial_id],
            ).map(|_| ())
        })
    }

    pub fn issue_code(
        &self,
        email: &str,
        code_hash: &str,
        ip: &str,
        now: i64,
        expires_at: i64,
    ) -> Result<(), TrialError> {
        self.with(|c| {
            c.execute(
                "INSERT INTO login_codes (email, code_hash, created_at, expires_at, ip) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![email, code_hash, now, expires_at, ip],
            ).map(|_| ())
        })
    }

    pub fn code_count_since(&self, email: &str, since: i64) -> Result<usize, TrialError> {
        self.with(|c| {
            c.query_row(
                "SELECT COUNT(*) FROM login_codes WHERE email = ?1 AND created_at >= ?2",
                params![email, since],
                |row| row.get::<_, i64>(0),
            )
            .map(|n| n as usize)
        })
    }

    pub fn accounts_since(&self, ip: &str, since: i64) -> Result<usize, TrialError> {
        self.with(|c| {
            c.query_row(
                "SELECT COUNT(*) FROM accounts WHERE created_ip = ?1 AND created_at >= ?2",
                params![ip, since],
                |row| row.get::<_, i64>(0),
            )
            .map(|n| n as usize)
        })
    }

    pub fn redeem_code(&self, email: &str, code_hash: &str, now: i64) -> Result<(), TrialError> {
        let row = self.with(|c| {
            c.query_row(
                "SELECT id, expires_at, used_at FROM login_codes WHERE email = ?1 AND code_hash = ?2
                 ORDER BY id DESC LIMIT 1",
                params![email, code_hash],
                |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, Option<i64>>(2)?)),
            ).optional()
        })?;
        let Some((id, expires_at, used_at)) = row else {
            return Err(TrialError::InvalidCode);
        };
        if used_at.is_some() {
            return Err(TrialError::CodeAlreadyUsed);
        }
        if expires_at <= now {
            return Err(TrialError::ExpiredCode);
        }
        let marked = self.with(|c| {
            c.execute(
                "UPDATE login_codes SET used_at = ?1 WHERE id = ?2 AND used_at IS NULL",
                params![now, id],
            )
        })?;
        if marked == 0 {
            return Err(TrialError::CodeAlreadyUsed);
        }
        Ok(())
    }

    pub fn create_session(
        &self,
        token_hash: &str,
        email: &str,
        trial_id: &str,
        now: i64,
        expires_at: i64,
    ) -> Result<(), TrialError> {
        self.with(|c| {
            c.execute(
                "INSERT INTO sessions (token_hash, email, trial_id, created_at, expires_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![token_hash, email, trial_id, now, expires_at],
            ).map(|_| ())
        })
    }

    pub fn session(&self, token_hash: &str) -> Result<Option<SessionRow>, TrialError> {
        self.with(|c| {
            c.query_row(
                "SELECT email, trial_id, created_at, expires_at FROM sessions WHERE token_hash = ?1",
                params![token_hash],
                |row| Ok(SessionRow {
                    email: row.get(0)?, trial_id: row.get(1)?, created_at: row.get(2)?,
                    expires_at: row.get(3)?,
                }),
            ).optional()
        })
    }

    pub fn delete_session(&self, token_hash: &str) -> Result<(), TrialError> {
        self.with(|c| {
            c.execute(
                "DELETE FROM sessions WHERE token_hash = ?1",
                params![token_hash],
            )
            .map(|_| ())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::hash_token;

    fn store(dir: &Path) -> IdentityStore {
        IdentityStore::open(&dir.join("identity.db")).unwrap()
    }

    #[test]
    fn a_code_is_hashed_at_rest_and_single_use() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(dir.path());
        let now = 1_700_000_000;
        store
            .issue_code(
                "a@example.com",
                &hash_token("123456"),
                "1.2.3.4",
                now,
                now + 600,
            )
            .unwrap();
        store
            .redeem_code("a@example.com", &hash_token("123456"), now + 5)
            .unwrap();
        assert!(matches!(
            store.redeem_code("a@example.com", &hash_token("123456"), now + 6),
            Err(TrialError::CodeAlreadyUsed)
        ));
        assert!(matches!(
            store.redeem_code("a@example.com", &hash_token("000000"), now + 7),
            Err(TrialError::InvalidCode)
        ));
    }

    #[test]
    fn a_code_expires_after_ten_minutes() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(dir.path());
        let now = 1_700_000_000;
        store
            .issue_code(
                "a@example.com",
                &hash_token("123456"),
                "1.2.3.4",
                now,
                now + 600,
            )
            .unwrap();
        assert!(matches!(
            store.redeem_code("a@example.com", &hash_token("123456"), now + 601),
            Err(TrialError::ExpiredCode)
        ));
    }

    #[test]
    fn consent_is_recorded_with_a_timestamp_and_changeable() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(dir.path());
        let now = 1_700_000_000;
        store
            .create_account("a@example.com", "trial-1", true, now, now, "1.2.3.4")
            .unwrap();
        let account = store.account("a@example.com").unwrap().unwrap();
        assert!(account.marketing_consent);
        assert_eq!(account.consent_changed_at, now);
        store
            .set_consent("a@example.com", false, now + 100)
            .unwrap();
        let account = store.account("a@example.com").unwrap().unwrap();
        assert!(!account.marketing_consent);
        assert_eq!(account.consent_changed_at, now + 100);
    }

    #[test]
    fn activity_refreshes_the_window_and_live_count_excludes_archived() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(dir.path());
        let now = 1_700_000_000;
        store
            .create_account("a@example.com", "trial-1", false, 0, now, "1.2.3.4")
            .unwrap();
        store.create_trial("trial-1", "a@example.com", now).unwrap();
        assert_eq!(store.live_trial_count().unwrap(), 1);
        store.record_activity("trial-1", now + 1_000_000).unwrap();
        let trial = store.trial("trial-1").unwrap().unwrap();
        assert_eq!(trial.last_activity_at, now + 1_000_000);
        store
            .archive_trial("trial-1", now + 2_000_000, "archive/trial-1.db", "deadbeef")
            .unwrap();
        assert_eq!(store.live_trial_count().unwrap(), 0);
        let trial = store.trial("trial-1").unwrap().unwrap();
        assert!(trial.is_archived());
        assert_eq!(trial.checksum.as_deref(), Some("deadbeef"));
    }

    #[test]
    fn rate_limit_counts_are_scoped_to_email_and_ip() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(dir.path());
        let now = 1_700_000_000;
        store
            .issue_code("a@example.com", &hash_token("1"), "1.1.1.1", now, now + 600)
            .unwrap();
        store
            .issue_code("b@example.com", &hash_token("2"), "1.1.1.1", now, now + 600)
            .unwrap();
        assert_eq!(
            store.code_count_since("a@example.com", now - 3600).unwrap(),
            1
        );
        assert_eq!(
            store.code_count_since("b@example.com", now - 3600).unwrap(),
            1
        );
        store
            .create_account("a@example.com", "t1", false, 0, now, "9.9.9.9")
            .unwrap();
        store
            .create_account("b@example.com", "t2", false, 0, now, "9.9.9.9")
            .unwrap();
        assert_eq!(store.accounts_since("9.9.9.9", now - 86400).unwrap(), 2);
        assert_eq!(store.accounts_since("8.8.8.8", now - 86400).unwrap(), 0);
    }
}
