// SPDX-License-Identifier: AGPL-3.0-or-later
//! The registered-trial tier: identity, sessions, the rolling lifecycle, rate limits, consent
//! and the pluggable mailer, orchestrated by [TrialService].

pub mod identity;
pub mod lifecycle;
pub mod mailer;

pub use identity::{Account, IdentityStore, SessionRow, TrialRow};
pub use lifecycle::{phase, Phase};
pub use mailer::{
    login_code_email, mailer_from_env, sender_from_env, ConsoleTransport, FileTransport, Mailer,
    OutboundEmail,
};

use std::path::PathBuf;
use std::sync::Arc;

use rand::Rng;
use rand::RngCore;

use crate::auth::{hash_token, SessionIdentity, SessionResolver};
use crate::store::{self, Store, StoreError, TrialRegistry};

/// A bearer session token, as issued to a client. The plaintext is returned ONCE at
/// redemption; only its hash is stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    pub token: String,
    pub email: String,
    pub trial_id: String,
    pub expires_at: i64,
}

/// The tunables for the registered tier. Every value has a default and an environment
/// override, so the cap and the rate limits are configuration, not code.
#[derive(Debug, Clone)]
pub struct TrialConfig {
    pub code_ttl_seconds: i64,
    pub session_ttl_seconds: i64,
    pub codes_per_email_per_hour: usize,
    pub registrations_per_ip_per_day: usize,
    pub live_trial_cap: usize,
}

impl Default for TrialConfig {
    fn default() -> Self {
        Self {
            code_ttl_seconds: 600,
            session_ttl_seconds: 30 * 86_400,
            codes_per_email_per_hour: 5,
            registrations_per_ip_per_day: 10,
            live_trial_cap: 1000,
        }
    }
}

impl TrialConfig {
    pub fn from_env() -> Self {
        let mut config = Self::default();
        if let Some(v) = env_int("MW_TRIAL_CODE_TTL_SECONDS") {
            config.code_ttl_seconds = v;
        }
        if let Some(v) = env_int("MW_TRIAL_SESSION_TTL_SECONDS") {
            config.session_ttl_seconds = v;
        }
        if let Some(v) = env_int("MW_TRIAL_CODES_PER_EMAIL_PER_HOUR") {
            config.codes_per_email_per_hour = v as usize;
        }
        if let Some(v) = env_int("MW_TRIAL_REGISTRATIONS_PER_IP_PER_DAY") {
            config.registrations_per_ip_per_day = v as usize;
        }
        if let Some(v) = env_int("MW_TRIAL_LIVE_CAP") {
            config.live_trial_cap = v as usize;
        }
        config
    }
}

fn env_int(name: &str) -> Option<i64> {
    std::env::var(name)
        .ok()
        .and_then(|v| v.trim().parse::<i64>().ok())
}

/// Every failure the registered tier can produce. The HTTP layer maps each to a status code;
/// the store never sees these.
#[derive(Debug)]
pub enum TrialError {
    InvalidEmail(String),
    RateLimited(String),
    CapReached,
    InvalidCode,
    ExpiredCode,
    CodeAlreadyUsed,
    NoAccount(String),
    NoTrial(String),
    ReadOnly(String),
    Store(StoreError),
    Mailer(String),
    Backend(String),
}

impl std::fmt::Display for TrialError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TrialError::InvalidEmail(m) => write!(f, "invalid email: {}", m),
            TrialError::RateLimited(m) => write!(f, "rate limited: {}", m),
            TrialError::CapReached => write!(f, "the trial cap has been reached; try again later"),
            TrialError::InvalidCode => write!(f, "invalid login code"),
            TrialError::ExpiredCode => write!(f, "that login code has expired"),
            TrialError::CodeAlreadyUsed => write!(f, "that login code has already been used"),
            TrialError::NoAccount(m) => write!(f, "no account for {}", m),
            TrialError::NoTrial(m) => write!(f, "no live trial {}", m),
            TrialError::ReadOnly(m) => write!(f, "{}", m),
            TrialError::Store(e) => write!(f, "{}", e),
            TrialError::Mailer(m) => write!(f, "mailer: {}", m),
            TrialError::Backend(m) => write!(f, "identity store: {}", m),
        }
    }
}

impl std::error::Error for TrialError {}

impl From<StoreError> for TrialError {
    fn from(e: StoreError) -> Self {
        TrialError::Store(e)
    }
}

/// A six-digit login code, uniform over 0..1_000_000. The code is the credential and is
/// single-use and short-lived; it is hashed before storage.
pub fn new_login_code() -> String {
    let mut rng = rand::rngs::OsRng;
    format!("{:06}", rng.gen_range(0..1_000_000u32))
}

/// A 256-bit session token, hex-encoded. Only its SHA-256 digest is ever stored.
pub fn new_session_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}

/// A 128-bit trial id, hex-encoded. Path-safe by construction, used to name the trial's
/// database file.
pub fn new_trial_id() -> String {
    let mut bytes = [0u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}

/// Lowercase and trim an email, and require a minimal shape (something@something.something).
/// This is the identity key, so it is normalised once and everywhere.
pub fn normalize_email(email: &str) -> Result<String, TrialError> {
    let email = email.trim().to_ascii_lowercase();
    if email.len() > 254 || !email.contains('@') {
        return Err(TrialError::InvalidEmail(email));
    }
    let (local, domain) = email.split_once('@').expect("contains @");
    if local.is_empty() || !domain.contains('.') || domain.starts_with('.') || domain.ends_with('.')
    {
        return Err(TrialError::InvalidEmail(email));
    }
    Ok(email)
}

/// The whole registered tier in one object: the identity store, the per-trial registry, the
/// mailer and the configuration. Held once, shared by every request.
pub struct TrialService {
    pub identity: Arc<IdentityStore>,
    pub registry: Arc<TrialRegistry>,
    pub mailer: Arc<dyn Mailer>,
    pub config: TrialConfig,
    pub sender: String,
    pub archive_root: PathBuf,
}

impl TrialService {
    pub fn new(
        identity: IdentityStore,
        registry: TrialRegistry,
        mailer: Arc<dyn Mailer>,
        config: TrialConfig,
        archive_root: PathBuf,
    ) -> Arc<Self> {
        Arc::new(Self {
            identity: Arc::new(identity),
            registry: Arc::new(registry),
            mailer,
            sender: sender_from_env(),
            config,
            archive_root,
        })
    }

    /// Request a login code for an email. Enforces the per-email hourly limit, the per-IP
    /// daily registration limit and the live-trial cap, provisions a new trial on first
    /// registration, records the marketing consent (which NEVER gates the code), and hands
    /// the code to the mailer. The code is also returned to the caller for the tests that
    /// cannot observe the mailer.
    pub fn request_code(
        &self,
        email: &str,
        ip: &str,
        consent: bool,
        now: i64,
    ) -> Result<String, TrialError> {
        let email = normalize_email(email)?;
        let hour_ago = now - 3600;
        if self.identity.code_count_since(&email, hour_ago)? >= self.config.codes_per_email_per_hour
        {
            return Err(TrialError::RateLimited(
                "too many codes for this email in the last hour".to_string(),
            ));
        }
        match self.identity.account(&email)? {
            None => {
                let day_ago = now - 86400;
                if self.identity.accounts_since(ip, day_ago)?
                    >= self.config.registrations_per_ip_per_day
                {
                    return Err(TrialError::RateLimited(
                        "too many registrations from this address today".to_string(),
                    ));
                }
                if self.identity.live_trial_count()? >= self.config.live_trial_cap {
                    return Err(TrialError::CapReached);
                }
                let trial_id = new_trial_id();
                let consent_at = if consent { now } else { 0 };
                self.identity
                    .create_account(&email, &trial_id, consent, consent_at, now, ip)?;
                self.identity.create_trial(&trial_id, &email, now)?;
                self.registry.store_for(&trial_id)?;
            }
            Some(account) => {
                if consent && !account.marketing_consent {
                    self.identity.set_consent(&email, true, now)?;
                }
            }
        }
        let code = new_login_code();
        let code_hash = hash_token(&code);
        self.identity.issue_code(
            &email,
            &code_hash,
            ip,
            now,
            now + self.config.code_ttl_seconds,
        )?;
        self.mailer
            .send(&login_code_email(&self.sender, &email, &code))?;
        Ok(code)
    }

    /// Redeem a single-use code for a session token. The token is the credential; it is
    /// returned once and stored only as a hash.
    pub fn redeem(&self, email: &str, code: &str, now: i64) -> Result<Session, TrialError> {
        let email = normalize_email(email)?;
        let code_hash = hash_token(code);
        self.identity.redeem_code(&email, &code_hash, now)?;
        let account = self
            .identity
            .account(&email)?
            .ok_or_else(|| TrialError::NoAccount(email.clone()))?;
        let token = new_session_token();
        let token_hash = hash_token(&token);
        let expires_at = now + self.config.session_ttl_seconds;
        self.identity
            .create_session(&token_hash, &email, &account.trial_id, now, expires_at)?;
        Ok(Session {
            token,
            email,
            trial_id: account.trial_id,
            expires_at,
        })
    }

    /// Resolve a presented token against the REAL clock, refusing expired sessions and
    /// sessions whose trial has been archived. Tests time-travel with [Self::resolve_session_at].
    pub fn resolve_session(&self, token: &str) -> Option<Session> {
        self.resolve_session_at(token, store::now_seconds())
    }

    /// Resolve a presented token at an explicit moment, for the time-travelled tests. The
    /// production path ([Self::resolve_session]) uses the real clock.
    pub fn resolve_session_at(&self, token: &str, now: i64) -> Option<Session> {
        let hash = hash_token(token);
        let row = self.identity.session(&hash).ok().flatten()?;
        if row.expires_at <= now {
            return None;
        }
        let trial = self.identity.trial(&row.trial_id).ok().flatten()?;
        if trial.is_archived() {
            return None;
        }
        Some(Session {
            token: token.to_string(),
            email: row.email,
            trial_id: row.trial_id,
            expires_at: row.expires_at,
        })
    }

    /// Remove a session (logout).
    pub fn logout(&self, token: &str) -> Result<(), TrialError> {
        self.identity.delete_session(&hash_token(token))
    }

    /// Refresh the rolling window: any authenticated activity rewinds the clock to now.
    pub fn record_activity(&self, trial_id: &str, now: i64) -> Result<(), TrialError> {
        self.identity.record_activity(trial_id, now)
    }

    /// The write refusal for a trial at this moment, or None if it is writable. A read-only
    /// trial produces the message naming its end and how to restart; an active trial produces
    /// None; a frozen trial has no live database and its session is already refused.
    pub fn write_refusal_for(&self, trial_id: &str, now: i64) -> Option<String> {
        let trial = self.identity.trial(trial_id).ok().flatten()?;
        if trial.is_archived() {
            return Some("this trial has been archived".to_string());
        }
        lifecycle::write_refusal(trial.last_activity_at, now)
    }

    /// Resolve a session's per-trial store and record the activity. This is the per-request
    /// trial resolution: the trial comes from the SESSION, never from the request body.
    pub fn resolve_store(&self, trial_id: &str, now: i64) -> Result<Arc<dyn Store>, TrialError> {
        self.record_activity(trial_id, now)?;
        self.registry.store_for(trial_id).map_err(TrialError::Store)
    }

    /// The nightly reaper: archive every live trial that has been idle for 21 days. Each is
    /// copied to the archive root, checksummed, the live copy removed, and the account kept.
    pub fn reap(&self, now: i64) -> Result<usize, TrialError> {
        std::fs::create_dir_all(&self.archive_root)
            .map_err(|e| TrialError::Backend(format!("cannot create archive root: {}", e)))?;
        let mut archived = 0;
        for trial in self.identity.list_live_trials()? {
            if lifecycle::phase(trial.last_activity_at, now) != Phase::Frozen {
                continue;
            }
            let live_path = self.registry.trial_db_path(&trial.trial_id);
            self.registry.evict(&trial.trial_id);
            let archive_name = format!("{}.db", trial.trial_id);
            let archive_path = self.archive_root.join(&archive_name);
            std::fs::copy(&live_path, &archive_path)
                .map_err(|e| TrialError::Backend(format!("cannot archive trial: {}", e)))?;
            let checksum = store::file_hash(&archive_path)?;
            if live_path.exists() {
                std::fs::remove_file(&live_path).map_err(|e| {
                    TrialError::Backend(format!("cannot remove live trial db: {}", e))
                })?;
            }
            self.identity
                .archive_trial(&trial.trial_id, now, &archive_name, &checksum)?;
            archived += 1;
        }
        Ok(archived)
    }
}

impl SessionResolver for TrialService {
    fn resolve(&self, token: &str) -> Option<SessionIdentity> {
        self.resolve_session(token).map(|s| SessionIdentity {
            email: s.email,
            trial_id: s.trial_id,
            expires_at: s.expires_at,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn service(dir: &std::path::Path, config: TrialConfig) -> Arc<TrialService> {
        let identity = IdentityStore::open(&dir.join("identity.db")).unwrap();
        let registry = TrialRegistry::new(dir.to_path_buf());
        TrialService::new(
            identity,
            registry,
            Arc::new(ConsoleTransport),
            config,
            dir.join("archive"),
        )
    }

    fn tight_config() -> TrialConfig {
        TrialConfig {
            codes_per_email_per_hour: 2,
            registrations_per_ip_per_day: 2,
            live_trial_cap: 2,
            ..TrialConfig::default()
        }
    }

    #[test]
    fn registration_provisions_a_trial_and_redeem_issues_a_session() {
        let dir = tempfile::tempdir().unwrap();
        let service = service(dir.path(), tight_config());
        let now = 1_700_000_000;
        let code = service
            .request_code("A@Example.com", "1.2.3.4", true, now)
            .unwrap();
        assert_eq!(code.len(), 6);
        let session = service.redeem("a@example.com", &code, now + 60).unwrap();
        assert_eq!(session.email, "a@example.com");
        assert!(!session.trial_id.is_empty());
        assert!(service
            .resolve_session_at(&session.token, now + 60)
            .is_some());
        // The trial database was provisioned on disk.
        assert!(dir
            .path()
            .join("trials")
            .join(format!("{}.db", session.trial_id))
            .exists());
    }

    #[test]
    fn the_rate_limit_stops_a_burst() {
        let dir = tempfile::tempdir().unwrap();
        let service = service(dir.path(), tight_config());
        let now = 1_700_000_000;
        service
            .request_code("a@example.com", "1.2.3.4", false, now)
            .unwrap();
        service
            .request_code("a@example.com", "1.2.3.4", false, now + 1)
            .unwrap();
        assert!(matches!(
            service.request_code("a@example.com", "1.2.3.4", false, now + 2),
            Err(TrialError::RateLimited(_))
        ));
    }

    #[test]
    fn the_ip_limit_and_the_cap_are_enforced() {
        let dir = tempfile::tempdir().unwrap();
        let service = service(dir.path(), tight_config());
        let now = 1_700_000_000;
        service
            .request_code("a@example.com", "9.9.9.9", false, now)
            .unwrap();
        service
            .request_code("b@example.com", "9.9.9.9", false, now)
            .unwrap();
        assert!(matches!(
            service.request_code("c@example.com", "9.9.9.9", false, now),
            Err(TrialError::RateLimited(_))
        ));
        assert!(matches!(
            service.request_code("d@example.com", "8.8.8.8", false, now),
            Err(TrialError::CapReached)
        ));
    }

    #[test]
    fn consent_is_recorded_and_does_not_gate_the_code() {
        let dir = tempfile::tempdir().unwrap();
        let service = service(dir.path(), tight_config());
        let now = 1_700_000_000;
        // Unchecked consent: the code STILL arrives.
        let code = service
            .request_code("a@example.com", "1.2.3.4", false, now)
            .unwrap();
        assert_eq!(code.len(), 6);
        let account = service.identity.account("a@example.com").unwrap().unwrap();
        assert!(!account.marketing_consent);
        assert_eq!(
            account.consent_changed_at, 0,
            "no explicit opt-in means no timestamp"
        );

        // Checked consent is recorded with a timestamp.
        let code2 = service
            .request_code("b@example.com", "1.2.3.4", true, now)
            .unwrap();
        assert_eq!(code2.len(), 6);
        let account = service.identity.account("b@example.com").unwrap().unwrap();
        assert!(account.marketing_consent);
        assert_eq!(account.consent_changed_at, now);
    }

    #[test]
    fn the_rolling_window_refuses_writes_after_fourteen_days_and_recovers() {
        let dir = tempfile::tempdir().unwrap();
        let service = service(dir.path(), tight_config());
        let now = 1_700_000_000;
        let code = service
            .request_code("a@example.com", "1.2.3.4", false, now)
            .unwrap();
        let session = service.redeem("a@example.com", &code, now + 1).unwrap();

        // Day 13: still active, writes allowed.
        assert!(service
            .write_refusal_for(&session.trial_id, now + 13 * 86400)
            .is_none());
        // Day 14: read-only, writes refused with a restart message.
        let refusal = service
            .write_refusal_for(&session.trial_id, now + 14 * 86400)
            .unwrap();
        assert!(refusal.contains("read-only"));
        // Activity on day 13 refreshes the window to day 27.
        service
            .record_activity(&session.trial_id, now + 13 * 86400)
            .unwrap();
        assert!(service
            .write_refusal_for(&session.trial_id, now + 27 * 86400 - 1)
            .is_none());
    }

    #[test]
    fn the_reaper_archives_a_trial_after_twenty_one_days_idle() {
        let dir = tempfile::tempdir().unwrap();
        let service = service(dir.path(), tight_config());
        let now = 1_700_000_000;
        let code = service
            .request_code("a@example.com", "1.2.3.4", false, now)
            .unwrap();
        let session = service.redeem("a@example.com", &code, now + 1).unwrap();
        let trial_id = session.trial_id.clone();
        let live = dir.path().join("trials").join(format!("{}.db", trial_id));
        assert!(live.exists());

        let archived = service.reap(now + 21 * 86400).unwrap();
        assert_eq!(archived, 1);
        assert!(!live.exists(), "the live copy is removed");
        assert!(dir
            .path()
            .join("archive")
            .join(format!("{}.db", trial_id))
            .exists());
        let trial = service.identity.trial(&trial_id).unwrap().unwrap();
        assert!(trial.is_archived());
        assert_eq!(trial.checksum.as_deref().map(|c| c.len()), Some(64));
        // The account is kept, and its old session is now dead.
        assert!(service.identity.account("a@example.com").unwrap().is_some());
        assert!(service
            .resolve_session_at(&session.token, now + 1)
            .is_none());
    }
}
