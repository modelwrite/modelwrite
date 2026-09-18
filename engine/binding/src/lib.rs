// SPDX-License-Identifier: AGPL-3.0-or-later
//! The binding contract: a versioned adapter between a source standard and OKF,
//! plus the fidelity harness that measures - rather than trusts - what it keeps.

pub mod report;

use std::fmt;

use okf::diff;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub use okf::diff::DiffReport;
pub use okf::types::OkfRoot;
pub use report::{LossReport, Mapping, MappingVerdict};

/// Which directions a binding supports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Direction {
    /// Reads a source artifact into OKF but cannot write it back. A viewer.
    ImportOnly,
    /// Reads and writes; a round trip can be measured.
    ImportAndExport,
}

/// The identity and capability a binding declares about itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BindingInfo {
    pub id: String,
    pub version: String,
    pub direction: Direction,
    pub description: String,
}

/// The single error type the binding contract uses, so a harness can reject a
/// binding without trusting its claims.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BindingError {
    /// The binding could not read a source artifact into OKF.
    Import(String),
    /// The binding could not write an OKF document back to its source form.
    Export(String),
    /// The binding declares itself a viewer and therefore cannot round-trip.
    Viewer { id: String },
    /// `round_trip` was handed a source that is not a valid OKF document, so
    /// the engine has no reference to measure against.
    NotOkf(String),
}

impl fmt::Display for BindingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BindingError::Import(m) => write!(f, "binding import failed: {}", m),
            BindingError::Export(m) => write!(f, "binding export failed: {}", m),
            BindingError::Viewer { id } => {
                write!(f, "binding {} is a viewer and cannot export", id)
            }
            BindingError::NotOkf(m) => write!(f, "source is not a valid OKF document: {}", m),
        }
    }
}

impl std::error::Error for BindingError {}

/// A versioned adapter between a source standard and OKF, with a declared
/// direction and a mapping matrix. Its fidelity is measured by the harness,
/// never taken on faith.
pub trait Binding {
    fn info(&self) -> BindingInfo;
    /// The declarative mapping matrix: the source constructs this binding
    /// understands, each marked exact, lossy or unmappable. Data the gate reads;
    /// a binding that declares nothing understands nothing.
    fn mapping_table(&self) -> Vec<Mapping> {
        Vec::new()
    }
    fn import(&self, source: &[u8]) -> Result<(OkfRoot, LossReport), BindingError>;
    fn export(&self, root: &OkfRoot) -> Result<Vec<u8>, BindingError>;
}

/// The measured result of a round trip: the import loss report (the binding's
/// own claim) plus the engine's diff (the engine's measurement of the same
/// journey). The two must agree before anyone trusts the binding.
#[derive(Debug, Clone, Serialize)]
pub struct FidelityOutcome {
    /// Content address of the source document, computed before anything else.
    pub artifact_hash: String,
    pub loss_report: LossReport,
    pub diff: DiffReport,
}

/// Content-address a byte slice: sha256, hex-encoded. This is the fingerprint
/// the platform records before anything else happens, so a migration can never
/// destroy the thing it migrated.
pub fn artifact_hash(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// Round-trip a source OKF document through a binding and back, then measure
/// the result with the engine's own diff against the engine's own reading of
/// the source.
///
/// The reference is never the binding's own reading: it is `serde_json` read
/// directly into `OkfRoot`, which is exactly what is under test. A viewer is
/// refused outright; a binding whose metadata claims export but whose export
/// fails is rejected, never trusted; anything dropped on the way is named by
/// the diff.
pub fn round_trip(binding: &dyn Binding, source: &[u8]) -> Result<FidelityOutcome, BindingError> {
    // Content-address the source before anything else happens.
    let artifact_hash = artifact_hash(source);

    // The engine reads the original document directly.
    let reference: OkfRoot =
        serde_json::from_slice(source).map_err(|e| BindingError::NotOkf(e.to_string()))?;

    // A viewer declares itself unable to export; there is nothing to round-trip.
    let info = binding.info();
    if info.direction == Direction::ImportOnly {
        return Err(BindingError::Viewer { id: info.id });
    }

    // Export to the source binding, then import back. A binding whose metadata
    // claims export but whose export fails is rejected, never trusted.
    let exported = binding.export(&reference)?;
    let (round_tripped, mut loss_report) = binding.import(&exported)?;

    // The report's own `artifact_hash` is the binding's business - a binding hashes what it
    // was handed. The harness overwrites it with the hash of the ORIGINAL source, because a
    // consumer correlating a report with the artifact retained in the repository must find
    // the same value; two different hashes under one field name is a trap.
    loss_report.artifact_hash = artifact_hash.clone();

    // The engine's own diff between the round-tripped document and the original.
    let diff = diff::diff(&reference, &round_tripped);

    Ok(FidelityOutcome {
        artifact_hash,
        loss_report,
        diff,
    })
}
