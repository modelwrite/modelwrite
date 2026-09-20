// SPDX-License-Identifier: AGPL-3.0-or-later
//! Unit tests for the SysML v2 textual-notation reader, pinned against the
//! committed fixtures.

use binding::{summarize_import, Binding, BindingError, Direction, MappingVerdict};
use binding_sysmlv2::{model, SysmlV2Binding, BINDING_ID, BINDING_VERSION};

fn fixture(name: &str) -> Vec<u8> {
    let path = format!("{}/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read(&path).unwrap_or_else(|e| panic!("cannot read {path}: {e}"))
}

fn import(name: &str) -> (okf::types::OkfRoot, binding::LossReport) {
    SysmlV2Binding::new()
        .import(&fixture(name))
        .unwrap_or_else(|e| panic!("fixture {name} must import: {e}"))
}

#[test]
fn the_binding_declares_itself_a_viewer() {
    let b = SysmlV2Binding::new();
    let info = b.info();
    assert_eq!(info.id, BINDING_ID);
    assert_eq!(info.version, BINDING_VERSION);
    assert_eq!(info.direction, Direction::ImportOnly);

    // A viewer refuses to export rather than pretend it can round-trip.
    let root = import("structural.sysml").0;
    match b.export(&root) {
        Err(BindingError::Export(m)) => assert!(m.contains("viewer")),
        other => panic!("expected Export error, got {:?}", other),
    }

    // The harness refuses a viewer outright.
    let source = serde_json::to_vec(&import("structural.sysml").0).unwrap();
    match binding::round_trip(&b, &source) {
        Err(BindingError::Viewer { id }) => assert_eq!(id, BINDING_ID),
        other => panic!("expected Viewer error, got {:?}", other),
    }
}

#[test]
fn imports_part_definitions_and_features() {
    let (root, report) = import("structural.sysml");
    let summary = summarize_import(&root, &report);

    assert_eq!(root.project, "Demo");
    assert_eq!(root.structure.len(), 4);
    assert_eq!(summary.elements_imported, 4);
    assert_eq!(summary.content_losses, 0);
    assert_eq!(summary.verdict, binding::ImportVerdict::Lossless);

    let car = root
        .structure
        .iter()
        .find(|e| e.name == "Car")
        .expect("Car");
    assert_eq!(car.id, "Demo::Car");
    assert_eq!(car.kind, "block");
    assert_eq!(car.stereotypes, vec!["partDef".to_string()]);
    assert_eq!(car.attributes.len(), 2);
    let eng = car
        .attributes
        .iter()
        .find(|a| a.name == "eng")
        .expect("eng attribute");
    assert_eq!(eng.attr_type, "Engine");
    assert_eq!(eng.aggregation, "composite");
    let speed = car
        .attributes
        .iter()
        .find(|a| a.name == "speed")
        .expect("speed attribute");
    assert_eq!(speed.default, "0");

    // The attribute def carries the keyword as its stereotype.
    let color = root
        .structure
        .iter()
        .find(|e| e.name == "Color")
        .expect("Color");
    assert_eq!(color.stereotypes, vec!["attributeDef".to_string()]);

    // The specialization becomes a generalization edge; the part becomes a part
    // edge (both endpoints resolve to carried blocks).
    let graph = root.graph.as_ref().expect("graph");
    assert_eq!(graph.nodes.len(), 4);
    let kinds: std::collections::HashSet<&str> =
        graph.edges.iter().map(|e| e.kind.as_str()).collect();
    assert!(kinds.contains("part"));
    assert!(kinds.contains("generalization"));
    let part_edge = graph
        .edges
        .iter()
        .find(|e| e.kind == "part")
        .expect("part edge");
    assert_eq!(part_edge.source, "Demo::Car");
    assert_eq!(part_edge.target, "Demo::Engine");
    let gen_edge = graph
        .edges
        .iter()
        .find(|e| e.kind == "generalization")
        .expect("generalization edge");
    assert_eq!(gen_edge.source, "Demo::SportsCar");
    assert_eq!(gen_edge.target, "Demo::Car");

    // The graph node ids mirror the structure ids, so OKF validation holds.
    let node_ids: std::collections::HashSet<&str> =
        graph.nodes.iter().map(|n| n.id.as_str()).collect();
    for e in &root.structure {
        assert!(node_ids.contains(e.id.as_str()), "{} has no node", e.id);
    }
}

#[test]
fn imports_a_requirement_with_subject_and_satisfy() {
    let (root, report) = import("requirement.sysml");
    let summary = summarize_import(&root, &report);

    assert_eq!(root.structure.len(), 1);
    assert_eq!(root.requirements.len(), 1);
    assert_eq!(summary.content_losses, 0);

    let req = &root.requirements[0];
    assert_eq!(req.id, "Demo::range");
    assert_eq!(req.req_id, "REQ-1");
    assert_eq!(req.kind, "requirement");
    assert!(
        req.documentation.contains("fly far"),
        "doc comment carried, got: {}",
        req.documentation
    );

    let graph = root.graph.as_ref().expect("graph");
    let subject = graph
        .edges
        .iter()
        .find(|e| e.kind == "subject")
        .expect("subject edge");
    assert_eq!(subject.source, "Demo::range");
    assert_eq!(subject.target, "Demo::Drone");
    assert_eq!(subject.label, "drone");

    let satisfy = graph
        .edges
        .iter()
        .find(|e| e.kind == "dependency")
        .expect("satisfy edge");
    assert_eq!(satisfy.source, "Demo::Drone");
    assert_eq!(satisfy.target, "Demo::range");
    assert_eq!(satisfy.label, "satisfy");
}

#[test]
fn out_of_scope_constructs_are_named_losses_never_a_panic() {
    let (root, report) = import("out-of-scope.sysml");

    // No model content is carried: every construct is outside the subset.
    assert_eq!(root.structure.len(), 0);
    assert_eq!(root.requirements.len(), 0);

    let losses = report.content_losses();
    let subjects: Vec<&str> = losses.iter().map(|m| m.subject.as_str()).collect();
    println!("named losses: {subjects:?}");

    // Each distinct out-of-scope construct is named, not merged into one.
    assert!(losses.iter().any(|m| m.subject.starts_with("calc")));
    assert!(losses.iter().any(|m| m.subject.starts_with("state")));
    assert!(losses.iter().any(|m| m.subject.starts_with("actor")));
    assert!(losses.iter().any(|m| m.subject.starts_with("usecase")));
    assert!(losses.iter().any(|m| m.subject.starts_with("association")));
    assert!(losses
        .iter()
        .all(|m| m.verdict == MappingVerdict::Unmappable));
}

#[test]
fn a_malformed_document_is_a_clean_typed_error() {
    let result = SysmlV2Binding::new().import(&fixture("malformed.sysml"));
    match result {
        Err(BindingError::Import(m)) => assert!(m.contains("unterminated block comment"), "{m}"),
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
    // The positive set is declared: definitions and requirements are Exact.
    assert!(table
        .iter()
        .any(|m| m.verdict == MappingVerdict::Exact && m.subject.contains("requirement")));
    assert!(table
        .iter()
        .any(|m| m.verdict == MappingVerdict::Exact && m.subject.contains("definition")));
}

#[test]
fn an_empty_document_is_a_clean_error() {
    match SysmlV2Binding::new().import(
        b"// just a comment
",
    ) {
        Err(BindingError::Import(m)) => assert!(m.contains("no package"), "{m}"),
        other => panic!("expected Import error, got {:?}", other),
    }
}

#[test]
fn non_utf8_source_is_a_clean_error() {
    match SysmlV2Binding::new().import(&[0xff, 0xfe, 0xfd]) {
        Err(BindingError::Import(m)) => assert!(m.contains("UTF-8"), "{m}"),
        other => panic!("expected Import error, got {:?}", other),
    }
}
