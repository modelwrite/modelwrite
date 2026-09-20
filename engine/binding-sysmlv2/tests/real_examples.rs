// SPDX-License-Identifier: AGPL-3.0-or-later
//! The FIRST import of REAL SysML v2 textual models from the bundled corpus.
//!
//! These are not hand-written: they are the two smallest GfSE community models
//! committed at sample/examples/sysml-v2/. They answer the question a synthetic
//! fixture cannot - what does the reader actually do with a model somebody else
//! wrote - and they are the measurement discipline: numbers, not adjectives.

use binding::{summarize_import, Binding, MappingVerdict};
use binding_sysmlv2::SysmlV2Binding;

fn real(name: &str) -> Vec<u8> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../sample/examples/sysml-v2/gfse-models/models/SE_Models/",
    );
    let path = format!("{path}{name}");
    std::fs::read(&path).unwrap_or_else(|e| panic!("cannot read {path}: {e}"))
}

fn import(name: &str) -> (okf::types::OkfRoot, binding::LossReport) {
    SysmlV2Binding::new()
        .import(&real(name))
        .unwrap_or_else(|e| panic!("real model {name} must import: {e}"))
}

/// Print the full measurement and named losses, so the test output is the
/// numbers a human reads, not adjectives.
fn dump(name: &str, root: &okf::types::OkfRoot, report: &binding::LossReport) {
    let summary = summarize_import(root, report);
    let graph = root.graph.as_ref();
    println!("=== {name} ===");
    println!("  blocks (definitions)     : {}", root.structure.len());
    println!("  requirements             : {}", root.requirements.len());
    println!(
        "  graph nodes              : {}",
        graph.map(|g| g.nodes.len()).unwrap_or(0)
    );
    println!(
        "  graph edges              : {}",
        graph.map(|g| g.edges.len()).unwrap_or(0)
    );
    println!(
        "  declarations recognised  : {}",
        report.declarations().len()
    );
    println!(
        "  content losses           : {}",
        report.content_losses().len()
    );
    println!("  lossy drops              : {}", report.lossy().len());
    println!("  {}", summary.statement());
    for m in &report.mappings {
        if m.verdict != MappingVerdict::Exact {
            println!("  [{:?}] {} :: {}", m.verdict, m.subject, m.note);
        }
    }
}

#[test]
fn the_drone_model_imports_with_counts_and_named_losses() {
    let (root, report) = import("Drone_BaseArchitecture.sysml");
    dump("Drone_BaseArchitecture.sysml", &root, &report);

    // The four requirements are the meat of this model and all survive.
    assert_eq!(root.requirements.len(), 4);
    assert_eq!(
        root.structure.len(),
        1,
        "the Drone part def is the one block"
    );
    assert!(root
        .requirements
        .iter()
        .any(|r| r.req_id == "REQ-42" && r.name == "longDistance"));

    // The long-distance requirement's subject resolves to the carried Drone
    // block, so a subject edge exists.
    let graph = root.graph.as_ref().expect("graph");
    assert!(graph
        .edges
        .iter()
        .any(|e| e.kind == "subject" && e.target == "Drone_BaseArchitecture::Drone"));

    // The things the subset cannot carry are NAMED, not silent: the top-level
    // part usage, the satisfy whose subject is that usage, the :>> subjects,
    // and the #derivation extension.
    let subjects: Vec<&str> = report
        .content_losses()
        .iter()
        .map(|m| m.subject.as_str())
        .collect();
    assert!(
        subjects.iter().any(|s| s.contains("derivation")),
        "the #derivation extension is named, got {subjects:?}"
    );
    assert!(
        report
            .content_losses()
            .iter()
            .any(|m| m.subject.contains("part drone")),
        "the top-level part usage is named"
    );
    // The import is a recognised declaration, not a content loss.
    assert!(report
        .declarations()
        .iter()
        .any(|m| m.subject.starts_with("import ")));
}

#[test]
fn the_internet_model_imports_with_counts() {
    let (root, report) = import("InternetModel_v1.sysml");
    dump("InternetModel_v1.sysml", &root, &report);

    // 5 part defs (Data, Device, WiFiRouter, DSLRouter, MobileDevice) and 4
    // attribute defs (Connection, WirelessConnection, CableConnection,
    // Electricity) - every definition is carried.
    assert_eq!(root.structure.len(), 9);
    assert_eq!(root.requirements.len(), 0);

    // Specialization survives as generalization edges: 3 devices :> Device and
    // 2 connections :> Connection.
    let graph = root.graph.as_ref().expect("graph");
    let gen_count = graph
        .edges
        .iter()
        .filter(|e| e.kind == "generalization")
        .count();
    assert_eq!(gen_count, 5);

    // The doc comment on MobileDevice is carried.
    let mobile = root
        .structure
        .iter()
        .find(|e| e.name == "MobileDevice")
        .expect("MobileDevice");
    assert!(mobile.documentation.contains("Bluetooth"));

    // The directional items (in item / out item) are carried as attributes and
    // the direction drop is NAMED, not silent.
    let conn = root
        .structure
        .iter()
        .find(|e| e.name == "Connection")
        .expect("Connection");
    assert_eq!(conn.attributes.len(), 2);
    assert!(report
        .lossy()
        .iter()
        .any(|m| m.subject.contains("inData") || m.subject.contains("outData")));
}
