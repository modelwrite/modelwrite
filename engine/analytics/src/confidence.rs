// SPDX-License-Identifier: AGPL-3.0-or-later
//! Confidence and fusion: the rule that governs every number the analytics slice
//! reports. A fused answer must be traceable to its sources, and its confidence
//! must be its weakest link. A total built from one measured value and three
//! estimated ones is an ESTIMATE, and must say so - the numbers are all correct
//! and the claim would be false. A number without a source is not reportable, and
//! a fused value with no sources is an error, not a zero: a total of nothing is
//! unknown, and zero is a number somebody would act on.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::ops::Add;

use serde::{Deserialize, Serialize};

use crate::source::TrustLevel;

/// A single number attributed to a single source. The trust level travels with the
/// value from the moment it is read - it is never supplied at fusion time, because
/// a query has no business grading its own inputs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Value<T> {
    /// The number itself.
    pub value: T,
    /// The id of the source this number came from. A number without a source is
    /// not reportable; the registry resolves this id to the full `Source`.
    pub source: String,
    /// The trust level of the source, carried with the value.
    pub trust: TrustLevel,
    /// When the source was read. A cost from three years ago is an estimate about
    /// the past, regardless of how reliable its source is.
    pub captured_at: String,
}

impl<T> Value<T> {
    pub fn new(
        value: T,
        source: impl Into<String>,
        trust: TrustLevel,
        captured_at: impl Into<String>,
    ) -> Self {
        Self {
            value,
            source: source.into(),
            trust,
            captured_at: captured_at.into(),
        }
    }

    /// Whether this value is an estimate: the weakest, least trustworthy state.
    pub fn is_estimate(&self) -> bool {
        matches!(self.trust, TrustLevel::Estimated)
    }
}

/// The weaker of two trust levels. Estimated is weakest, then Reported, then
/// Measured. Confidence is never averaged and never rounded up: one estimated
/// input makes the whole answer estimated.
fn weakest(a: TrustLevel, b: TrustLevel) -> TrustLevel {
    use TrustLevel::*;
    match (a, b) {
        (Estimated, _) | (_, Estimated) => Estimated,
        (Reported, _) | (_, Reported) => Reported,
        (Measured, Measured) => Measured,
    }
}

/// The weakest trust among the inputs. Returns `None` when there are no inputs:
/// a fused value over zero sources is an error ([`FusionError::NoSources`]), not a
/// default, because a trust level for nothing would itself be a lie.
pub fn fused_trust<T>(values: &[Value<T>]) -> Option<TrustLevel> {
    values.iter().map(|v| v.trust).reduce(weakest)
}

/// The result of fusing values from several sources: the number, its weakest
/// trust, the complete set of contributing sources, and each source's read time.
/// Every fused value names all its sources; nothing here is anonymous.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FusedValue<T> {
    /// The fused number.
    pub value: T,
    /// The weakest trust among every contributing source.
    pub trust: TrustLevel,
    /// Every source that contributed, by id. No majority of measured values can
    /// mask a single estimated one: it is still named here.
    pub sources: BTreeSet<String>,
    /// Each contributing source's read time, keyed by source id, so a figure built
    /// partly from a three-year-old cost still says which part is the old one.
    pub captured_at: BTreeMap<String, String>,
}

impl<T> FusedValue<T> {
    /// Whether the fused answer is an estimate - true when any one of its inputs
    /// was estimated, however many of the rest were measured.
    pub fn is_estimate(&self) -> bool {
        matches!(self.trust, TrustLevel::Estimated)
    }
}

/// The error fusion returns when it refuses to produce an answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FusionError {
    /// Fusing zero values was requested. A total of nothing is not zero - it is
    /// unknown - and zero is a number somebody would act on.
    NoSources,
}

impl fmt::Display for FusionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FusionError::NoSources => {
                write!(f, "a fused value must have at least one source")
            }
        }
    }
}

impl std::error::Error for FusionError {}

/// Combine values with a binary operation, producing a fused value that carries
/// the weakest trust, the complete set of contributing sources, and every
/// source's read time. Fusing nothing is a typed error, never a zero.
fn fuse_with<T>(
    values: &[Value<T>],
    mut combine: impl FnMut(T, T) -> T,
) -> Result<FusedValue<T>, FusionError>
where
    T: Clone,
{
    let (first, rest) = values.split_first().ok_or(FusionError::NoSources)?;
    let trust = fused_trust(values).ok_or(FusionError::NoSources)?;

    let mut acc = first.value.clone();
    for v in rest {
        acc = combine(acc, v.value.clone());
    }

    let sources = values.iter().map(|v| v.source.clone()).collect();
    let captured_at = values
        .iter()
        .map(|v| (v.source.clone(), v.captured_at.clone()))
        .collect();

    Ok(FusedValue {
        value: acc,
        trust,
        sources,
        captured_at,
    })
}

/// The sum of the values, carrying the weakest trust and the set of contributing
/// sources. A sum of measured and estimated values is an ESTIMATE.
pub fn sum<T>(values: &[Value<T>]) -> Result<FusedValue<T>, FusionError>
where
    T: Add<Output = T> + Clone,
{
    fuse_with(values, |a, b| a + b)
}

/// The largest value, carrying the weakest trust and the set of contributing
/// sources. Comparison, like arithmetic, does not wash out a weak source.
pub fn max<T>(values: &[Value<T>]) -> Result<FusedValue<T>, FusionError>
where
    T: Ord + Clone,
{
    fuse_with(values, |a, b| std::cmp::max(a, b))
}

/// The smallest value, carrying the weakest trust and the set of contributing
/// sources.
pub fn min<T>(values: &[Value<T>]) -> Result<FusedValue<T>, FusionError>
where
    T: Ord + Clone,
{
    fuse_with(values, |a, b| std::cmp::min(a, b))
}
