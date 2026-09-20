// SPDX-License-Identifier: AGPL-3.0-or-later
use okf::{hash, types::OkfRoot, validate};

fn expected() -> OkfRoot {
    serde_json::from_str(&test_support::load_okf_expected()).expect("corpus fixture parses")
}

#[test]
fn corpus_fixture_validates() {
    let report = validate::validate(&expected());
    assert!(report.valid, "unexpected errors: {:?}", report.errors);
    assert_eq!(report.errors, Vec::<String>::new());
    // The reference export carries two edges whose source element was never emitted as
    // a node; they are warned about, so the finding is recorded rather than silently
    // accepted. See the corpus README.
    assert_eq!(
        report
            .warnings
            .iter()
            .filter(|w| w.contains("dangling edge endpoint"))
            .count(),
        2,
        "expected exactly two dangling edge endpoints in the legacy export"
    );
}

#[test]
fn corpus_fixture_has_expected_graph() {
    let root = expected();
    let graph = root.graph.as_ref().expect("graph present");
    assert_eq!(graph.nodes.len(), 99);
    assert_eq!(graph.edges.len(), 165);
    assert_eq!(root.requirements.len(), 25);
}

#[test]
fn rejects_duplicate_element_ids() {
    let mut root = expected();
    if root.structure.len() >= 2 {
        let dup = root.structure[1].id.clone();
        root.structure[0].id = dup;
    }
    let report = validate::validate(&root);
    assert!(!report.valid);
    assert!(report
        .errors
        .iter()
        .any(|e| e.contains("duplicate element id")));
}

#[test]
fn warns_dangling_edge_endpoint() {
    let mut root = expected();
    if let Some(graph) = root.graph.as_mut() {
        let target = graph.nodes[0].id.clone();
        graph.edges.push(okf::types::GraphEdge {
            source: "missing-node".into(),
            target,
            kind: "part".into(),
            label: String::new(),
        });
    }
    let report = validate::validate(&root);
    // Unresolved endpoints are warnings, not errors (ruling B): a real exporter can
    // emit them, so the document stays valid and the defect is reported loudly.
    assert!(report.valid, "unexpected errors: {:?}", report.errors);
    assert!(report.warnings.iter().any(|w| w.contains("missing-node")));
}

#[test]
fn rejects_unknown_edge_kind() {
    let mut root = expected();
    if let Some(graph) = root.graph.as_mut() {
        let target = graph.nodes[0].id.clone();
        graph.edges.push(okf::types::GraphEdge {
            source: "missing-node".into(),
            target,
            kind: "not-a-kind".into(),
            label: String::new(),
        });
    }
    let report = validate::validate(&root);
    assert!(!report.valid);
    assert!(report
        .errors
        .iter()
        .any(|e| e.contains("unknown edge kind")));
}

#[test]
fn rejects_empty_requirement_id() {
    let mut root = expected();
    root.requirements[0].req_id = String::new();
    let report = validate::validate(&root);
    assert!(!report.valid);
    assert!(report.errors.iter().any(|e| e.contains("empty reqId")));
}

#[test]
fn canonical_hash_is_stable() {
    let a = hash::canonical_hash(&expected());
    let b = hash::canonical_hash(&expected());
    assert_eq!(a, b);
    assert_eq!(a.len(), 64);
}

#[test]
fn a_renamed_element_is_reported_as_invisible_to_the_graph() {
    // Renaming an element's id without updating the graph leaves the element in the
    // document and its node under the old name. Nothing else notices: a dangling EDGE
    // endpoint is reported, but an orphaned ELEMENT was not, so the element would quietly
    // disappear from every coverage and traceability answer while still being visible in
    // the structure tree - two views of one model disagreeing, with no warning.
    let mut root = expected();
    root.structure[0].id = "renamed-and-now-orphaned".to_string();
    let report = validate::validate(&root);
    assert!(
        report.valid,
        "an orphaned element is not a validation failure: {:?}",
        report.errors
    );
    assert!(
        report
            .warnings
            .iter()
            .any(|w| w.contains("renamed-and-now-orphaned") && w.contains("no node in the graph")),
        "the orphaned element must be named: {:?}",
        report.warnings
    );
}

#[test]
fn the_corpus_has_no_orphaned_elements() {
    // The reference export has two dangling EDGES, not a missing element: every element it
    // declares is mirrored by a node, which is why the check above is a warning for other
    // models rather than a new failure for this one.
    let report = validate::validate(&expected());
    assert!(
        !report
            .warnings
            .iter()
            .any(|w| w.contains("no node in the graph")),
        "unexpected orphaned elements: {:?}",
        report.warnings
    );
}
#[test]
fn a_complete_reference_validates() {
    let mut root = expected();
    root.references.push(okf::types::SubsystemReference {
        project: "radar".into(),
        revision: "a".repeat(64),
        role: "radar".into(),
        bounds: Vec::new(),
        cross_model_edges: Vec::new(),
    });
    let report = validate::validate(&root);
    assert!(report.valid, "unexpected errors: {:?}", report.errors);
}

#[test]
fn a_reference_without_a_project_is_an_error() {
    let mut root = expected();
    root.references.push(okf::types::SubsystemReference {
        project: String::new(),
        revision: "a".repeat(64),
        role: "radar".into(),
        bounds: Vec::new(),
        cross_model_edges: Vec::new(),
    });
    let report = validate::validate(&root);
    assert!(!report.valid);
    assert!(report.errors.iter().any(|e| e.contains("empty project")));
}

#[test]
fn a_reference_without_a_revision_is_an_error() {
    let mut root = expected();
    root.references.push(okf::types::SubsystemReference {
        project: "radar".into(),
        revision: String::new(),
        role: "radar".into(),
        bounds: Vec::new(),
        cross_model_edges: Vec::new(),
    });
    let report = validate::validate(&root);
    assert!(!report.valid);
    assert!(report.errors.iter().any(|e| e.contains("empty revision")));
}

#[test]
fn a_reference_without_a_role_is_an_error() {
    let mut root = expected();
    root.references.push(okf::types::SubsystemReference {
        project: "radar".into(),
        revision: "a".repeat(64),
        role: String::new(),
        bounds: Vec::new(),
        cross_model_edges: Vec::new(),
    });
    let report = validate::validate(&root);
    assert!(!report.valid);
    assert!(report.errors.iter().any(|e| e.contains("empty role")));
}
