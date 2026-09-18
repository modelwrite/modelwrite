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
}
