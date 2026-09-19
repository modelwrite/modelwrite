// SPDX-License-Identifier: AGPL-3.0-or-later
//! The first real binding, put through the engine's OKF->XMI->OKF round trip: the
//! subset it claims to carry is exported and re-imported, and the engine's diff - which
//! reads the source independently - must agree nothing was lost ON THAT JOURNEY. The
//! native XMI->OKF read is not exercised here; it rests on the binding's loss report.

use binding::{artifact_hash, round_trip, Binding, Direction};
use binding_xmi::XmiBinding;
use okf::types::{Attribute, Element, Graph, GraphEdge, GraphNode, OkfRoot, StateMachine, Summary};

fn subset_document() -> OkfRoot {
    OkfRoot {
        okf: "1.0".to_string(),
        project: "Grinder".to_string(),
        exported_at: String::new(),
        summary: Summary {
            blocks: 2,
            requirements: 0,
            interfaces: 0,
            signals: 0,
            activities: 0,
            graph_nodes: 2,
            graph_edges: 1,
        },
        structure: vec![
            Element {
                id: "block-grinder".to_string(),
                name: "Grinder".to_string(),
                kind: "block".to_string(),
                stereotypes: vec!["Block".to_string()],
                attributes: vec![
                    Attribute {
                        name: "motor".to_string(),
                        attr_type: "Motor".to_string(),
                        aggregation: "composite".to_string(),
                        default: String::new(),
                    },
                    Attribute {
                        name: "capacity".to_string(),
                        attr_type: "Integer".to_string(),
                        aggregation: "none".to_string(),
                        default: "1".to_string(),
                    },
                ],
                documentation: "Grinds coffee beans.".to_string(),
            },
            Element {
                id: "block-motor".to_string(),
                name: "Motor".to_string(),
                kind: "block".to_string(),
                stereotypes: vec!["Block".to_string()],
                attributes: Vec::new(),
                documentation: String::new(),
            },
        ],
        interfaces: Vec::new(),
        signals: Vec::new(),
        requirements: Vec::new(),
        // The binding emits an empty state machine on import, so a source that a real
        // round trip would feed it must carry the same section or the diff reports an extra
        // element the binding itself added.
        state_machine: Some(StateMachine {
            name: "stateMachine".to_string(),
            regions: Vec::new(),
        }),
        activities: Vec::new(),
        graph: Some(Graph {
            nodes: vec![
                GraphNode {
                    id: "block-grinder".to_string(),
                    kind: "block".to_string(),
                    name: "Grinder".to_string(),
                    stereotypes: vec!["Block".to_string()],
                },
                GraphNode {
                    id: "block-motor".to_string(),
                    kind: "block".to_string(),
                    name: "Motor".to_string(),
                    stereotypes: vec!["Block".to_string()],
                },
            ],
            edges: vec![GraphEdge {
                source: "block-grinder".to_string(),
                target: "block-motor".to_string(),
                kind: "dependency".to_string(),
                label: "Satisfy".to_string(),
            }],
        }),
        provenance: None,
        references: Vec::new(),
    }
}

#[test]
fn the_real_binding_round_trips_the_subset_with_only_named_id_loss() {
    let binding = XmiBinding::new();
    assert_eq!(binding.info().id, "sysml-v1-xmi");
    assert_eq!(binding.info().direction, Direction::ImportAndExport);

    let source = serde_json::to_vec(&subset_document()).expect("source must serialize");
    let outcome = round_trip(&binding, &source).expect("round trip must succeed");

    // The engine's own diff, not the binding's claim, says nothing structural
    // was lost.
    assert!(outcome.diff.equal, "engine diff: {:?}", outcome.diff);

    // The binding is honestly lossy rather than silently lossy: it names the
    // transient XMI ids (Model, property, comment, dependency) that OKF has no
    // slot for, each with a Lossy verdict. Nothing structural is Unmappable, so
    // the migration decision rests on named, Lossy id drops - never a silent one.
    assert!(
        outcome
            .loss_report
            .mappings
            .iter()
            .all(|m| m.verdict == binding::MappingVerdict::Lossy),
        "loss report should name only dropped ids: {:?}",
        outcome.loss_report.mappings
    );
    assert!(!outcome.loss_report.mappings.is_empty());
    assert_eq!(outcome.artifact_hash, artifact_hash(&source));
}
