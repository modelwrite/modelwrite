// SPDX-License-Identifier: AGPL-3.0-or-later
use graph::{graph_stats, requirement_coverage};
use okf::types::OkfRoot;

fn expected() -> OkfRoot {
    serde_json::from_str(&test_support::load_okf_expected()).expect("corpus fixture parses")
}

fn okf(text: &str) -> OkfRoot {
    serde_json::from_str(text).expect("test OKF parses")
}

const TINY: &str = r#"{
  "project": "tiny",
  "exportedAt": "2026-09-17T00:00:00Z",
  "summary": {},
  "stateMachine": {"name": "tiny sm", "regions": []},
  "requirements": [
    {"id": "r1", "name": "R1", "kind": "requirement", "stereotypes": ["Requirement"], "attributes": [], "documentation": "", "reqId": "1.1", "reqText": "covered"},
    {"id": "r2", "name": "R2", "kind": "requirement", "stereotypes": ["Requirement"], "attributes": [], "documentation": "", "reqId": "1.2", "reqText": "uncovered"}
  ],
  "graph": {
    "nodes": [
      {"id": "p1", "kind": "block", "name": "P1"},
      {"id": "r1", "kind": "requirement", "name": "R1"},
      {"id": "r2", "kind": "requirement", "name": "R2"}
    ],
    "edges": [
      {"source": "p1", "target": "r1", "kind": "dependency", "label": "Satisfy"}
    ]
  }
}"#;

#[test]
fn corpus_is_fully_integrated() {
    let stats = graph_stats(&expected());
    assert_eq!(stats.node_count, 99);
    assert_eq!(stats.edge_count, 165);
    assert!(stats.isolated.is_empty(), "isolated: {:?}", stats.isolated);
    assert_eq!(stats.component_count, 1);
    assert_eq!(stats.component_sizes, vec![99]);
}

#[test]
fn isolated_node_is_detected() {
    let stats = graph_stats(&okf(TINY));
    assert_eq!(stats.isolated, vec!["r2"]);
}

#[test]
fn two_components_are_counted() {
    let stats = graph_stats(&okf(TINY));
    assert_eq!(stats.component_count, 2);
    assert_eq!(stats.component_sizes, vec![2, 1]);
}

#[test]
fn coverage_counts_traceability() {
    let cov = requirement_coverage(&okf(TINY));
    assert_eq!(cov.total, 2);
    assert_eq!(cov.satisfied, 1);
    assert_eq!(cov.covered, 1);
    assert_eq!(cov.uncovered, vec!["r2"]);
}

/// The reference model carries 20 Satisfy, 3 Refine, 1 Verify and 3 Allocate edges,
/// and 15 of its 25 requirements are covered by at least one of them. Pinning the
/// coverage keeps this measured claim honest: it is the number the gate reports.
#[test]
fn corpus_coverage_is_pinned() {
    let cov = requirement_coverage(&expected());
    assert_eq!(cov.total, 25);
    assert_eq!(cov.satisfied, 20);
    assert_eq!(cov.refined, 3);
    assert_eq!(cov.verified, 1);
    assert_eq!(cov.allocated, 0);
    assert_eq!(cov.covered, 15);
    assert_eq!(cov.uncovered.len(), 10);
}
/// Regression check for the coverage inflation bug: an allocate edge from a part to a
/// requirement must cover the requirement and must NOT add the part to covered. The old
/// unconditional insert would have reported covered = 2 for a single requirement.
#[test]
fn allocate_edge_counts_only_requirement_endpoints() {
    const ALLOC: &str = r#"{
  "project": "alloc",
  "exportedAt": "2026-09-17T00:00:00Z",
  "summary": {},
  "stateMachine": {"name": "alloc sm", "regions": []},
  "requirements": [
    {"id": "r1", "name": "R1", "kind": "requirement", "stereotypes": ["Requirement"], "attributes": [], "documentation": "", "reqId": "1.1", "reqText": "allocated to a part"}
  ],
  "graph": {
    "nodes": [
      {"id": "p1", "kind": "block", "name": "P1"},
      {"id": "r1", "kind": "requirement", "name": "R1"}
    ],
    "edges": [
      {"source": "p1", "target": "r1", "kind": "dependency", "label": "Allocate"}
    ]
  }
}"#;
    let cov = requirement_coverage(&okf(ALLOC));
    assert_eq!(cov.total, 1);
    assert_eq!(cov.allocated, 1);
    assert_eq!(cov.covered, 1);
    assert!(cov.uncovered.is_empty());
}
