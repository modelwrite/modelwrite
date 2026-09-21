// SPDX-License-Identifier: AGPL-3.0-or-later
//! The metric catalog: the single, engine-side source of truth for every metric
//! the analytics schema exposes. The schema's `metric_definitions` table is
//! GENERATED from this catalog (see `examples/gen_metric_definitions.rs`), so the
//! published table and the analytics page read one list and can never disagree.
//!
//! Every id here names a scalar the engine's existing computations actually
//! produce - `graph::graph_stats`, `graph::requirement_coverage`,
//! `portfolio_report`, `cost_by_requirement` and `gate::run`. Rule 7 of the
//! analytics-interfaces brief forbids inventing a column the engine does not carry:
//! `ENGINE_METRIC_IDS` is the closed list, and `tests/metric_definitions.rs`
//! fails when a definition is missing or when a definition has no engine metric
//! behind it.

use serde::Serialize;

/// The lifecycle of a metric in the schema. Within a major version changes are
/// additive only, so a new metric enters as `Active` and a removed metric keeps
/// its row (marked deprecated) rather than disappearing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MetricStatus {
    Active,
}

/// One `metric_definitions` row: the id, its human name, what it means, the
/// engine constructs it depends on, and its status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MetricDefinition {
    pub id: String,
    pub name: String,
    pub description: String,
    #[serde(rename = "depends_on")]
    pub depends_on: Vec<String>,
    pub status: MetricStatus,
}

/// The metric ids the engine's computations produce. This is the closed list a
/// definition must cover exactly: it names every scalar the analytics schema
/// carries, and nothing else. A metric added to the engine without a row in
/// `metric_definitions()` - or a definition added without an engine metric here -
/// fails `tests/metric_definitions.rs`.
pub const ENGINE_METRIC_IDS: &[&str] = &[
    // Graph health - the graph-gaps analytic (ruling 4 of the open/licensed boundary):
    // orphans, isolated groups, dangling links.
    "graph.node_count",
    "graph.edge_count",
    "graph.isolated_count",
    "graph.component_count",
    // Requirement coverage - the engine's own coverage report.
    "coverage.total",
    "coverage.satisfied",
    "coverage.refined",
    "coverage.verified",
    "coverage.allocated",
    "coverage.covered",
    "coverage.uncovered",
    // Portfolio compliance - Covered / Uncovered / Unknown, three states, never blank.
    "compliance.specification_size",
    "compliance.covered_count",
    "compliance.uncovered_count",
    "compliance.unknown_count",
    // Cost - Costed / Uncosted, never zero and never omitted.
    "cost.costed_count",
    "cost.uncosted_count",
    // The gate - round-trip fidelity and validation.
    "gate.passed",
    "gate.validation_errors",
    "gate.validation_warnings",
    "gate.roundtrip.missing_elements",
    "gate.roundtrip.extra_elements",
    "gate.roundtrip.missing_edges",
    "gate.roundtrip.extra_edges",
    "gate.roundtrip.changed_attributes",
];

fn def(id: &str, name: &str, description: &str, depends_on: &[&str]) -> MetricDefinition {
    MetricDefinition {
        id: id.to_string(),
        name: name.to_string(),
        description: description.to_string(),
        depends_on: depends_on.iter().map(|s| s.to_string()).collect(),
        status: MetricStatus::Active,
    }
}

/// The full catalog, in the order of `ENGINE_METRIC_IDS` so the generated
/// `metric_definitions` table is deterministic (design decision 5).
pub fn metric_definitions() -> Vec<MetricDefinition> {
    vec![
        def(
            "graph.node_count",
            "Graph nodes",
            "Number of nodes in the graph section (every element as a node).",
            &["graph.nodes"],
        ),
        def(
            "graph.edge_count",
            "Graph edges",
            "Number of edges in the graph section (every relationship as an edge).",
            &["graph.edges"],
        ),
        def(
            "graph.isolated_count",
            "Orphaned elements",
            "Number of graph nodes with no incident edge. The graph-gaps headline: every orphan is named in graph_stats.isolated.",
            &["graph.nodes", "graph.edges"],
        ),
        def(
            "graph.component_count",
            "Isolated groups",
            "Number of connected components in the graph. More than one means the model is split into isolated groups.",
            &["graph.nodes", "graph.edges"],
        ),
        def(
            "coverage.total",
            "Requirements total",
            "Number of requirements in the model.",
            &["requirements"],
        ),
        def(
            "coverage.satisfied",
            "Satisfied links",
            "Number of dependency edges labelled Satisfy that target a requirement.",
            &["graph.edges"],
        ),
        def(
            "coverage.refined",
            "Refined links",
            "Number of dependency edges labelled Refine that target a requirement.",
            &["graph.edges"],
        ),
        def(
            "coverage.verified",
            "Verified links",
            "Number of dependency edges labelled Verify that target a requirement.",
            &["graph.edges"],
        ),
        def(
            "coverage.allocated",
            "Allocated links",
            "Number of dependency edges labelled Allocate with a requirement endpoint.",
            &["graph.edges"],
        ),
        def(
            "coverage.covered",
            "Covered requirements",
            "Number of requirements covered by at least one Satisfy/Refine/Verify/Allocate edge.",
            &["requirements", "graph.edges"],
        ),
        def(
            "coverage.uncovered",
            "Uncovered requirements",
            "Number of requirements with no covering edge. Each is named in the coverage report's uncovered list.",
            &["requirements", "graph.edges"],
        ),
        def(
            "compliance.specification_size",
            "Specification size",
            "Number of requirements in the caller's specification (the portfolio question).",
            &["requirements"],
        ),
        def(
            "compliance.covered_count",
            "Specification covered",
            "Requirements of the specification present and covered by the model.",
            &["requirements", "graph.edges"],
        ),
        def(
            "compliance.uncovered_count",
            "Specification uncovered",
            "Requirements of the specification present in the model but uncovered.",
            &["requirements", "graph.edges"],
        ),
        def(
            "compliance.unknown_count",
            "Specification unknown",
            "Requirements of the specification absent from the model entirely. Never rendered as a blank.",
            &["requirements"],
        ),
        def(
            "cost.costed_count",
            "Costed requirements",
            "Number of requirements with at least one cost record across the supplied datasets.",
            &["requirements", "dataset"],
        ),
        def(
            "cost.uncosted_count",
            "Uncosted requirements",
            "Number of requirements with no cost record. Never rendered as zero.",
            &["requirements", "dataset"],
        ),
        def(
            "gate.passed",
            "Gate passed",
            "Whether the gate run passed (1) or failed (0) against its reference.",
            &["gate"],
        ),
        def(
            "gate.validation_errors",
            "Validation errors",
            "Number of validation errors the gate recorded for the candidate.",
            &["gate", "validate"],
        ),
        def(
            "gate.validation_warnings",
            "Validation warnings",
            "Number of validation warnings the gate recorded, including dangling edge endpoints.",
            &["gate", "validate"],
        ),
        def(
            "gate.roundtrip.missing_elements",
            "Missing elements",
            "Elements present in the reference but missing from the candidate after a round trip.",
            &["gate", "diff"],
        ),
        def(
            "gate.roundtrip.extra_elements",
            "Extra elements",
            "Elements present in the candidate but not the reference after a round trip.",
            &["gate", "diff"],
        ),
        def(
            "gate.roundtrip.missing_edges",
            "Missing edges",
            "Edges present in the reference but missing from the candidate after a round trip.",
            &["gate", "diff"],
        ),
        def(
            "gate.roundtrip.extra_edges",
            "Extra edges",
            "Edges present in the candidate but not the reference after a round trip.",
            &["gate", "diff"],
        ),
        def(
            "gate.roundtrip.changed_attributes",
            "Changed attributes",
            "Attributes whose value differs between reference and candidate after a round trip.",
            &["gate", "diff"],
        ),
    ]
}
