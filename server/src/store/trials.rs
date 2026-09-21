// SPDX-License-Identifier: AGPL-3.0-or-later
//! Per-trial database resolution. Each registered trial gets ONE SQLite database for its
//! models, opened lazily on first use and cached. The trial identity arrives from the
//! SESSION, never from the request: a caller names a trial id only by holding a session that
//! was issued to that trial.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use super::{sqlite, Store, StoreError};

/// Resolve a trial id to the store that holds that trial's models. This is the seam where the
/// registered tier turns a session (already verified to belong to a trial) into the trial's
/// own database.
pub trait TrialStoreResolver: Send + Sync {
    fn resolve(&self, trial_id: &str) -> Result<Arc<dyn Store>, StoreError>;
}

/// Opens and caches one SQLite database per trial under data_root/trials/<id>.db.
pub struct TrialRegistry {
    data_root: PathBuf,
    cache: Mutex<HashMap<String, Arc<dyn Store>>>,
}

impl TrialRegistry {
    pub fn new(data_root: PathBuf) -> Self {
        Self {
            data_root,
            cache: Mutex::new(HashMap::new()),
        }
    }

    /// The filesystem path of a trial's database. The trial id is a server-generated random
    /// token, so it is path-safe by construction.
    pub fn trial_db_path(&self, trial_id: &str) -> PathBuf {
        self.data_root
            .join("trials")
            .join(format!("{}.db", trial_id))
    }

    /// Resolve (opening and caching if needed) the store for a trial. Opening also CREATES the
    /// database and its schema, so this doubles as the per-trial provisioning step at
    /// registration.
    pub fn store_for(&self, trial_id: &str) -> Result<Arc<dyn Store>, StoreError> {
        if let Some(cached) = self
            .cache
            .lock()
            .ok()
            .and_then(|m| m.get(trial_id).cloned())
        {
            return Ok(cached);
        }
        let path = self.trial_db_path(trial_id);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| StoreError::Backend(format!("cannot create trials dir: {}", e)))?;
        }
        let store =
            sqlite::SqliteStore::open(&path).map_err(|e| StoreError::Backend(e.to_string()))?;
        let arc = Arc::new(store) as Arc<dyn Store>;
        if let Ok(mut cache) = self.cache.lock() {
            cache.insert(trial_id.to_string(), arc.clone());
        }
        Ok(arc)
    }

    /// Drop the cached handle for a trial so its database file can be archived and removed.
    /// A later resolve for the same id would recreate an empty database, which is why the
    /// reaper evicts only AFTER the trial is marked archived and sessions for it are refused.
    pub fn evict(&self, trial_id: &str) {
        if let Ok(mut cache) = self.cache.lock() {
            cache.remove(trial_id);
        }
    }
}

impl TrialStoreResolver for TrialRegistry {
    fn resolve(&self, trial_id: &str) -> Result<Arc<dyn Store>, StoreError> {
        self.store_for(trial_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_trial_gets_its_own_database() {
        let dir = tempfile::tempdir().unwrap();
        let registry = TrialRegistry::new(dir.path().to_path_buf());
        let a = registry.store_for("trial-a").unwrap();
        let b = registry.store_for("trial-b").unwrap();
        a.create_project("only-in-a", None).unwrap();
        assert!(a.project("only-in-a").unwrap().is_some());
        assert!(b.project("only-in-a").unwrap().is_none());
        assert!(dir.path().join("trials").join("trial-a.db").exists());
        assert!(dir.path().join("trials").join("trial-b.db").exists());
    }

    #[test]
    fn resolution_is_cached_and_eviction_drops_the_handle() {
        let dir = tempfile::tempdir().unwrap();
        let registry = TrialRegistry::new(dir.path().to_path_buf());
        let first = registry.store_for("trial-a").unwrap();
        let second = registry.store_for("trial-a").unwrap();
        assert!(
            Arc::ptr_eq(&first, &second),
            "the same trial resolves to the same cached store"
        );
        registry.evict("trial-a");
        let third = registry.store_for("trial-a").unwrap();
        assert!(!Arc::ptr_eq(&first, &third), "eviction reopens the store");
    }
}
