// SPDX-License-Identifier: AGPL-3.0-or-later
//! The hand-checked acceptance for the SysML v1 XMI reader.

use binding::{Binding, BindingError, Direction, MappingVerdict};
use binding_xmi::{model, XmiBinding, BINDING_ID, BINDING_VERSION};
use okf::types::{GraphEdge, OkfRoot, StateMachine};

fn fixture(name: &str) -> String {
    let path = format!("{}/fixtures/{}", env!("CARGO_MANIFEST_DIR"), name);
    std::fs::read_to_string(path).expect("fixture must exist")
}

fn import(name: &str) -> (OkfRoot, binding::LossReport) {
    let bytes = fixture(name);
    XmiBinding::new()
        .import(bytes.as_bytes())
        .expect("import must succeed")
}

fn has(mappings: &[binding::Mapping], verdict: MappingVerdict, subject: &str) -> bool {
    mappings
        .iter()
        .any(|m| m.verdict == verdict && m.subject == subject)
}

#[test]
fn the_binding_declares_its_identity_direction_and_subset() {
    let binding = XmiBinding::new();
    let info = binding.info();
    assert_eq!(info.id, BINDING_ID);
    assert_eq!(info.id, "sysml-v1-xmi");
    assert_eq!(info.version, BINDING_VERSION);
    assert_eq!(info.version, "2.4");
    assert_eq!(info.direction, Direction::ImportAndExport);

    // The subset is data, not prose: the mapping table names each construct.
    let table = binding.mapping_table();
    assert_eq!(table, model::mapping_table());
    assert!(table.iter().any(|m| m.subject.contains("Block stereotype")));
    assert!(table
        .iter()
        .any(|m| m.subject.contains("Satisfy/Allocate/Refine/Verify")));

    // CRITICAL 2: the xmi:id row no longer claims a blanket Exact. It states the
    // truth per construct: Exact only where OKF has a slot (block element and
    // graph node), Lossy everywhere else.
    assert!(table.iter().any(|m| {
        m.subject == "xmi:id on a block (element and graph node)"
            && m.verdict == MappingVerdict::Exact
    }));
    assert!(table.iter().any(|m| {
        m.subject.contains("xmi:id on uml:Model") && m.verdict == MappingVerdict::Lossy
    }));
    assert!(!table
        .iter()
        .any(|m| m.subject == "xmi:id" && m.verdict == MappingVerdict::Exact));

    // M3: the negative set is stated, so the boundary is visible from the table.
    assert!(table.iter().any(|m| {
        m.verdict == MappingVerdict::Unmappable && m.subject.contains("Any other XMI element")
    }));
}

#[test]
fn the_fixture_imports_to_a_hand_checked_document() {
    let (root, loss) = import("coffee-grinder.xmi");

    assert_eq!(root.okf, "1.0");
    assert_eq!(root.project, "Coffee Grinder");
    assert_eq!(root.structure.len(), 2);

    // The first block: a composite property resolved to the Motor block, a
    // primitive property with a default, and a comment.
    let grinder = &root.structure[0];
    assert_eq!(grinder.id, "block-grinder");
    assert_eq!(grinder.name, "Grinder");
    assert_eq!(grinder.kind, "block");
    assert_eq!(grinder.stereotypes, vec!["Block".to_string()]);
    assert_eq!(
        grinder.documentation,
        "Grinds coffee beans to a selected size."
    );
    assert_eq!(grinder.attributes.len(), 2);
    assert_eq!(grinder.attributes[0].name, "motor");
    assert_eq!(grinder.attributes[0].attr_type, "Motor");
    assert_eq!(grinder.attributes[0].aggregation, "composite");
    assert_eq!(grinder.attributes[0].default, "");
    assert_eq!(grinder.attributes[1].name, "capacity");
    assert_eq!(grinder.attributes[1].attr_type, "Integer");
    assert_eq!(grinder.attributes[1].aggregation, "none");
    assert_eq!(grinder.attributes[1].default, "1");

    // The second block: no properties, no documentation.
    let motor = &root.structure[1];
    assert_eq!(motor.id, "block-motor");
    assert_eq!(motor.name, "Motor");
    assert_eq!(motor.kind, "block");
    assert!(motor.attributes.is_empty());
    assert_eq!(motor.documentation, "");

    // The graph mirrors the blocks and carries the Satisfy dependency edge.
    let graph = root.graph.as_ref().expect("graph present");
    assert_eq!(graph.nodes.len(), 2);
    assert_eq!(graph.nodes[0].id, "block-grinder");
    assert_eq!(graph.nodes[1].id, "block-motor");
    assert_eq!(
        graph.edges,
        vec![GraphEdge {
            source: "block-grinder".to_string(),
            target: "block-motor".to_string(),
            kind: "dependency".to_string(),
            label: "Satisfy".to_string(),
        }]
    );

    // Summary counts match the hand-checked structure.
    assert_eq!(root.summary.blocks, 2);
    assert_eq!(root.summary.graph_nodes, 2);
    assert_eq!(root.summary.graph_edges, 1);

    // CRITICAL 2: every dropped id is named as a Lossy entry, plus the package
    // flattening - nothing is silent. (Model, Comment, two Property ids, the
    // Dependency id, and the Package.)
    assert_eq!(loss.mappings.len(), 6);
    assert!(loss
        .mappings
        .iter()
        .all(|m| m.verdict == MappingVerdict::Lossy));
    assert!(has(
        &loss.mappings,
        MappingVerdict::Lossy,
        "uml:Model model-grinder"
    ));
    assert!(has(
        &loss.mappings,
        MappingVerdict::Lossy,
        "uml:Comment doc-grinder"
    ));
    assert!(has(
        &loss.mappings,
        MappingVerdict::Lossy,
        "uml:Property prop-motor"
    ));
    assert!(has(
        &loss.mappings,
        MappingVerdict::Lossy,
        "uml:Property prop-capacity"
    ));
    assert!(has(
        &loss.mappings,
        MappingVerdict::Lossy,
        "uml:Dependency dep-satisfy"
    ));
    assert!(has(
        &loss.mappings,
        MappingVerdict::Lossy,
        "uml:Package pkg-structure (Structure)"
    ));
    assert!(!loss.is_lossless());
}

#[test]
fn an_unknown_element_and_attribute_are_named_not_dropped() {
    let (root, loss) = import("unknown-element.xmi");

    // The known block still imports.
    assert_eq!(root.structure.len(), 1);
    assert_eq!(root.structure[0].id, "block-1");
    assert_eq!(root.structure[0].name, "Known Block");

    // The unknown element is named with its xmi:id.
    assert!(loss.mappings.iter().any(|m| {
        m.verdict == MappingVerdict::Unmappable && m.subject == "uml:StateMachine sm-1"
    }));

    // The unmapped attribute is named on its element.
    assert!(loss.mappings.iter().any(|m| {
        m.verdict == MappingVerdict::Unmappable
            && m.subject == "uml:Class block-1 attribute 'visibility'"
    }));

    // CRITICAL 2: the Model id is also named, not dropped silently.
    assert!(has(
        &loss.mappings,
        MappingVerdict::Lossy,
        "uml:Model model-unknown"
    ));

    // Exactly three losses: the unknown element, the unmapped attribute, and the
    // dropped Model id.
    assert_eq!(loss.mappings.len(), 3);
    assert!(!loss.is_lossless());
}

#[test]
fn a_malformed_document_is_a_clean_error_not_a_panic() {
    let bytes = fixture("malformed.xmi");
    let result = XmiBinding::new().import(bytes.as_bytes());
    match result {
        Err(BindingError::Import(msg)) => assert!(msg.contains("malformed")),
        other => panic!("expected a clean Import error, got {:?}", other.map(|_| ())),
    }
}

#[test]
fn export_round_trips_the_subset() {
    let (root, _) = import("coffee-grinder.xmi");
    let bytes = XmiBinding::new()
        .export(&root)
        .expect("export must succeed");
    let (round_tripped, _) = XmiBinding::new()
        .import(&bytes)
        .expect("re-import must succeed");
    assert_eq!(round_tripped, root);
}

// --- MINOR: loss-report subjects are unique, so acceptance by name is unambiguous ---

#[test]
fn blocking_loss_subjects_are_unique() {
    // Acceptance keys on the subject string: two DIFFERENT losses that share a subject would
    // be impossible to accept or refuse separately. A comment produces BOTH a Lossy id-drop
    // and an Unmappable body-drop, so the reader must disambiguate their subjects.
    let (_, loss) = import("comment-nonblock.xmi");
    let mut subjects: Vec<&str> = loss.blocking().iter().map(|m| m.subject.as_str()).collect();
    subjects.sort_unstable();
    let unique: Vec<&str> = {
        let mut s = subjects.clone();
        s.dedup();
        s
    };
    assert_eq!(
        subjects.len(),
        unique.len(),
        "blocking loss subjects must be unique: {:?}",
        subjects
    );
}

// --- I3: the binding emits the empty state machine, and export tolerates it ---

#[test]
fn import_emits_the_empty_state_machine_okf_requires() {
    let (root, _) = import("coffee-grinder.xmi");
    // OKF validation requires the stateMachine section even when the model has none, so the
    // binding emits the empty section itself rather than leaving the platform to patch it in.
    assert_eq!(
        root.state_machine,
        Some(StateMachine {
            name: "stateMachine".to_string(),
            regions: Vec::new(),
        })
    );
}

#[test]
fn export_tolerates_an_empty_state_machine_but_refuses_states() {
    let (mut root, _) = import("coffee-grinder.xmi");

    // The empty state machine is the binding's own output, so export must accept it.
    XmiBinding::new()
        .export(&root)
        .expect("an empty state machine must export");

    // A state machine WITH states is outside the subset and must still be refused.
    root.state_machine = Some(StateMachine {
        name: "stateMachine".to_string(),
        regions: vec![okf::types::Region {
            states: vec![okf::types::State {
                id: "s1".to_string(),
                name: "s1".to_string(),
                entry: None,
                do_activity: None,
                exit: None,
            }],
        }],
    });
    match XmiBinding::new().export(&root) {
        Err(BindingError::Export(msg)) => assert!(msg.contains("state machine")),
        other => panic!("expected Export refusal, got {:?}", other.map(|_| ())),
    }
}

// --- CRITICAL 1: a comment whose target is not an emitted block is reported ---

#[test]
fn a_comment_on_a_non_block_target_is_reported_not_dropped() {
    let (root, loss) = import("comment-nonblock.xmi");

    // The block still imports and receives the part of the multi-target comment
    // that points at it.
    assert_eq!(root.structure.len(), 1);
    assert_eq!(root.structure[0].id, "block-a");
    assert_eq!(root.structure[0].documentation, "multi target");

    // The comment on the package, the dangling target, and the dangling half of
    // the multi-target comment are each named.
    assert!(loss.mappings.iter().any(|m| {
        m.verdict == MappingVerdict::Unmappable
            && m.subject == "uml:Comment comment-pkg body (target pkg-a)"
            && m.note.contains("not an emitted block")
    }));
    assert!(loss.mappings.iter().any(|m| {
        m.verdict == MappingVerdict::Unmappable
            && m.subject == "uml:Comment comment-dangling body (target no-such-id)"
            && m.note.contains("dangling id")
    }));
    assert!(loss.mappings.iter().any(|m| {
        m.verdict == MappingVerdict::Unmappable
            && m.subject == "uml:Comment comment-multi body (target no-such-id)"
            && m.note.contains("dangling id")
    }));
}

// --- I1: name dropped on Comment and on a stereotyped Dependency is reported ---

#[test]
fn a_comment_name_and_a_stereotyped_dependency_name_are_reported() {
    let (_, loss) = import("name-loss.xmi");

    // The comment body still attaches to its block.
    assert!(loss.mappings.iter().any(|m| {
        m.verdict == MappingVerdict::Unmappable
            && m.subject == "uml:Comment doc-nl attribute 'name'"
    }));
    // The stereotyped dependency's name is dropped (the label is the stereotype)
    // and named.
    assert!(loss.mappings.iter().any(|m| {
        m.verdict == MappingVerdict::Lossy
            && m.subject == "uml:Dependency dep-nl (trace link)"
            && m.note.contains("name")
    }));
}

// --- I2: export refuses out-of-subset stereotypes and graph-node mismatch ---

#[test]
fn export_refuses_a_structure_element_with_extra_stereotypes() {
    let (mut root, _) = import("coffee-grinder.xmi");
    root.structure[0].stereotypes.push("Trace".to_string());
    let result = XmiBinding::new().export(&root);
    match result {
        Err(BindingError::Export(msg)) => assert!(msg.contains("stereotype")),
        other => panic!("expected Export refusal, got {:?}", other.map(|_| ())),
    }
}

#[test]
fn export_refuses_a_graph_node_whose_name_does_not_match_its_element() {
    let (mut root, _) = import("coffee-grinder.xmi");
    root.graph.as_mut().unwrap().nodes[0].name = "Wrong Name".to_string();
    let result = XmiBinding::new().export(&root);
    match result {
        Err(BindingError::Export(msg)) => assert!(msg.contains("name")),
        other => panic!("expected Export refusal, got {:?}", other.map(|_| ())),
    }
}

#[test]
fn export_refuses_a_graph_node_whose_stereotypes_do_not_match_its_element() {
    let (mut root, _) = import("coffee-grinder.xmi");
    root.graph.as_mut().unwrap().nodes[0].stereotypes = vec!["Trace".to_string()];
    let result = XmiBinding::new().export(&root);
    match result {
        Err(BindingError::Export(msg)) => assert!(msg.contains("stereotype")),
        other => panic!("expected Export refusal, got {:?}", other.map(|_| ())),
    }
}

// --- I3: properties of a non-block class are individually named ---

#[test]
fn properties_of_a_non_block_class_are_named_not_passed_over() {
    let (root, loss) = import("nonblock-class.xmi");

    // No block, so nothing is emitted.
    assert!(root.structure.is_empty());

    // The class and its property are each named, with the property's id.
    assert!(loss.mappings.iter().any(|m| {
        m.verdict == MappingVerdict::Unmappable
            && m.subject == "uml:Class plain-class (Plain Class)"
    }));
    assert!(loss.mappings.iter().any(|m| {
        m.verdict == MappingVerdict::Unmappable && m.subject == "uml:Property prop-plain (thing)"
    }));
}

// --- M1: attributes on the xmi:XMI root element are reported ---

#[test]
fn an_attribute_on_the_root_element_is_reported() {
    let (_, loss) = import("root-attr.xmi");
    assert!(loss.mappings.iter().any(|m| {
        m.verdict == MappingVerdict::Unmappable
            && m.subject.contains("root attribute")
            && m.subject.contains("version")
    }));
}

// --- M2: a foreign attribute is not read as UML, and the ambiguity is reported ---

#[test]
fn a_foreign_attribute_with_a_uml_local_name_is_not_mistaken_for_uml() {
    let (root, loss) = import("foreign-attr.xmi");

    // The unqualified UML name wins; the foreign foo:name is NOT read as the name.
    assert_eq!(root.structure[0].name, "Good Name");

    // The foreign attribute is reported, with its namespace so the ambiguity is
    // visible rather than silently swallowed.
    assert!(loss.mappings.iter().any(|m| {
        m.verdict == MappingVerdict::Unmappable
            && m.subject.contains("attribute 'name")
            && m.subject.contains("http://example.com/foreign")
    }));
}

// --- M4: an empty document is a clean error, never a panic ---

#[test]
fn an_empty_document_is_a_clean_error_not_a_panic() {
    for empty in [
        b"".as_slice(),
        b"   
  "
        .as_slice(),
    ] {
        let result = XmiBinding::new().import(empty);
        match result {
            Err(BindingError::Import(_)) => {}
            other => panic!("expected a clean Import error, got {:?}", other.map(|_| ())),
        }
    }
}

#[test]
fn a_document_without_a_model_root_is_a_clean_error_not_a_panic() {
    let bytes = b"<xmi:XMI xmlns:xmi='http://www.omg.org/spec/XMI/20131001'/>";
    let result = XmiBinding::new().import(bytes);
    match result {
        Err(BindingError::Import(msg)) => assert!(msg.contains("uml:Model")),
        other => panic!("expected a clean Import error, got {:?}", other.map(|_| ())),
    }
}
