// SPDX-License-Identifier: AGPL-3.0-or-later
//! The compositional gate: the single-model gate lifted one level (section 4). It runs
//! over the platform model's own document plus a bounded set of cross-model checks that
//! are provable WITHOUT importing any subsystem's graph (section 4.1), and it states its
//! measured-vs-asserted boundary as data (section 4.2), never as prose.
//!
//! The three measured checks are: resolution (the named project and pinned revision
//! exist, and every bound element exists within that revision), was-gated (the pinned
//! revision carries a passing gate record), and platform coverage computed locally (every
//! platform requirement has a local satisfying element or a typed cross-model reference).
//! The one asserted claim is the global system-of-systems graph property, which the
//! engine cannot prove without importing every subsystem - the copy R1 forbids.

use std::collections::HashSet;

use okf::types::OkfRoot;
use serde_json::{json, Value};

use crate::api::{check_reference, load_model};
use crate::store::{Store, StoreError};

const MEASURED: [&str; 3] = [
    "every subsystem reference resolves at its pinned revision",
    "every integrated revision was itself gated",
    "every platform requirement is covered within the platform model",
];

const ASSERTED: [&str; 1] = [
    "the combined system-of-systems, as a whole, has the global graph property (connected, coverage-complete, orphan-free); proving it would require importing every subsystem, which is the copy R1 forbids",
];

pub struct CompositionReport {
    pub failures: Vec<String>,
    pub evidence: Value,
}

pub fn check(store: &dyn Store, platform: &OkfRoot) -> Result<CompositionReport, StoreError> {
    let mut failures: Vec<String> = Vec::new();

    let mut resolution = Vec::new();
    let mut resolved: Vec<bool> = Vec::with_capacity(platform.references.len());
    for reference in &platform.references {
        let mut reason = check_reference(store, &reference.project, &reference.revision)?;
        if reason.is_none() && !reference.bounds.is_empty() {
            let subsystem = load_model(store, &reference.project, &reference.revision)?;
            let ids = okf::diff::element_ids(&subsystem);
            let missing: Vec<String> = reference
                .bounds
                .iter()
                .filter(|b| !ids.contains(*b))
                .cloned()
                .collect();
            if !missing.is_empty() {
                reason = Some(format!(
                    "bound element(s) {} not found in {}@{}",
                    missing.join(", "),
                    reference.project,
                    short(&reference.revision)
                ));
            }
        }
        let resolves = reason.is_none();
        if !resolves {
            failures.push(format!(
                "reference to {} does not resolve: {}",
                reference.project,
                reason.clone().unwrap_or_default()
            ));
        }
        resolved.push(resolves);
        resolution.push(json!({
            "kind": "measured",
            "project": reference.project,
            "revision": reference.revision,
            "role": reference.role,
            "bounds": reference.bounds,
            "resolves": resolves,
            "reason": reason,
        }));
    }

    let mut was_gated = Vec::new();
    for (index, reference) in platform.references.iter().enumerate() {
        let resolves = resolved[index];
        let gated = if resolves {
            let runs = store.gate_runs_for_commit(&reference.project, &reference.revision)?;
            runs.iter().any(|run| run.passed)
        } else {
            false
        };
        if resolves && !gated {
            failures.push(format!(
                "reference to {}@{} is not gated: the pinned revision has no passing gate record",
                reference.project,
                short(&reference.revision)
            ));
        }
        was_gated.push(json!({
            "kind": "measured",
            "project": reference.project,
            "revision": reference.revision,
            "role": reference.role,
            "gated": gated,
        }));
    }

    let (local, cross_model_only) = platform_coverage(platform);

    let evidence = json!({
        "boundary": {
            "measured": MEASURED,
            "asserted": ASSERTED,
        },
        "resolution": resolution,
        "wasGated": was_gated,
        "coverage": {
            "local": local,
            "crossModelOnly": cross_model_only,
        },
    });

    Ok(CompositionReport { failures, evidence })
}

fn short(revision: &str) -> &str {
    let end = revision.len().min(8);
    revision.get(..end).unwrap_or(revision)
}

fn platform_coverage(platform: &OkfRoot) -> (Vec<String>, Vec<String>) {
    if platform.graph.is_none() {
        return (Vec::new(), Vec::new());
    }

    let bound_ids: HashSet<&str> = platform
        .references
        .iter()
        .flat_map(|r| r.bounds.iter().map(|s| s.as_str()))
        .collect();

    let full = graph::requirement_coverage(platform);
    let full_uncovered: HashSet<&str> = full.uncovered.iter().map(|s| s.as_str()).collect();

    if bound_ids.is_empty() {
        let mut local: Vec<String> = platform
            .requirements
            .iter()
            .map(|r| r.id.clone())
            .filter(|id| !full_uncovered.contains(id.as_str()))
            .collect();
        local.sort();
        return (local, Vec::new());
    }

    let mut local_view = platform.clone();
    if let Some(g) = local_view.graph.as_mut() {
        g.nodes.retain(|n| !bound_ids.contains(n.id.as_str()));
        g.edges.retain(|e| {
            !bound_ids.contains(e.source.as_str()) && !bound_ids.contains(e.target.as_str())
        });
    }
    let local_cov = graph::requirement_coverage(&local_view);
    let local_uncovered: HashSet<&str> = local_cov.uncovered.iter().map(|s| s.as_str()).collect();

    let mut local: Vec<String> = platform
        .requirements
        .iter()
        .map(|r| r.id.clone())
        .filter(|id| !local_uncovered.contains(id.as_str()))
        .collect();
    local.sort();

    let mut cross_model_only: Vec<String> = local_cov
        .uncovered
        .iter()
        .filter(|id| !full_uncovered.contains(id.as_str()))
        .cloned()
        .collect();
    cross_model_only.sort();

    (local, cross_model_only)
}
