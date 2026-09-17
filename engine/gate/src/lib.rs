// SPDX-License-Identifier: AGPL-3.0-or-later
use graph::{graph_stats, requirement_coverage};
use okf::{diff, hash, types::OkfRoot, validate};
use serde_json::json;

pub struct GateOutcome {
    pub passed: bool,
    pub failures: Vec<String>,
    pub evidence: serde_json::Value,
}

/// The provable gate: round-trip fidelity plus graph health, with evidence
/// recorded for every run. The evidence is deterministic: given the same
/// inputs, the record reproduces exactly.
pub fn run(reference: &OkfRoot, candidate: &OkfRoot, strict_coverage: bool) -> GateOutcome {
    let mut failures: Vec<String> = Vec::new();

    let d = diff::diff(reference, candidate);
    if !d.equal {
        failures.push(format!(
            "roundtrip: {} missing elements, {} extra elements, {} missing edges, {} extra edges, {} changed attributes",
            d.missing_elements.len(),
            d.extra_elements.len(),
            d.missing_edges.len(),
            d.extra_edges.len(),
            d.changed_attributes.len()
        ));
    }

    let v = validate::validate(candidate);
    if !v.valid {
        for e in &v.errors {
            failures.push(format!("validation: {}", e));
        }
    }

    let stats = graph_stats(candidate);
    if !stats.isolated.is_empty() {
        failures.push(format!(
            "integration: {} isolated nodes",
            stats.isolated.len()
        ));
    }
    if stats.component_count != 1 {
        failures.push(format!(
            "integration: {} connected components",
            stats.component_count
        ));
    }

    let cov = requirement_coverage(candidate);
    if strict_coverage && !cov.uncovered.is_empty() {
        failures.push(format!(
            "coverage: {} uncovered requirements",
            cov.uncovered.len()
        ));
    }

    let evidence = json!({
        "gateVersion": env!("CARGO_PKG_VERSION"),
        "okfVersion": "1.0",
        "referenceHash": hash::canonical_hash(reference),
        "candidateHash": hash::canonical_hash(candidate),
        "roundtrip": {
            "equal": d.equal,
            "missingElements": d.missing_elements,
            "extraElements": d.extra_elements,
            "missingEdges": d.missing_edges,
            "extraEdges": d.extra_edges,
            "changedAttributes": d.changed_attributes
        },
        "integration": {
            "isolated": stats.isolated,
            "componentCount": stats.component_count,
            "componentSizes": stats.component_sizes
        },
        "coverage": {
            "total": cov.total,
            "satisfied": cov.satisfied,
            "refined": cov.refined,
            "verified": cov.verified,
            "allocated": cov.allocated,
            "covered": cov.covered,
            "uncovered": cov.uncovered
        },
        "strictCoverage": strict_coverage,
        "validationErrors": v.errors,
        "validationWarnings": v.warnings,
        "passed": failures.is_empty(),
        "failures": failures
    });

    GateOutcome {
        passed: failures.is_empty(),
        failures,
        evidence,
    }
}

pub fn write_evidence(path: &std::path::Path, evidence: &serde_json::Value) -> anyhow::Result<()> {
    let text = serde_json::to_string_pretty(evidence)?;
    std::fs::write(path, text)?;
    Ok(())
}
