// SPDX-License-Identifier: AGPL-3.0-or-later
//! The Capella / Arcadia binding: reads a stated subset of a Capella `.capella`
//! model into OKF.
//!
//! This is a VIEWER, not a round-trippable binding: it reads the Capella
//! semantic model into OKF but does not write `.capella` back out. Its
//! metadata says so (Direction::ImportOnly) and its export method refuses rather
//! than pretend.
//!
//! A Capella model is a multi-file project. The semantic content lives in the
//! `.capella` file (XMI 2.0, root `capellamodeller:Project`, element type in
//! `xsi:type`, element identity in the `id` attribute, cross-references as
//! `#uuid` strings). The `.aird` file is the Sirius diagram *representation*
//! layer and the `.afm` file is viewpoint metadata; this reader imports the
//! `.capella` file and rejects the other two with a clear message.
//!
//! The subset is declared as data in [crate::model::mapping_table]. Every
//! construct outside that subset is reported as an Unmappable mapping naming the
//! element type, id and name, never dropped in silence. A malformed document is
//! a clean BindingError::Import, never a panic.

pub mod model;

use std::collections::HashSet;

use binding::{
    artifact_hash, Binding, BindingError, BindingInfo, Direction, LossReport, Mapping,
    MappingVerdict,
};
use okf::types::{
    Attribute, Element, Graph, GraphEdge, GraphNode, OkfRoot, Requirement, StateMachine, Summary,
};

/// The binding identity the gate and the workbench use to select this reader.
pub const BINDING_ID: &str = "capella-arcadia";
/// The Capella version line this binding reads (the 7.0.0 metamodel namespace).
pub const BINDING_VERSION: &str = "1.0";

const XSI_NS: &str = "http://www.w3.org/2001/XMLSchema-instance";

/// The Capella / Arcadia binding.
#[derive(Debug, Clone, Default)]
pub struct CapellaBinding;

impl CapellaBinding {
    pub fn new() -> Self {
        CapellaBinding
    }
}

impl Binding for CapellaBinding {
    fn info(&self) -> BindingInfo {
        BindingInfo {
            id: BINDING_ID.to_string(),
            version: BINDING_VERSION.to_string(),
            direction: Direction::ImportOnly,
            description: "Capella / Arcadia (.capella): components, functions, functional and component exchanges, allocations and constraints"
                .to_string(),
        }
    }

    fn mapping_table(&self) -> Vec<Mapping> {
        model::mapping_table()
    }

    fn import(&self, source: &[u8]) -> Result<(OkfRoot, LossReport), BindingError> {
        let text = std::str::from_utf8(source)
            .map_err(|e| BindingError::Import(format!("Capella source is not valid UTF-8: {e}")))?;
        let doc = roxmltree::Document::parse(text)
            .map_err(|e| BindingError::Import(format!("Capella .capella is malformed XML: {e}")))?;
        let root = doc.root_element();
        if root.tag_name().name() != "Project" {
            return Err(BindingError::Import(format!(
                "not a Capella semantic model: expected a <Project> root, found <{}>. A .aird is the Sirius diagram representation layer and a .afm is viewpoint metadata; import the .capella file",
                root.tag_name().name()
            )));
        }
        Importer::new(self.info(), artifact_hash(source)).run(root)
    }

    fn export(&self, _root: &OkfRoot) -> Result<Vec<u8>, BindingError> {
        // This binding is a viewer: it reads the .capella model but does not
        // write it back. Declaring Direction::ImportOnly in info() is the
        // metadata half of that promise; this error is the behavioural half.
        Err(BindingError::Export(
            "capella-arcadia is a viewer (ImportOnly): it reads the .capella model into OKF but cannot write it back"
                .to_string(),
        ))
    }
}

/// An unqualified attribute (Capella writes id/name/source/target/etc.
/// unqualified); matching only the unqualified form keeps a namespaced attribute
/// of the same local name from being mistaken for it.
fn attr<'a, 'input>(node: roxmltree::Node<'a, 'input>, name: &str) -> Option<&'a str> {
    node.attributes()
        .find(|a| a.name() == name && a.namespace().is_none())
        .map(|a| a.value())
}

/// The xsi:type value, matched by namespace, reduced to its local name (e.g.
/// "org.polarsys.capella.core.data.ctx:SystemComponent" -> "SystemComponent").
fn type_local<'a, 'input>(node: roxmltree::Node<'a, 'input>) -> &'a str {
    for a in node.attributes() {
        if a.name() == "type" && a.namespace() == Some(XSI_NS) {
            let v = a.value();
            return v.rsplit(':').next().unwrap_or(v);
        }
    }
    ""
}

/// Strip the leading '#' from a Capella reference ("#uuid" -> "uuid").
fn strip_hash(r: &str) -> &str {
    r.trim().trim_start_matches('#')
}

/// A carried block: a component (controller / controlled process) or a function
/// (control action), both carried as kind "block" with a distinguishing
/// stereotype.
struct BlockRec {
    id: String,
    name: String,
    stereotype: String,
    description: String,
    ports: Vec<Attribute>,
}

/// A carried requirement: a Constraint or a re:Requirement.
struct ReqRec {
    id: String,
    name: String,
    stereotype: String,
    description: String,
    req_text: String,
}

/// A pending graph edge, with its endpoints still as raw "#uuid" references
/// (resolved against the carried ids after the whole tree is indexed).
struct EdgeRec {
    /// The Capella construct name, for loss-report subjects.
    construct: &'static str,
    /// The OKF edge kind: "dependency" for an exchange, "part" for an allocation.
    kind: &'static str,
    label: String,
    source: String,
    target: String,
    id: Option<String>,
    name: String,
}

struct Importer {
    info: BindingInfo,
    artifact_hash: String,
    project: String,
    /// Port id -> the owning function/component id, so an exchange that
    /// references a port (Capella attaches FunctionalExchange/ComponentExchange
    /// to ports) resolves to the port's owner.
    port_owner: std::collections::HashMap<String, String>,
    blocks: Vec<BlockRec>,
    requirements: Vec<ReqRec>,
    edges: Vec<EdgeRec>,
    contains: Vec<(String, String)>,
    losses: Vec<Mapping>,
}

impl Importer {
    fn new(info: BindingInfo, artifact_hash: String) -> Self {
        Importer {
            info,
            artifact_hash,
            project: String::new(),
            port_owner: std::collections::HashMap::new(),
            blocks: Vec::new(),
            requirements: Vec::new(),
            edges: Vec::new(),
            contains: Vec::new(),
            losses: Vec::new(),
        }
    }

    fn run(mut self, root: roxmltree::Node<'_, '_>) -> Result<(OkfRoot, LossReport), BindingError> {
        self.project = attr(root, "name").unwrap_or("").to_string();
        self.walk(root, None, None);
        if self.project.is_empty() {
            return Err(BindingError::Import(
                "Capella Project has no name".to_string(),
            ));
        }
        self.build()
    }

    fn walk(
        &mut self,
        node: roxmltree::Node<'_, '_>,
        parent_component: Option<&str>,
        parent_function: Option<&str>,
    ) {
        for child in node.children().filter(|c| c.is_element()) {
            let typ = type_local(child);
            // A plain XML content holder (<bodies>, <languages>, ...) has no
            // xsi:type and no model identity: it is consumed as part of its
            // parent's serialisation, never a model element and never a loss.
            if typ.is_empty() {
                self.walk(child, parent_component, parent_function);
                continue;
            }
            let id = attr(child, "id").map(str::to_string);
            let name = attr(child, "name").unwrap_or("").to_string();

            if model::is_component(typ) {
                let actor = attr(child, "actor") == Some("true");
                let stereotype = if actor {
                    format!("{typ} actor")
                } else {
                    typ.to_string()
                };
                let cid = id.clone();
                let ports = self.collect_ports(child, cid.as_deref().unwrap_or(""));
                let description = attr(child, "description").unwrap_or("").to_string();
                self.blocks.push(BlockRec {
                    id: id.clone().unwrap_or_default(),
                    name: name.clone(),
                    stereotype,
                    description,
                    ports,
                });
                if let (Some(p), Some(c)) = (parent_component, &cid) {
                    self.contains.push((p.to_string(), c.clone()));
                }
                self.walk(child, cid.as_deref(), parent_function);
            } else if model::is_function(typ) {
                let stereotype = typ.to_string();
                let fid = id.clone();
                let ports = self.collect_ports(child, fid.as_deref().unwrap_or(""));
                let description = attr(child, "description").unwrap_or("").to_string();
                self.blocks.push(BlockRec {
                    id: id.clone().unwrap_or_default(),
                    name: name.clone(),
                    stereotype,
                    description,
                    ports,
                });
                if let (Some(p), Some(f)) = (parent_function, &fid) {
                    self.contains.push((p.to_string(), f.clone()));
                }
                self.walk(child, parent_component, fid.as_deref());
            } else if model::FUNCTIONAL_EXCHANGE_TYPES.contains(&typ) {
                let source = attr(child, "source").unwrap_or("").to_string();
                let target = attr(child, "target").unwrap_or("").to_string();
                self.edges.push(EdgeRec {
                    construct: "FunctionalExchange",
                    kind: "dependency",
                    label: name.clone(),
                    source,
                    target,
                    id,
                    name,
                });
            } else if model::COMPONENT_EXCHANGE_TYPES.contains(&typ) {
                let source = attr(child, "source").unwrap_or("").to_string();
                let target = attr(child, "target").unwrap_or("").to_string();
                let construct = if typ == "ComponentExchange" {
                    "ComponentExchange"
                } else {
                    "CommunicationMean"
                };
                self.edges.push(EdgeRec {
                    construct,
                    kind: "dependency",
                    label: name.clone(),
                    source,
                    target,
                    id,
                    name,
                });
            } else if typ == model::ALLOCATION_TYPE {
                let source = attr(child, "sourceElement").unwrap_or("").to_string();
                let target = attr(child, "targetElement").unwrap_or("").to_string();
                self.edges.push(EdgeRec {
                    construct: "ComponentFunctionalAllocation",
                    kind: "part",
                    label: "allocated".to_string(),
                    source,
                    target,
                    id,
                    name,
                });
                self.walk(child, parent_component, parent_function);
            } else if typ == model::CONSTRAINT_TYPE {
                let req_text = constraint_text(child);
                let description = attr(child, "description").unwrap_or("").to_string();
                self.requirements.push(ReqRec {
                    id: id.clone().unwrap_or_default(),
                    name: name.clone(),
                    stereotype: "constraint".to_string(),
                    description,
                    req_text,
                });
                self.walk(child, parent_component, parent_function);
            } else if typ == model::REQUIREMENT_TYPE {
                let req_text = attr(child, "text")
                    .or_else(|| attr(child, "requirementText"))
                    .unwrap_or("")
                    .to_string();
                let description = attr(child, "description").unwrap_or("").to_string();
                self.requirements.push(ReqRec {
                    id: id.clone().unwrap_or_default(),
                    name: name.clone(),
                    stereotype: "requirement".to_string(),
                    description,
                    req_text,
                });
                self.walk(child, parent_component, parent_function);
            } else if model::PORT_TYPES.contains(&typ) {
                // A port owned by a function/component is already carried as an
                // attribute (and its id named as a loss) by collect_ports; the
                // recursion just indexes it and keeps its children out of the
                // loss set.
                self.walk(child, parent_component, parent_function);
            } else if model::is_declaration(typ) {
                self.losses.push(Mapping {
                    subject: format!("{typ} {}", id.as_deref().unwrap_or("<no id>")),
                    verdict: MappingVerdict::Exact,
                    note: format!(
                        "declaration: {typ} recognised and not carried — it holds no STPA-relevant model content"
                    ),
                });
                self.walk(child, parent_component, parent_function);
            } else if model::is_package(typ) {
                self.losses.push(Mapping {
                    subject: format!("{typ} {}", id.as_deref().unwrap_or("<no id>")),
                    verdict: MappingVerdict::Lossy,
                    note: format!(
                        "package flattened: OKF has no namespace concept; the members of '{}' are promoted to the top level and the package id/name is dropped",
                        name
                    ),
                });
                self.walk(child, parent_component, parent_function);
            } else {
                self.losses.push(Mapping {
                    subject: format!("{typ} {}", id.as_deref().unwrap_or("<no id>")),
                    verdict: MappingVerdict::Unmappable,
                    note: format!(
                        "outside the capella-arcadia subset; element '{}' is named, never dropped in silence",
                        name
                    ),
                });
                self.walk(child, parent_component, parent_function);
            }
        }
    }

    /// Carry the ports directly owned by a function or component as attributes,
    /// record each port's owner (so an exchange endpoint can resolve through the
    /// port to the owning function/component), and name each port's id (which has
    /// no OKF Attribute slot).
    fn collect_ports(&mut self, node: roxmltree::Node<'_, '_>, owner: &str) -> Vec<Attribute> {
        let mut ports = Vec::new();
        for child in node.children().filter(|c| c.is_element()) {
            let ptype = type_local(child);
            if model::PORT_TYPES.contains(&ptype) {
                let pname = attr(child, "name").unwrap_or("").to_string();
                ports.push(Attribute {
                    name: pname.clone(),
                    attr_type: ptype.to_string(),
                    aggregation: String::new(),
                    default: String::new(),
                });
                if let Some(pid) = attr(child, "id") {
                    self.port_owner.insert(pid.to_string(), owner.to_string());
                    self.losses.push(Mapping {
                        subject: format!("{ptype} {pid}"),
                        verdict: MappingVerdict::Lossy,
                        note: format!(
                            "port '{}' id dropped: OKF Attribute has no id slot; the port is carried as an attribute of its owner",
                            pname
                        ),
                    });
                }
            }
        }
        ports
    }

    /// Resolve an exchange/alloc endpoint to a carried node id: a direct id of a
    /// carried block/requirement, or a port id resolved through to its owner.
    fn resolve_endpoint(&self, raw: &str, carried: &HashSet<String>) -> Option<String> {
        let id = strip_hash(raw);
        if carried.contains(id) {
            return Some(id.to_string());
        }
        if let Some(owner) = self.port_owner.get(id) {
            return Some(owner.clone());
        }
        None
    }

    fn build(mut self) -> Result<(OkfRoot, LossReport), BindingError> {
        let mut structure: Vec<Element> = Vec::new();
        let mut graph_nodes: Vec<GraphNode> = Vec::new();
        for b in &self.blocks {
            structure.push(Element {
                id: b.id.clone(),
                name: b.name.clone(),
                kind: "block".to_string(),
                stereotypes: vec![b.stereotype.clone()],
                attributes: b.ports.clone(),
                documentation: b.description.clone(),
            });
            graph_nodes.push(GraphNode {
                id: b.id.clone(),
                kind: "block".to_string(),
                name: b.name.clone(),
                stereotypes: vec![b.stereotype.clone()],
            });
        }

        let mut requirements: Vec<Requirement> = Vec::new();
        for r in &self.requirements {
            requirements.push(Requirement {
                id: r.id.clone(),
                name: r.name.clone(),
                kind: "requirement".to_string(),
                stereotypes: vec![r.stereotype.clone()],
                attributes: Vec::new(),
                documentation: r.description.clone(),
                req_id: String::new(),
                req_text: r.req_text.clone(),
            });
            graph_nodes.push(GraphNode {
                id: r.id.clone(),
                kind: "requirement".to_string(),
                name: r.name.clone(),
                stereotypes: vec![r.stereotype.clone()],
            });
        }

        let carried: HashSet<String> = self
            .blocks
            .iter()
            .map(|b| b.id.clone())
            .chain(self.requirements.iter().map(|r| r.id.clone()))
            .collect();

        let mut graph_edges: Vec<GraphEdge> = Vec::new();
        for (parent, child) in &self.contains {
            graph_edges.push(GraphEdge {
                source: parent.clone(),
                target: child.clone(),
                kind: "contains".to_string(),
                label: String::new(),
            });
        }

        for e in &self.edges {
            // The edge element's own id has no OKF GraphEdge slot: named,
            // regardless of whether the endpoints resolve.
            if let Some(eid) = &e.id {
                self.losses.push(Mapping {
                    subject: format!("{} {eid}", e.construct),
                    verdict: MappingVerdict::Lossy,
                    note: "edge id dropped: OKF GraphEdge has no id slot; the relationship is carried by the edge".to_string(),
                });
            }
            let source = self.resolve_endpoint(&e.source, &carried);
            let target = self.resolve_endpoint(&e.target, &carried);
            match (source, target) {
                (Some(source), Some(target)) => graph_edges.push(GraphEdge {
                    source,
                    target,
                    kind: e.kind.to_string(),
                    label: e.label.clone(),
                }),
                _ => self.losses.push(Mapping {
                    subject: format!(
                        "{} {}",
                        e.construct,
                        e.id.clone().unwrap_or_else(|| "<no id>".to_string())
                    ),
                    verdict: MappingVerdict::Unmappable,
                    note: format!(
                        "flow '{}' has an endpoint that does not resolve to a carried component/function; the relationship is named, never dropped",
                        e.name
                    ),
                }),
            }
        }

        let summary = Summary {
            blocks: structure.len() as u64,
            requirements: requirements.len() as u64,
            interfaces: 0,
            signals: 0,
            activities: 0,
            graph_nodes: graph_nodes.len() as u64,
            graph_edges: graph_edges.len() as u64,
        };

        let root = OkfRoot {
            okf: "1.0".to_string(),
            project: self.project.clone(),
            exported_at: String::new(),
            summary,
            structure,
            interfaces: Vec::new(),
            signals: Vec::new(),
            requirements,
            state_machine: Some(StateMachine {
                name: "stateMachine".to_string(),
                regions: Vec::new(),
            }),
            activities: Vec::new(),
            graph: Some(Graph {
                nodes: graph_nodes,
                edges: graph_edges,
            }),
            provenance: None,
            references: Vec::new(),
        };

        let report = LossReport {
            binding: self.info,
            mappings: self.losses,
            artifact_hash: self.artifact_hash,
        };
        Ok((root, report))
    }
}

/// The text of a Constraint's OpaqueExpression specification body.
fn constraint_text(node: roxmltree::Node<'_, '_>) -> String {
    for spec in node
        .children()
        .filter(|c| c.is_element() && c.tag_name().name() == "ownedSpecification")
    {
        for bodies in spec
            .children()
            .filter(|c| c.is_element() && c.tag_name().name() == "bodies")
        {
            if let Some(t) = bodies.text() {
                let t = t.trim();
                if !t.is_empty() {
                    return t.to_string();
                }
            }
        }
    }
    String::new()
}
