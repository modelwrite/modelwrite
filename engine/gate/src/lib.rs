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

    // A candidate can lose its graph section entirely. Validation already records that
    // as an error, so the gate must report a failure rather than panic inside the graph
    // helpers, which require a graph to exist. Exit code 1 is the contract for a lossy
    // candidate; a crash would be neither a pass nor a failure.
    let stats = if candidate.graph.is_some() {
        Some(graph_stats(candidate))
    } else {
        failures.push("integration: candidate has no graph section".to_string());
        None
    };
    if let Some(stats) = &stats {
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
    }

    let cov = if candidate.graph.is_some() {
        Some(requirement_coverage(candidate))
    } else {
        None
    };
    if let Some(cov) = &cov {
        if strict_coverage && !cov.uncovered.is_empty() {
            failures.push(format!(
                "coverage: {} uncovered requirements",
                cov.uncovered.len()
            ));
        }
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
        // The schema stays stable when the candidate has no graph: the keys are always
        // present, with empty values, so a consumer never has to handle a missing object.
        "integration": match &stats {
            Some(s) => json!({
                "isolated": s.isolated,
                "componentCount": s.component_count,
                "componentSizes": s.component_sizes
            }),
            None => json!({
                "isolated": [],
                "componentCount": 0,
                "componentSizes": []
            })
        },
        "coverage": match &cov {
            Some(c) => json!({
                "total": c.total,
                "satisfied": c.satisfied,
                "refined": c.refined,
                "verified": c.verified,
                "allocated": c.allocated,
                "covered": c.covered,
                "uncovered": c.uncovered
            }),
            // Without a graph nothing can be shown to be covered, but the requirement
            // count is still known: reporting total 0 would understate the model. Every
            // requirement the candidate retains is reported as uncovered, which is the
            // truthful reading and keeps the evidence schema stable.
            None => {
                let mut uncovered: Vec<String> =
                    candidate.requirements.iter().map(|r| r.id.clone()).collect();
                uncovered.sort();
                json!({
                    "total": candidate.requirements.len(),
                    "satisfied": 0,
                    "refined": 0,
                    "verified": 0,
                    "allocated": 0,
                    "covered": 0,
                    "uncovered": uncovered
                })
            }
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
