// SPDX-License-Identifier: AGPL-3.0-or-later
use serde::{Deserialize, Serialize};

/// How faithfully one mapping carries a source construct into OKF.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MappingVerdict {
    /// The target expresses the same thing; the migration is mechanical.
    Exact,
    /// The target expresses something close, and the difference matters.
    Lossy,
    /// The target cannot express it at all.
    Unmappable,
}

/// One entry in a binding's mapping matrix: a named subject, its verdict, and
/// the reason. Nothing is ever dropped silently; a construct the binding could
/// not carry is a `Lossy` or `Unmappable` entry here, with its subject named.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mapping {
    pub subject: String,
    pub verdict: MappingVerdict,
    pub note: String,
}

/// The record of everything a binding could not carry, produced during import.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LossReport {
    pub binding: crate::BindingInfo,
    pub mappings: Vec<Mapping>,
    pub artifact_hash: String,
}

impl LossReport {
    /// True when nothing was lost or left unmapped: every mapping is `Exact`.
    pub fn is_lossless(&self) -> bool {
        self.mappings
            .iter()
            .all(|m| m.verdict == MappingVerdict::Exact)
    }

    /// The entries a human must decide on: everything that is not `Exact`.
    pub fn blocking(&self) -> Vec<&Mapping> {
        self.mappings
            .iter()
            .filter(|m| m.verdict != MappingVerdict::Exact)
            .collect()
    }
}
