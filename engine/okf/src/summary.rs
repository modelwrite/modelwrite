// SPDX-License-Identifier: AGPL-3.0-or-later
//! The OKF summary is DERIVED: it counts the sections and the graph. Nothing else is
//! authoritative about it, so the only honest way to write one is to recompute it from the
//! document it describes. A stored summary that disagrees with its own sections is a false
//! claim inside the model - the exact class of thing the platform exists to catch - so every
//! write path must recompute it rather than trust a client-supplied count.
//!
//! derive computes the counts from a document; recompute writes them back into the document's
//! summary field. The two are the ONE place the counts are decided, so the validator's
//! cross-check, the commit path and the merge path can never disagree about what the summary
//! should say.

use crate::types::{OkfRoot, Summary};

/// The summary a document SHOULD assert: one count per element section plus the graph node
/// and edge counts, read from the document itself.
pub fn derive(root: &OkfRoot) -> Summary {
    Summary {
        blocks: root.structure.len() as u64,
        requirements: root.requirements.len() as u64,
        interfaces: root.interfaces.len() as u64,
        signals: root.signals.len() as u64,
        activities: root.activities.len() as u64,
        graph_nodes: root.graph.as_ref().map(|g| g.nodes.len()).unwrap_or(0) as u64,
        graph_edges: root.graph.as_ref().map(|g| g.edges.len()).unwrap_or(0) as u64,
    }
}

/// Recompute a document's summary in place so it cannot contradict its own content.
pub fn recompute(root: &mut OkfRoot) {
    root.summary = derive(root);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Element, Graph, GraphEdge, GraphNode};

    #[test]
    fn derive_counts_every_section_and_the_graph() {
        let mut root = OkfRoot {
            okf: "1.0".to_string(),
            project: "p".to_string(),
            exported_at: String::new(),
            summary: Summary::default(),
            structure: vec![Element {
                id: "a".to_string(),
                name: String::new(),
                kind: "block".to_string(),
                stereotypes: Vec::new(),
                attributes: Vec::new(),
                documentation: String::new(),
            }],
            interfaces: vec![Element {
                id: "if".to_string(),
                name: String::new(),
                kind: "interface".to_string(),
                stereotypes: Vec::new(),
                attributes: Vec::new(),
                documentation: String::new(),
            }],
            signals: Vec::new(),
            requirements: Vec::new(),
            state_machine: None,
            activities: Vec::new(),
            graph: Some(Graph {
                nodes: vec![GraphNode {
                    id: "a".to_string(),
                    kind: "block".to_string(),
                    name: "A".to_string(),
                    stereotypes: Vec::new(),
                }],
                edges: vec![GraphEdge {
                    source: "a".to_string(),
                    target: "if".to_string(),
                    kind: "part".to_string(),
                    label: String::new(),
                }],
            }),
            provenance: None,
            references: Vec::new(),
        };

        // A stale summary is overwritten by the derived one.
        root.summary = Summary {
            blocks: 99,
            requirements: 99,
            interfaces: 99,
            signals: 99,
            activities: 99,
            graph_nodes: 99,
            graph_edges: 99,
        };
        recompute(&mut root);

        assert_eq!(root.summary.blocks, 1);
        assert_eq!(root.summary.interfaces, 1);
        assert_eq!(root.summary.signals, 0);
        assert_eq!(root.summary.requirements, 0);
        assert_eq!(root.summary.activities, 0);
        assert_eq!(root.summary.graph_nodes, 1);
        assert_eq!(root.summary.graph_edges, 1);
    }
}
