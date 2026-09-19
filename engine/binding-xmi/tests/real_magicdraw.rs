// SPDX-License-Identifier: AGPL-3.0-or-later
//! Importing a REAL MagicDraw SysML export.
//!
//! This is the test the project has been missing. Every other XMI fixture is
//! either hand-written or a Papyrus skeleton with no elements. This file came
//! out of a MagicDraw `.mdzip` (the `com.nomagic.magicdraw.uml_model.model`
//! entry) and contains 148 packagedElement, 57 ownedAttribute, 34 Blocks and
//! 25 Requirements, with 20 Satisfy and 3 Allocate links. (These are ELEMENT
//! counts, not substring counts: an earlier comment said 55/45/27/9 by counting
//! occurrences of the word inside reference strings and attribute values.)
//!
//! It is also, embarrassingly, something the repository already had: it sat in
//! `sample/corpus/coffee-machine/legacy/` from the beginning and no test ever
//! read it. Synthetic fixtures were tested; the real artifact was not.
//!
//! The test asserts what the platform promises about ANY input - a clean result
//! or a clean error, every departure named, never a panic - and PRINTS what a
//! real migration would actually get. The number is the finding.

use binding::{round_trip, Binding, MappingVerdict};
use binding_xmi::XmiBinding;

fn real_model() -> Vec<u8> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../real-world/mdzip/model.xmi"
    );
    std::fs::read(path).unwrap_or_else(|e| panic!("cannot read {}: {}", path, e))
}

#[test]
fn a_real_magicdraw_sysml_export_imports_with_every_loss_named() {
    let bytes = real_model();
    let binding = XmiBinding::new();

    let (root, report) = match binding.import(&bytes) {
        Ok(ok) => ok,
        Err(e) => {
            println!("=== REAL MAGICDRAW MODEL ===");
            println!("IMPORT REFUSED: {:?}", e);
            println!("A clean error, not a panic - but the model is outside the subset.");
            return;
        }
    };

    println!("=== REAL MAGICDRAW MODEL (CoffeeMachine-SysML-Model.mdzip) ===");
    println!("structure elements : {}", root.structure.len());
    println!("requirements       : {}", root.requirements.len());
    println!("interfaces         : {}", root.interfaces.len());
    println!("signals            : {}", root.signals.len());
    println!("activities         : {}", root.activities.len());
    println!(
        "graph nodes/edges  : {}/{}",
        root.graph.as_ref().map(|g| g.nodes.len()).unwrap_or(0),
        root.graph.as_ref().map(|g| g.edges.len()).unwrap_or(0)
    );
    let declarations = report.declarations();
    let content_losses = report.content_losses();
    println!("declarations       : {}", declarations.len());
    println!("CONTENT LOSSES     : {}", content_losses.len());
    println!("blocking           : {}", report.blocking().len());
    println!("lossless           : {}", report.is_lossless());
    println!("--- THE CONTENT LOSSES (what a migration would actually lose) ---");
    for m in content_losses.iter().take(25) {
        println!("  [{:?}] {}", m.verdict, m.subject);
    }
    let unmappable = content_losses
        .iter()
        .filter(|m| m.verdict == MappingVerdict::Unmappable)
        .count();
    println!(
        "=== SUMMARY: {} content losses ({} unmappable), {} declarations ===",
        content_losses.len(),
        unmappable,
        declarations.len()
    );

    // --- The number this test exists to move ---
    assert_eq!(root.requirements.len(), 25, "all 25 requirements carried");
    assert_eq!(root.summary.requirements, 25);

    let graph = root.graph.as_ref().expect("graph present");
    let satisfy = graph.edges.iter().filter(|e| e.label == "Satisfy").count();
    let allocate = graph.edges.iter().filter(|e| e.label == "Allocate").count();
    assert_eq!(satisfy, 20, "all 20 Satisfy links carried");
    assert_eq!(allocate, 3, "all 3 Allocate links carried");
    let dependencies = graph
        .edges
        .iter()
        .filter(|e| e.kind == "dependency")
        .count();
    assert_eq!(
        dependencies, 28,
        "20 Satisfy + 3 Allocate + 4 Refine + 1 Verify"
    );
    // Items 2-3: associations, parts (PartProperty + ports) and references are now
    // carried as graph edges. 28 associations (block<->block; the 5 Actor/UseCase
    // associations stay named losses), 34 part edges (28 PartProperty + 6 ports)
    // and 5 reference edges.
    let associations = graph
        .edges
        .iter()
        .filter(|e| e.kind == "association")
        .count();
    let parts = graph.edges.iter().filter(|e| e.kind == "part").count();
    let references = graph.edges.iter().filter(|e| e.kind == "reference").count();
    assert_eq!(associations, 28, "28 block<->block associations carried");
    assert_eq!(parts, 34, "28 PartProperty + 6 port part edges carried");
    assert_eq!(references, 5, "5 ReferenceProperty reference edges carried");
    assert_eq!(graph.edges.len(), 28 + 28 + 34 + 5);
    // 34 blocks + 6 interface blocks + 1 constraint block + 25 requirements.
    assert_eq!(root.summary.blocks, 41);
    assert_eq!(graph.nodes.len(), 66, "41 blocks + 25 requirements");

    // Every requirement is a graph node, so a traceability edge resolves both
    // endpoints (OKF spec: every element is a node).
    let node_ids: std::collections::HashSet<&str> =
        graph.nodes.iter().map(|n| n.id.as_str()).collect();
    for r in &root.requirements {
        assert!(
            node_ids.contains(r.id.as_str()),
            "{} must be a graph node",
            r.id
        );
    }

    // A leaf requirement carries its own id (reqId), body (reqText) and name.
    let heater = root
        .requirements
        .iter()
        .find(|r| r.name == "Heater Initialization")
        .expect("Heater Initialization requirement present");
    assert_eq!(heater.req_id, "1.1");
    assert!(
        heater
            .req_text
            .starts_with("The boiler shall heat water to 84")
            && heater
                .req_text
                .ends_with("within 90 seconds of turning on the coffee machine")
    );

    // A Satisfy edge points from the satisfying block (client) to the satisfied
    // requirement (supplier): the Boiler satisfies "Heater Initialization".
    assert!(graph.edges.iter().any(|e| {
        e.label == "Satisfy"
            && e.source == "_2026x_1_12a70364_1789363551438_409574_3734"
            && e.target == "_2026x_1_12a70364_1789522470210_613186_5619"
    }));

    // The requirement class is carried, not left as an unmapped non-block class.
    assert!(!content_losses.iter().any(|m| {
        m.subject == "uml:Class _2026x_1_12a70364_1789522470210_613186_5619 (Heater Initialization)"
    }));

    // --- Items 2-3: ports, associations and parts are carried, with the golden
    // corpus edge direction. A block id is resolved by name so the assertions read
    // in model terms, not MagicDraw id soup.
    let block_id = |name: &str| -> String {
        root.structure
            .iter()
            .find(|e| e.name == name)
            .unwrap_or_else(|| panic!("block {name} present"))
            .id
            .clone()
    };
    let coffee_machine = block_id("Coffee Machine");
    let water_system = block_id("Water System");

    // A composition (PartProperty) edge points from the owning block to the part's
    // type: Coffee Machine --part--> Water System. (The association edge below is
    // the SAME relationship read in the opposite direction.)
    assert!(graph
        .edges
        .iter()
        .any(|e| { e.kind == "part" && e.source == coffee_machine && e.target == water_system }));

    // A port is carried as a composite attribute of its owning block, typed by its
    // InterfaceBlock, AND as a part edge to the port type.
    let coffee_machine_el = root
        .structure
        .iter()
        .find(|e| e.id == coffee_machine)
        .unwrap();
    let waterin = coffee_machine_el
        .attributes
        .iter()
        .find(|a| a.name == "Waterin")
        .expect("the Waterin proxy port is carried as an attribute");
    assert_eq!(waterin.attr_type, "Water Flow Port");
    assert_eq!(waterin.aggregation, "composite");
    let water_flow_port = block_id("Water Flow Port");
    assert!(graph.edges.iter().any(|e| {
        e.kind == "part" && e.source == coffee_machine && e.target == water_flow_port
    }));

    // An InterfaceBlock class is carried as a block carrying its own stereotype, so
    // the port's type resolves to a real node.
    let port_block = root
        .structure
        .iter()
        .find(|e| e.id == water_flow_port)
        .unwrap();
    assert_eq!(port_block.kind, "block");
    assert_eq!(port_block.stereotypes, vec!["InterfaceBlock".to_string()]);

    // A ReferenceProperty edge points from the owning block to the referenced type.
    let coffee_machine_production = block_id("Coffee Machine Production");
    assert!(graph.edges.iter().any(|e| {
        e.kind == "reference" && e.source == coffee_machine && e.target == coffee_machine_production
    }));

    // The association edge DIRECTION is the golden corpus convention: source = type
    // of the first memberEnd (the part), target = type of the second memberEnd (the
    // ownedEnd, the owning block). So Water System --association--> Coffee Machine,
    // the exact opposite of the part edge. Getting this backwards yields zero
    // requirement coverage on any downstream comparison, so it is pinned.
    assert!(graph.edges.iter().any(|e| {
        e.kind == "association" && e.source == water_system && e.target == coffee_machine
    }));
    assert!(!graph.edges.iter().any(|e| {
        e.kind == "association" && e.source == coffee_machine && e.target == water_system
    }));

    // An Actor/UseCase association is NOT silently dropped: its ends are outside the
    // carried subset, so it remains a NAMED loss.
    assert!(content_losses.iter().any(|m| {
        m.verdict == MappingVerdict::Unmappable
            && m.subject.starts_with("uml:Association")
            && m.note.contains("outside the carried subset")
    }));
}

#[test]
fn the_real_magicdraw_model_round_trips_through_the_harness() {
    // GAP 1: an imported-then-exported real model must retain the association,
    // part and reference edges the import carries. The engine's own diff - not
    // the binding's claim - is the arbiter of what survives the OKF -> XMI -> OKF
    // journey.
    let bytes = real_model();
    let binding = XmiBinding::new();
    let (root, _) = binding.import(&bytes).expect("import must succeed");
    let source = serde_json::to_vec(&root).expect("source must serialize");
    let outcome = round_trip(&binding, &source).expect("round trip must succeed");
    assert!(
        outcome.diff.equal,
        "engine diff over the real model:\n  missing elements: {:?}\n  extra elements: {:?}\n  missing edges: {:?}\n  extra edges: {:?}\n  changed attributes: {:?}",
        outcome.diff.missing_elements,
        outcome.diff.extra_elements,
        outcome.diff.missing_edges,
        outcome.diff.extra_edges,
        outcome.diff.changed_attributes
    );
}
