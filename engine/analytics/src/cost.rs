// SPDX-License-Identifier: AGPL-3.0-or-later
//! Cost joins: which requirements are the most expensive, and which have no
//! estimate. Cost never lives in a model - it lives in an ERP, a contract, a
//! spreadsheet - so this is the first place the platform joins its own provable
//! data to somebody else's records.
//!
//! The ruling that governs it: AN UNCOSTED REQUIREMENT IS NOT A ZERO. Zero is a
//! number somebody would act on - it would sit at the bottom of a sorted list and
//! be treated as free. A requirement nobody has estimated must say UNCOSTED, for
//! the same reason compliance says UNKNOWN: absence must never render as a value.
//!
//! Every figure carries its source and its date, because a cost from three years
//! ago is an estimate about the past no matter how reliable the spreadsheet was.
//! When one requirement is priced by more than one dataset, the costs are fused
//! with the weakest-link rule from the confidence module, reused rather than
//! restated.

use std::collections::{HashMap, HashSet};
use std::fmt;

use okf::types::OkfRoot;
use serde::{Deserialize, Serialize};

use crate::confidence::{self, FusedValue, FusionError, Value};
use crate::dataset::Dataset;

/// Which column of a dataset holds the requirement id, and which holds the cost.
/// The mapping is declared per dataset because no two organisations store cost the
/// same way: one names its columns "requirement_id" and "unit_cost", another "req"
/// and "amount". The names here are the dataset's own header cells.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ColumnMapping {
    /// The header name of the column whose cells name a requirement.
    pub requirement_id: String,
    /// The header name of the column whose cells hold the cost.
    pub cost: String,
}

impl ColumnMapping {
    pub fn new(requirement_id: impl Into<String>, cost: impl Into<String>) -> Self {
        Self {
            requirement_id: requirement_id.into(),
            cost: cost.into(),
        }
    }
}

/// The cost state of one requirement. Uncosted is a state, never a number: zero
/// is a number somebody would act on, and absence must never render as a value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Cost {
    /// No dataset has a cost record for this requirement.
    Uncosted,
    /// One or more datasets priced this requirement. The value is their sum, the
    /// trust is the weakest link, and every source and read time is named.
    Costed(FusedValue<i64>),
}

impl Cost {
    /// Whether this requirement has no estimate. An uncosted requirement is never
    /// reported as a zero: it is reported as uncosted.
    pub fn is_uncosted(&self) -> bool {
        matches!(self, Cost::Uncosted)
    }
}

/// One requirement and its cost state. An uncosted requirement still appears here
/// - it is Cost::Uncosted, never omitted and never zero.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CostedRequirement {
    /// The requirement's id, exactly as the model and the graph coverage key it.
    pub requirement: String,
    /// The cost state: a fused cost, or Uncosted.
    pub cost: Cost,
}

/// The error a cost join returns when it refuses to answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CostError {
    /// The dataset has no column with the declared header name.
    MissingColumn { source: String, column: String },
    /// A row's cost cell is absent or not a whole-number amount.
    MalformedCost {
        source: String,
        requirement: String,
        value: String,
    },
    /// Fusion refused. This is FusionError::NoSources, which cannot arise when a
    /// record exists, but is propagated rather than assumed away.
    Fusion(FusionError),
}

impl fmt::Display for CostError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CostError::MissingColumn { source, column } => {
                write!(f, "dataset {} has no column named {}", source, column)
            }
            CostError::MalformedCost {
                source,
                requirement,
                value,
            } => {
                if requirement.is_empty() {
                    write!(
                        f,
                        "dataset {} has a row too short for its declared columns",
                        source
                    )
                } else if value.is_empty() {
                    write!(
                        f,
                        "dataset {} has no cost value for requirement {}",
                        source, requirement
                    )
                } else {
                    write!(
                        f,
                        "dataset {} has a cost {:?} for requirement {} that is not a whole-number amount",
                        source, value, requirement
                    )
                }
            }
            CostError::Fusion(e) => write!(f, "{}", e),
        }
    }
}

impl std::error::Error for CostError {}

/// Join the requirements of the models to cost records in the datasets, returning
/// one CostedRequirement per distinct requirement in first-seen order. A
/// requirement with no cost record in any dataset is Cost::Uncosted - it still
/// appears, and it is never reported as a zero. A requirement priced by one
/// dataset carries that dataset's value, source and read time; a requirement
/// priced by several has its costs summed with the weakest-link trust rule.
///
/// Each datasets entry pairs a dataset with the column mapping declaring which of
/// its columns is the requirement id and which is the cost. A dataset missing a
/// declared column is a clear error, never a silent zero or a partial result.
pub fn cost_by_requirement(
    models: &[&OkfRoot],
    datasets: &[(&Dataset, &ColumnMapping)],
) -> Result<Vec<CostedRequirement>, CostError> {
    // The union of every model's requirement ids, in first-seen order, so a
    // requirement present in several models still yields one row and cannot hide.
    let mut requirements = Vec::new();
    let mut seen = HashSet::new();
    for model in models {
        for requirement in &model.requirements {
            if seen.insert(requirement.id.as_str()) {
                requirements.push(requirement.id.clone());
            }
        }
    }

    // Every cost record, keyed by requirement id. A record carries the value and,
    // through the dataset's own source, the source id, trust and read time.
    let mut records: HashMap<String, Vec<Value<i64>>> = HashMap::new();
    for (dataset, mapping) in datasets {
        let (req_col, cost_col) = resolve_columns(dataset, mapping)?;
        let source_id = dataset.source.id.clone();
        for row in dataset.rows.iter().skip(1) {
            let requirement = match row.get(req_col) {
                Some(cell) => cell.clone(),
                None => {
                    return Err(CostError::MalformedCost {
                        source: source_id.clone(),
                        requirement: String::new(),
                        value: String::new(),
                    })
                }
            };
            let raw = match row.get(cost_col) {
                Some(cell) => cell.clone(),
                None => {
                    return Err(CostError::MalformedCost {
                        source: source_id.clone(),
                        requirement: requirement.clone(),
                        value: String::new(),
                    })
                }
            };
            let value: i64 = match raw.trim().parse() {
                Ok(value) => value,
                Err(_) => {
                    return Err(CostError::MalformedCost {
                        source: source_id.clone(),
                        requirement: requirement.clone(),
                        value: raw,
                    })
                }
            };
            records.entry(requirement).or_default().push(Value::new(
                value,
                source_id.clone(),
                dataset.source.trust,
                dataset.captured_at.clone(),
            ));
        }
    }

    let mut result = Vec::with_capacity(requirements.len());
    for requirement in requirements {
        match records.remove(&requirement) {
            Some(values) => {
                let fused = confidence::sum(&values).map_err(CostError::Fusion)?;
                result.push(CostedRequirement {
                    requirement,
                    cost: Cost::Costed(fused),
                });
            }
            None => result.push(CostedRequirement {
                requirement,
                cost: Cost::Uncosted,
            }),
        }
    }
    Ok(result)
}

/// Resolve the declared column names to their indices in the dataset's header
/// row. A dataset missing a declared column is a clear error.
fn resolve_columns(
    dataset: &Dataset,
    mapping: &ColumnMapping,
) -> Result<(usize, usize), CostError> {
    let header = dataset
        .rows
        .first()
        .ok_or_else(|| CostError::MissingColumn {
            source: dataset.source.id.clone(),
            column: mapping.requirement_id.clone(),
        })?;
    let req_col = header
        .iter()
        .position(|name| name == &mapping.requirement_id)
        .ok_or_else(|| CostError::MissingColumn {
            source: dataset.source.id.clone(),
            column: mapping.requirement_id.clone(),
        })?;
    let cost_col = header
        .iter()
        .position(|name| name == &mapping.cost)
        .ok_or_else(|| CostError::MissingColumn {
            source: dataset.source.id.clone(),
            column: mapping.cost.clone(),
        })?;
    Ok((req_col, cost_col))
}
