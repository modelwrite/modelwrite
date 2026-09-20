// SPDX-License-Identifier: AGPL-3.0-or-later
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Attribute {
    pub name: String,
    #[serde(rename = "type", default)]
    pub attr_type: String,
    #[serde(default)]
    pub aggregation: String,
    #[serde(default)]
    pub default: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Element {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub stereotypes: Vec<String>,
    #[serde(default)]
    pub attributes: Vec<Attribute>,
    #[serde(default)]
    pub documentation: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Requirement {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub stereotypes: Vec<String>,
    #[serde(default)]
    pub attributes: Vec<Attribute>,
    #[serde(default)]
    pub documentation: String,
    #[serde(rename = "reqId", default)]
    pub req_id: String,
    #[serde(rename = "reqText", default)]
    pub req_text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct State {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub entry: Option<String>,
    #[serde(rename = "doActivity", default)]
    pub do_activity: Option<String>,
    #[serde(default)]
    pub exit: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Region {
    #[serde(default)]
    pub states: Vec<State>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StateMachine {
    pub name: String,
    #[serde(default)]
    pub regions: Vec<Region>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ActivityNode {
    pub id: String,
    #[serde(rename = "type", default)]
    pub node_type: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub partition: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ActivityEdge {
    #[serde(rename = "type", default)]
    pub edge_type: String,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub target: String,
    #[serde(default)]
    pub guard: String,
}

/// A swim-lane on an activity diagram. The fixture stores partitions as
/// objects with a name and an optional represented element, not as plain strings.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Partition {
    pub name: String,
    #[serde(default)]
    pub represents: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Activity {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub partitions: Vec<Partition>,
    #[serde(default)]
    pub nodes: Vec<ActivityNode>,
    #[serde(default)]
    pub edges: Vec<ActivityEdge>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GraphNode {
    pub id: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub stereotypes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GraphEdge {
    pub source: String,
    pub target: String,
    pub kind: String,
    #[serde(default)]
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Graph {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Summary {
    #[serde(default)]
    pub blocks: u64,
    #[serde(default)]
    pub requirements: u64,
    #[serde(default)]
    pub interfaces: u64,
    #[serde(default)]
    pub signals: u64,
    #[serde(default)]
    pub activities: u64,
    #[serde(rename = "graphNodes", default)]
    pub graph_nodes: u64,
    #[serde(rename = "graphEdges", default)]
    pub graph_edges: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Provenance {
    #[serde(rename = "sourceTool", default)]
    pub source_tool: String,
    #[serde(default)]
    pub exporter: String,
    #[serde(rename = "exporterVersion", default)]
    pub exporter_version: String,
}

/// A typed, content-addressed cross-model edge: a platform element (an activity or a
/// requirement) declares that it is implemented by, or satisfied by, a named element inside
/// the pinned subsystem revision its reference binds to. The edge carries no revision of its
/// own: it lives inside a SubsystemReference and inherits that reference's (project,
/// revision, role) - the content address, never "latest". When the reference's revision
/// changes, every edge re-resolves against the new revision or fails by name.
///
/// This is the S2 primitive that SubsystemReference::bounds alone cannot express: bounds
/// names WHICH elements the platform binds to, but nothing says WHICH platform element a
/// bound element implements or satisfies. The edge closes that gap as one typed edge used by
/// the compositional gate, the composition page and the variant impact panel.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CrossModelEdge {
    /// The platform element the edge leaves from: an activity id or a requirement id.
    pub from: String,
    /// The relationship: "implementedBy" (an activity implemented by the subsystem element)
    /// or "satisfiedBy" (a requirement satisfied by the subsystem element).
    pub relation: String,
    /// The element id inside the pinned revision that implements or satisfies 'from'.
    pub to: String,
}

/// A typed subsystem reference: a platform model declares each integrated subsystem as a
/// (project, pinned commit hash, role) triple. This is the R1 primitive - a reference, never
/// a copy - and R2's pinned revision: the commit hash is part of the reference's identity,
/// never a branch name and never "latest".
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SubsystemReference {
    /// The identity of the subsystem project.
    pub project: String,
    /// The pinned commit hash of the subsystem model integrated by this platform - the
    /// content address, not a branch name and not "latest".
    pub revision: String,
    /// The role the subsystem plays in the platform (for example radar or propulsion).
    /// The role is model vocabulary, not free prose.
    pub role: String,
    /// The specific elements within the pinned revision this platform binds to - the
    /// interfaces or blocks the platform connects to - each named by element id within
    /// that revision. Empty means the reference binds to the subsystem as a whole without
    /// naming any element. The typed link from a specific platform element to a specific
    /// bound element is carried by [SubsystemReference::cross_model_edges], not by this list.
    #[serde(default)]
    pub bounds: Vec<String>,
    /// The typed cross-model traceability edges this reference carries: each connects a
    /// specific platform element (an activity or requirement) to a specific element id
    /// inside the pinned revision. This is what bounds alone cannot say - WHICH platform
    /// element is implemented or satisfied by WHICH bound element. Empty means the
    /// reference binds to the subsystem without declaring any specific traceability.
    #[serde(rename = "crossModelEdges", default)]
    pub cross_model_edges: Vec<CrossModelEdge>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OkfRoot {
    #[serde(default)]
    pub okf: String,
    pub project: String,
    #[serde(rename = "exportedAt", default)]
    pub exported_at: String,
    #[serde(default)]
    pub summary: Summary,
    #[serde(default)]
    pub structure: Vec<Element>,
    #[serde(default)]
    pub interfaces: Vec<Element>,
    #[serde(default)]
    pub signals: Vec<Element>,
    #[serde(default)]
    pub requirements: Vec<Requirement>,
    #[serde(rename = "stateMachine", default)]
    pub state_machine: Option<StateMachine>,
    #[serde(default)]
    pub activities: Vec<Activity>,
    #[serde(default)]
    pub graph: Option<Graph>,
    #[serde(default)]
    pub provenance: Option<Provenance>,
    /// The subsystem references a platform model declares: one entry per integrated
    /// subsystem, each (project, pinned revision, role). References are ordinary model
    /// content - committed, diffed and gated like every other element, never a side table.
    #[serde(default)]
    pub references: Vec<SubsystemReference>,
}
