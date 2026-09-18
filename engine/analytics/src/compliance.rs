// SPDX-License-Identifier: AGPL-3.0-or-later
//! Portfolio compliance: which products meet which specifications. The answer has
//! THREE states, not two, and blurring them is the single most harmful thing this
//! slice could ship. A requirement present and covered is SATISFIED; present and
//! uncovered is NOT SATISFIED; ABSENT from the model entirely is UNKNOWN. A missing
//! requirement and a met requirement must never look alike, so absence is always an
//! explicit UNKNOWN - never a blank, never omitted, never a pass. Coverage is the
//! graph engine's, never a reimplementation: a second coverage rule would be a
//! second source of truth.

use graph::requirement_coverage;
use okf::types::OkfRoot;
use serde::{Deserialize, Serialize};

/// The state of one requirement against one model. Three states, not two: a
/// requirement the model does not mention at all is UNKNOWN, and must be rendered
/// as UNKNOWN - never as a blank, because a blank reads exactly like a pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Compliance {
    /// Present in the model and covered by the engine's graph coverage.
    Satisfied,
    /// Present in the model, but the engine's graph coverage shows no
    /// Satisfy/Refine/Verify/Allocate edge covering it.
    NotSatisfied,
    /// Absent from the model entirely. The most common answer, and the most
    /// dangerous to render as a blank.
    Unknown,
}

/// Classify one requirement against one model. The requirement is identified by
/// its `id`, exactly as the engine's coverage keys it. `Satisfied` and
/// `NotSatisfied` come from the engine's own `requirement_coverage`; `Unknown`
/// is a requirement the model does not contain at all.
pub fn classify(requirement: &str, model: &OkfRoot) -> Compliance {
    let present = model.requirements.iter().any(|r| r.id == requirement);
    if !present {
        return Compliance::Unknown;
    }
    // A model with no graph section has no coverage edges, so every present
    // requirement is uncovered: the same reading the gate records when a candidate
    // loses its graph. Guarding here keeps classify panic-free, because the engine's
    // coverage helper expects a graph to exist.
    if model.graph.is_none() {
        return Compliance::NotSatisfied;
    }
    let coverage = requirement_coverage(model);
    if coverage.uncovered.iter().any(|id| id == requirement) {
        Compliance::NotSatisfied
    } else {
        Compliance::Satisfied
    }
}

/// One model's compliance against the whole specification. Every requirement in
/// the specification lands in exactly one bucket, so the three counts always sum
/// to the specification's size. A requirement can never hide: it is here in one
/// of the three buckets, or the report is incomplete.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelCompliance {
    /// The model's project identifier, so the report says WHICH product.
    pub model: String,
    /// Requirements of the specification this model satisfies (present and covered).
    pub satisfied: Vec<String>,
    /// Requirements the model has but does not cover (present and uncovered).
    pub not_satisfied: Vec<String>,
    /// Requirements absent from this model. Never omitted, never blank.
    pub unknown: Vec<String>,
}

impl ModelCompliance {
    pub fn satisfied_count(&self) -> usize {
        self.satisfied.len()
    }

    pub fn not_satisfied_count(&self) -> usize {
        self.not_satisfied.len()
    }

    pub fn unknown_count(&self) -> usize {
        self.unknown.len()
    }

    /// The total number of requirements accounted for. Always equals the
    /// specification's size, so a report whose rows do not sum to the
    /// specification is provably hiding a row.
    pub fn total(&self) -> usize {
        self.satisfied.len() + self.not_satisfied.len() + self.unknown.len()
    }
}

/// A portfolio report: every model classified against the same specification,
/// grouped by state. The counts in each model sum to `specification_size`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    /// The number of requirements in the specification. Every model's three
    /// counts sum to exactly this.
    pub specification_size: usize,
    /// One row per model, in the order the models were supplied.
    pub models: Vec<ModelCompliance>,
}

/// Classify every requirement of `requirement_set` against every model, grouping
/// each model's results by state. An empty requirement set is an explicit empty
/// report (specification size 0), not a panic; an empty model list is an empty
/// `models` row set, also not a panic.
pub fn portfolio_report(requirement_set: &[&str], models: &[&OkfRoot]) -> Report {
    let models = models
        .iter()
        .map(|model| {
            let mut satisfied = Vec::new();
            let mut not_satisfied = Vec::new();
            let mut unknown = Vec::new();
            for requirement in requirement_set {
                match classify(requirement, model) {
                    Compliance::Satisfied => satisfied.push((*requirement).to_string()),
                    Compliance::NotSatisfied => not_satisfied.push((*requirement).to_string()),
                    Compliance::Unknown => unknown.push((*requirement).to_string()),
                }
            }
            ModelCompliance {
                model: model.project.clone(),
                satisfied,
                not_satisfied,
                unknown,
            }
        })
        .collect();
    Report {
        specification_size: requirement_set.len(),
        models,
    }
}
