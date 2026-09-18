// SPDX-License-Identifier: AGPL-3.0-or-later
//! Confidence and fusion: the rule that governs every number the analytics slice
//! reports. A fused answer must be traceable to its sources, and its confidence
//! must be its weakest link. A total built from one measured value and three
//! estimated ones is an ESTIMATE, and must say so - the numbers are all correct
//! and the claim would be false. A number without a source is not reportable, and
//! a fused value with no sources is an error, not a zero: a total of nothing is
//! unknown, and zero is a number somebody would act on.
//!
//! Trust is never self-attested. A value's trust level is resolved from the source
//! REGISTRY at construction; the caller supplies only the source id, never a trust
//! level. So a caller cannot stamp Measured onto a number that came from a source
//! the registry knows as Estimated - the combinator is sound precisely because its
//! attribution is not a suggestion.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::ops::Add;

use serde::{Deserialize, Serialize};

use crate::source::{Registry, RegistryError, TrustLevel};

/// A single number attributed to a single source. Its trust level is resolved
/// from the registry when the value is built, so it cannot be forged at call
/// sites: the source id names a registered source, and the registry alone decides
/// how much to trust it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Value<T> {
    value: T,
    source: String,
    trust: TrustLevel,
    captured_at: String,
}

impl<T> Value<T> {
    /// Build a value from a REGISTERED source. The trust level is read from the
    /// registry, never supplied here; an unregistered source is refused rather
    /// than given an assumed trust.
    pub fn from_registry(
        value: T,
        source_id: impl Into<String>,
        captured_at: impl Into<String>,
        registry: &Registry,
    ) -> Result<Self, RegistryError> {
        let source = source_id.into();
        let trust = registry
            .source(&source)
            .map(|s| s.trust)
            .ok_or_else(|| RegistryError::UnregisteredSource(source.clone()))?;
        Ok(Self::stamped(value, source, trust, captured_at.into()))
    }

    /// Stamp a value with an already-resolved trust. Crate-private: only the
    /// analytics pipeline, which has consulted the registry, may set a trust.
    pub(crate) fn stamped(
        value: T,
        source: String,
        trust: TrustLevel,
        captured_at: String,
    ) -> Self {
        Self {
            value,
            source,
            trust,
            captured_at,
        }
    }

    /// The number itself.
    pub fn value(&self) -> &T {
        &self.value
    }

    /// The id of the registered source this number came from.
    pub fn source(&self) -> &str {
        &self.source
    }

    /// The trust level the registry fixed for this source.
    pub fn trust(&self) -> TrustLevel {
        self.trust
    }

    /// When the source was read.
    pub fn captured_at(&self) -> &str {
        &self.captured_at
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
/// a fused value over zero sources is an error ([FusionError::NoSources]), not a
/// default, because a trust level for nothing would itself be a lie.
pub fn fused_trust<T>(values: &[Value<T>]) -> Option<TrustLevel> {
    values.iter().map(|v| v.trust).reduce(weakest)
}

/// The result of fusing values from several sources: the number, its weakest
/// trust, the complete set of contributing sources, each source's read time, and
/// each source's own trust. Every fused value names all its sources and which of
/// them was the weak one; nothing here is anonymous.
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
    /// Each contributing source's trust, keyed by source id. The weakest-link
    /// `trust` says SOME source was weak; this map says WHICH, so a serialized
    /// finding is reproducible without the in-memory registry.
    pub trusts: BTreeMap<String, TrustLevel>,
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
/// the weakest trust, the complete set of contributing sources, every source's
/// read time, and every source's own trust. Fusing nothing is a typed error, never
/// a zero.
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
    let trusts = values.iter().map(|v| (v.source.clone(), v.trust)).collect();

    Ok(FusedValue {
        value: acc,
        trust,
        sources,
        captured_at,
        trusts,
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
