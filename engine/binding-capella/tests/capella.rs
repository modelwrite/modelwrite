// SPDX-License-Identifier: AGPL-3.0-or-later
//! Unit tests for the Capella/Arcadia reader, pinned against the committed
//! hand-written fixtures. These fixtures mirror the real .capella format
//! (verified against the fetched corpus); they prove nothing about Capella's
//! exporter, only that the reader holds the boundary and names every loss.

use binding::{summarize_import, Binding, BindingError, Direction, MappingVerdict};
use binding_capella::{model, CapellaBinding, BINDING_ID, BINDING_VERSION};

fn fixture(name: &str) -> Vec<u8> {
    let path = format!("{}/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read(&path).unwrap_or_else(|e| panic!("cannot read {path}: {e}"))
}

fn import(name: &str) -> (okf::types::OkfRoot, binding::LossReport) {
    CapellaBinding::new()
        .import(&fixture(name))
        .unwrap_or_else(|e| panic!("fixture {name} must import: {e}"))
}

#[test]
fn the_binding_declares_itself_a_viewer() {
    let b = CapellaBinding::new();
    let info = b.info();
    assert_eq!(info.id, BINDING_ID);
    assert_eq!(info.version, BINDING_VERSION);
    assert_eq!(info.direction, Direction::ImportOnly);

    // A viewer refuses to export rather than pretend it can round-trip.
    let root = import("control-structure.capella").0;
    match b.export(&root) {
        Err(BindingError::Export(m)) => assert!(m.contains("viewer")),
        other => panic!("expected Export error, got {:?}", other),
    }

    // The harness refuses a viewer outright.
    let source = serde_json::to_vec(&import("control-structure.capella").0).unwrap();
    match binding::round_trip(&b, &source) {
        Err(BindingError::Viewer { id }) => assert_eq!(id, BINDING_ID),
        other => panic!("expected Viewer error, got {:?}", other),
    }
}

#[test]
fn imports_the_control_structure() {
    let (root, report) = import("control-structure.capella");
    let summary = summarize_import(&root, &report);

    // 3 functions (Issue, Trigger, Report) + 2 components (Controller, Display).
    assert_eq!(root.project, "IFE Control Structure");
    assert_eq!(root.structure.len(), 5);
    assert_eq!(root.requirements.len(), 1);
    assert_eq!(summary.elements_imported, 5);
    assert_eq!(summary.content_losses, 0);
    assert_eq!(summary.verdict, binding::ImportVerdict::Lossless);

    // Components carry their Capella type as the stereotype.
    let controller = root
        .structure
        .iter()
        .find(|e| e.name == "Cabin Crew Controller")
        .expect("controller");
    assert_eq!(controller.kind, "block");
    assert_eq!(controller.stereotypes, vec!["SystemComponent".to_string()]);

    // Functions carry their Capella type as the stereotype, and their ports as
    // attributes.
    let issue = root
        .structure
        .iter()
        .find(|e| e.name == "Issue Safety Instruction")
        .expect("issue function");
    assert_eq!(issue.stereotypes, vec!["SystemFunction".to_string()]);
    assert_eq!(issue.attributes.len(), 1);
    assert_eq!(issue.attributes[0].name, "FOP1");
    assert_eq!(issue.attributes[0].attr_type, "FunctionOutputPort");

    // The constraint's expression body is carried as the requirement text.
    let req = &root.requirements[0];
    assert_eq!(req.name, "SC-1");
    assert_eq!(req.stereotypes, vec!["constraint".to_string()]);
    assert!(req.req_text.contains("safety instruction"));

    let graph = root.graph.as_ref().expect("graph");
    assert_eq!(graph.nodes.len(), 6, "5 blocks + 1 requirement");

    // The control action and its feedback are two directed dependency edges.
    let deps: Vec<&okf::types::GraphEdge> = graph
        .edges
        .iter()
        .filter(|e| e.kind == "dependency")
        .collect();
    assert_eq!(deps.len(), 2);
    let control = deps
        .iter()
        .find(|e| e.label == "Safety Instruction")
        .expect("control flow");
    assert_eq!(control.source, "f-issue");
    assert_eq!(control.target, "f-report");

    // The allocations are 'part' edges from controller to control function.
    let allocs: Vec<&okf::types::GraphEdge> =
        graph.edges.iter().filter(|e| e.kind == "part").collect();
    assert_eq!(allocs.len(), 2);
    assert!(allocs
        .iter()
        .any(|e| e.source == "c-controller" && e.target == "f-issue" && e.label == "allocated"));

    // Function nesting survives as a 'contains' edge.
    let contains: Vec<&okf::types::GraphEdge> = graph
        .edges
        .iter()
        .filter(|e| e.kind == "contains")
        .collect();
    assert_eq!(contains.len(), 1);
    assert_eq!(contains[0].source, "f-issue");
    assert_eq!(contains[0].target, "f-trigger");

    // Every carried block has a graph node, so OKF validation holds.
    let node_ids: std::collections::HashSet<&str> =
        graph.nodes.iter().map(|n| n.id.as_str()).collect();
    for e in &root.structure {
        assert!(node_ids.contains(e.id.as_str()), "{} has no node", e.id);
    }

    // The flattening and dropped ids are NAMED: 2 packages, 2 port ids, 4 edge
    // ids (2 exchanges + 2 allocations) = 8 lossy drops, no content lost.
    assert_eq!(report.lossy().len(), 8);
    assert_eq!(report.content_losses().len(), 0);
    let lossy_subjects: Vec<&str> = report.lossy().iter().map(|m| m.subject.as_str()).collect();
    assert!(
        lossy_subjects
            .iter()
            .any(|s| s.starts_with("SystemFunctionPkg")),
        "package flattening named, got {lossy_subjects:?}"
    );
    assert!(
        lossy_subjects
            .iter()
            .any(|s| s.starts_with("FunctionalExchange fx-")),
        "exchange id drop named"
    );
    assert!(
        lossy_subjects
            .iter()
            .any(|s| s.starts_with("FunctionOutputPort p-issue-out")),
        "port id drop named"
    );

    // The layer roots and the consumed expression are declarations, not losses.
    let decls: Vec<&str> = report
        .declarations()
        .iter()
        .map(|m| m.subject.as_str())
        .collect();
    assert!(
        decls.iter().any(|s| s.starts_with("SystemEngineering")),
        "{decls:?}"
    );
    assert!(
        decls.iter().any(|s| s.starts_with("SystemAnalysis")),
        "{decls:?}"
    );
    assert!(
        decls.iter().any(|s| s.starts_with("OpaqueExpression")),
        "{decls:?}"
    );
}

#[test]
fn out_of_scope_constructs_are_named_losses_never_a_panic() {
    let (root, report) = import("out-of-scope.capella");

    // The carried function and component still import.
    assert_eq!(root.structure.len(), 2);

    let losses = report.content_losses();
    let subjects: Vec<&str> = losses.iter().map(|m| m.subject.as_str()).collect();
    println!("named losses: {subjects:?}");

    assert!(losses.iter().any(|m| m.subject.starts_with("TransfoLink")));
    assert!(losses.iter().any(|m| m.subject.starts_with("StateMachine")));
    assert!(losses.iter().any(|m| m.subject.starts_with("Part")));
    assert!(losses
        .iter()
        .all(|m| m.verdict == MappingVerdict::Unmappable));
}

#[test]
fn a_malformed_document_is_a_clean_typed_error() {
    let result = CapellaBinding::new().import(&fixture("malformed.capella"));
    match result {
        Err(BindingError::Import(m)) => assert!(m.contains("malformed XML"), "{m}"),
        other => panic!("expected Import error, got {:?}", other),
    }
}

#[test]
fn an_aird_representation_is_rejected_with_a_clear_message() {
    let result = CapellaBinding::new().import(&fixture("representation.aird"));
    match result {
        Err(BindingError::Import(m)) => {
            assert!(m.contains("not a Capella semantic model"), "{m}");
            assert!(m.contains(".aird"), "{m}");
        }
        other => panic!("expected Import error, got {:?}", other),
    }
}

#[test]
fn non_utf8_source_is_a_clean_error() {
    match CapellaBinding::new().import(&[0xff, 0xfe, 0xfd]) {
        Err(BindingError::Import(m)) => assert!(m.contains("UTF-8"), "{m}"),
        other => panic!("expected Import error, got {:?}", other),
    }
}

#[test]
fn a_project_without_a_name_is_a_clean_error() {
    let doc = r#"<?xml version="1.0" encoding="UTF-8"?>
<org.polarsys.capella.core.data.capellamodeller:Project xmi:version="2.0" xmlns:xmi="http://www.omg.org/XMI"
    xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
    xmlns:org.polarsys.capella.core.data.capellamodeller="http://www.polarsys.org/capella/core/modeller/7.0.0"
    id="no-name-project"/>
"#;
    match CapellaBinding::new().import(doc.as_bytes()) {
        Err(BindingError::Import(m)) => assert!(m.contains("no name"), "{m}"),
        other => panic!("expected Import error, got {:?}", other),
    }
}

#[test]
fn the_mapping_table_declares_the_subset_as_data() {
    let table = model::mapping_table();
    assert!(!table.is_empty());
    // The negative set is declared: an Unmappable catch-all for anything else.
    assert!(table
        .iter()
        .any(|m| m.verdict == MappingVerdict::Unmappable));
    // The positive set is declared: components, functions and constraints are Exact.
    assert!(table
        .iter()
        .any(|m| m.verdict == MappingVerdict::Exact && m.subject.contains("SystemComponent")));
    assert!(table
        .iter()
        .any(|m| m.verdict == MappingVerdict::Exact && m.subject.contains("Constraint")));
    // The flows and allocations carry a directional relationship but drop ids.
    assert!(table
        .iter()
        .any(|m| m.verdict == MappingVerdict::Lossy && m.subject.contains("FunctionalExchange")));
}
