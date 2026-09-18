// SPDX-License-Identifier: AGPL-3.0-or-later
//! Sources: where a piece of analytics data came from, and how much it is
//! trusted. A source carries its trust level with it from the moment it is
//! declared, so a query can never decide for itself that an untrusted source
//! is trustworthy.

use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::dataset::{CsvError, Dataset};

/// What kind of data a source supplies. The kind decides how the data is read,
/// never how much it is trusted: trust is declared separately.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SourceKind {
    /// A model (OKF) produced by this platform: structured, provable, versioned.
    Model,
    /// Tabular records - a spreadsheet, an ERP or PLM export, a CSV. Unversioned
    /// and of unknown provenance.
    Structured,
    /// Unstructured text - specifications, supplier documents, standards, news.
    Text,
}

/// How trustworthy a source's numbers are. This is declared when the source is
/// registered and travels WITH the data; it is never supplied at query time,
/// because a query has no business grading its own inputs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TrustLevel {
    /// Derived from instrumentation or a provable model.
    Measured,
    /// Stated by someone, not independently verified.
    Reported,
    /// A judgement, a projection, a guess dressed as a number.
    Estimated,
}

/// A declared source. The trust level here is the one every snapshot built from
/// this source inherits.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Source {
    pub id: String,
    pub kind: SourceKind,
    pub trust: TrustLevel,
    pub description: String,
}

/// The error the source registry returns when it refuses something.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegistryError {
    /// A source with this id was already declared.
    DuplicateSource(String),
    /// The dataset's source was never declared, so it has no trust level and
    /// cannot be queried.
    UnregisteredSource(String),
    /// The bytes handed in were not a valid dataset for this source.
    Csv(CsvError),
}

impl fmt::Display for RegistryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RegistryError::DuplicateSource(id) => {
                write!(f, "source {} is already registered", id)
            }
            RegistryError::UnregisteredSource(id) => {
                write!(f, "source {} is not registered", id)
            }
            RegistryError::Csv(e) => write!(f, "{}", e),
        }
    }
}

impl std::error::Error for RegistryError {}

/// The source registry: the single place a source is declared with its trust
/// level. A dataset whose source is not declared here is refused, because an
/// undeclared source has no trust level and an untrusted number is worse than
/// no number.
#[derive(Debug, Default)]
pub struct Registry {
    sources: HashMap<String, Source>,
    datasets: HashMap<String, Dataset>,
}

impl Registry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Declare a source and fix its trust level. This is the only place trust is
    /// set; from here on it travels with every snapshot of this source.
    pub fn register(&mut self, source: Source) -> Result<(), RegistryError> {
        match self.sources.entry(source.id.clone()) {
            Entry::Vacant(slot) => {
                slot.insert(source);
                Ok(())
            }
            Entry::Occupied(_) => Err(RegistryError::DuplicateSource(source.id)),
        }
    }

    pub fn source(&self, id: &str) -> Option<&Source> {
        self.sources.get(id)
    }

    pub fn is_registered(&self, id: &str) -> bool {
        self.sources.contains_key(id)
    }

    /// Build a content-addressed snapshot of `bytes` for a declared source. The
    /// trust level is the one the source was registered with: the caller supplies
    /// only the id and the read time, never a trust level. An undeclared source is
    /// refused outright.
    pub fn snapshot(
        &mut self,
        source_id: &str,
        captured_at: String,
        bytes: &[u8],
    ) -> Result<Dataset, RegistryError> {
        let source = self
            .source(source_id)
            .cloned()
            .ok_or_else(|| RegistryError::UnregisteredSource(source_id.to_string()))?;
        let dataset =
            Dataset::from_csv_at(source, captured_at, bytes).map_err(RegistryError::Csv)?;
        let hash = dataset.content_hash.clone();
        self.datasets.insert(hash, dataset.clone());
        Ok(dataset)
    }

    /// Accept an already-built dataset, refusing one whose source was not declared.
    pub fn register_dataset(&mut self, dataset: Dataset) -> Result<(), RegistryError> {
        if !self.sources.contains_key(&dataset.source.id) {
            return Err(RegistryError::UnregisteredSource(dataset.source.id.clone()));
        }
        let hash = dataset.content_hash.clone();
        self.datasets.insert(hash, dataset);
        Ok(())
    }

    /// The snapshots recorded so far, keyed by content hash so re-reading the same
    /// bytes is idempotent rather than a duplicate.
    pub fn datasets(&self) -> impl Iterator<Item = &Dataset> + '_ {
        self.datasets.values()
    }

    /// Fetch a snapshot by its content hash.
    pub fn dataset(&self, content_hash: &str) -> Option<&Dataset> {
        self.datasets.get(content_hash)
    }

    /// Fetch a snapshot's exact bytes by its content hash. This is what makes a
    /// snapshot reproducible rather than merely hashed: the bytes are retained, so
    /// an answer can be rebuilt a year later from the same bytes it was computed
    /// from. A hash that was never recorded returns `None`, never a panic.
    pub fn bytes(&self, content_hash: &str) -> Option<&[u8]> {
        self.datasets.get(content_hash).map(|d| d.bytes.as_slice())
    }
}
