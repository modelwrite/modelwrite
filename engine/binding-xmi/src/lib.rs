// SPDX-License-Identifier: AGPL-3.0-or-later
//! The SysML v1 XMI binding: reads the stated subset into OKF and writes it back.
//!
//! The subset is declared as data in [crate::model::mapping_table]. Every XMI
//! element outside that subset is reported as an Unmappable mapping naming the
//! element and its xmi:id, never dropped in silence. A malformed document is a
//! clean BindingError::Import, never a panic.

pub mod model;

use std::collections::{HashMap, HashSet};

use binding::{
    artifact_hash, Binding, BindingError, BindingInfo, Direction, LossReport, Mapping,
    MappingVerdict,
};
use okf::types::{Attribute, Element, Graph, GraphEdge, GraphNode, OkfRoot, Summary};

/// The binding identity the gate and the workbench use to select this reader.
pub const BINDING_ID: &str = "sysml-v1-xmi";
/// The SysML v1 version this binding reads.
pub const BINDING_VERSION: &str = "2.4";

const XMI_NS: &str = "http://www.omg.org/spec/XMI/20131001";

/// The SysML v1 XMI binding.
#[derive(Debug, Clone, Default)]
pub struct XmiBinding;

impl XmiBinding {
    pub fn new() -> Self {
        XmiBinding
    }
}

impl Binding for XmiBinding {
    fn info(&self) -> BindingInfo {
        BindingInfo {
            id: BINDING_ID.to_string(),
            version: BINDING_VERSION.to_string(),
            direction: Direction::ImportAndExport,
            description:
                "SysML v1 (UML profile) XMI: blocks, properties, dependency edges and documentation"
                    .to_string(),
        }
    }

    fn mapping_table(&self) -> Vec<Mapping> {
        model::mapping_table()
    }

    fn import(&self, source: &[u8]) -> Result<(OkfRoot, LossReport), BindingError> {
        let text = std::str::from_utf8(source)
            .map_err(|e| BindingError::Import(format!("XMI source is not valid UTF-8: {e}")))?;
        let doc = roxmltree::Document::parse(text)
            .map_err(|e| BindingError::Import(format!("malformed XMI document: {e}")))?;
        let importer = Importer::new(self.info(), artifact_hash(source));
        importer.run(doc.root_element())
    }

    fn export(&self, root: &OkfRoot) -> Result<Vec<u8>, BindingError> {
        export_document(root)
    }
}

/// The trailing local part of a QName such as "uml:Class".
fn local_part(qname: &str) -> &str {
    qname.rsplit(':').next().unwrap_or(qname)
}

/// An attribute matched by local name only (for unambiguous names like name,
/// aggregation, client, supplier, body and the base_* references).
fn attr<'a, 'input>(node: roxmltree::Node<'a, 'input>, name: &str) -> Option<&'a str> {
    for a in node.attributes() {
        if a.name() == name {
            return Some(a.value());
        }
    }
    None
}

/// An attribute matched by namespace URI and local name (for xmi:id and xmi:type,
/// whose local names collide with the UML "type" reference).
fn attr_ns<'a, 'input>(
    node: roxmltree::Node<'a, 'input>,
    ns: Option<&str>,
    name: &str,
) -> Option<&'a str> {
    for a in node.attributes() {
        if a.name() == name && a.namespace() == ns {
            return Some(a.value());
        }
    }
    None
}

/// The xmi:type value, matched by namespace so it is never confused with a UML
/// "type" reference on a property.
fn xmi_type<'a, 'input>(node: roxmltree::Node<'a, 'input>) -> Option<&'a str> {
    attr_ns(node, Some(XMI_NS), "type")
}

/// The first base_* reference on a stereotype application element.
fn base_ref<'a, 'input>(node: roxmltree::Node<'a, 'input>) -> Option<&'a str> {
    for a in node.attributes() {
        if a.name().starts_with("base_") {
            return Some(a.value());
        }
    }
    None
}

/// Resolve a property type reference: an xmi:id resolves to the named element,
/// anything else is a primitive type literal carried verbatim.
fn resolve_type(type_ref: &str, id_to_name: &HashMap<&str, &str>) -> String {
    id_to_name
        .get(type_ref)
        .map(|n| (*n).to_string())
        .unwrap_or_else(|| type_ref.to_string())
}

fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
    out
}

/// The kind of a recognised XMI element, used to decide which attributes are
/// understood (and therefore which are reported as unmapped).
#[derive(Clone, Copy)]
enum ElementKind {
    Model,
    Package,
    Class,
    Property,
    Dependency,
    Comment,
    Stereotype,
}

struct RawProperty {
    name: String,
    type_ref: String,
    aggregation: String,
    default: String,
}

struct RawClass {
    id: String,
    name: String,
    properties: Vec<RawProperty>,
}

struct RawDependency {
    id: String,
    name: String,
    client: String,
    supplier: String,
}

struct Importer {
    info: BindingInfo,
    artifact_hash: String,
    project: String,
    found_model: bool,
    classes: Vec<RawClass>,
    block_ids: HashSet<String>,
    dependencies: Vec<RawDependency>,
    dep_stereotypes: HashMap<String, String>,
    packages: Vec<(String, String)>,
    comments: Vec<(String, String)>,
    losses: Vec<Mapping>,
}

impl Importer {
    fn new(info: BindingInfo, artifact_hash: String) -> Self {
        Importer {
            info,
            artifact_hash,
            project: String::new(),
            found_model: false,
            classes: Vec::new(),
            block_ids: HashSet::new(),
            dependencies: Vec::new(),
            dep_stereotypes: HashMap::new(),
            packages: Vec::new(),
            comments: Vec::new(),
            losses: Vec::new(),
        }
    }

    fn run(mut self, root: roxmltree::Node<'_, '_>) -> Result<(OkfRoot, LossReport), BindingError> {
        // Walk the root itself when the document omits the xmi:XMI wrapper, else
        // walk its children (the uml:Model and any sibling extensions).
        let root_type = xmi_type(root).map(local_part);
        if root_type == Some("Model") || root.tag_name().name() == "Model" {
            self.walk(root, None);
        } else {
            for child in root.children() {
                self.walk(child, None);
            }
        }
        if !self.found_model {
            return Err(BindingError::Import(
                "XMI document has no uml:Model root element".to_string(),
            ));
        }
        Ok(self.build())
    }

    fn walk(&mut self, node: roxmltree::Node<'_, '_>, enclosing_class: Option<&str>) {
        // children() yields text and comment nodes too; only elements carry a
        // UML type or an xmi:id, so skip everything else.
        if !node.is_element() {
            return;
        }
        let tag = node.tag_name().name().to_string();
        let type_full = xmi_type(node).map(|s| s.to_string());
        let type_local = type_full.as_deref().map(local_part);
        let id = attr_ns(node, Some(XMI_NS), "id").unwrap_or("").to_string();

        // A stereotype application element carries no xmi:type; it is recognised
        // by its local name and its base_* reference.
        if type_local.is_none() {
            if tag == model::BLOCK_STEREOTYPE {
                if let Some(base) = base_ref(node) {
                    self.block_ids.insert(base.to_string());
                    self.report_unmapped_attributes(
                        node,
                        ElementKind::Stereotype,
                        &format!("Block {id}"),
                    );
                    return;
                }
            }
            if model::DEPENDENCY_STEREOTYPES.contains(&tag.as_str()) {
                if let Some(base) = base_ref(node) {
                    self.dep_stereotypes.insert(base.to_string(), tag.clone());
                    self.report_unmapped_attributes(
                        node,
                        ElementKind::Stereotype,
                        &format!("{tag} {id}"),
                    );
                    return;
                }
            }
        }

        // The effective type is the xmi:type local name when present, else the
        // tag name (the uml:Model root and stereotype applications carry no xmi:type).
        let effective = match &type_full {
            Some(full) => local_part(full).to_string(),
            None => tag.clone(),
        };
        match effective.as_str() {
            "Model" => {
                self.found_model = true;
                if self.project.is_empty() {
                    self.project = attr(node, "name").unwrap_or("").to_string();
                }
                self.report_unmapped_attributes(
                    node,
                    ElementKind::Model,
                    &format!("uml:Model {id}"),
                );
                for child in node.children() {
                    self.walk(child, None);
                }
            }
            "Package" => {
                let name = attr(node, "name").unwrap_or("").to_string();
                self.packages.push((id.clone(), name));
                self.report_unmapped_attributes(
                    node,
                    ElementKind::Package,
                    &format!("uml:Package {id}"),
                );
                for child in node.children() {
                    self.walk(child, None);
                }
            }
            "Class" => {
                let name = attr(node, "name").unwrap_or("").to_string();
                self.classes.push(RawClass {
                    id: id.clone(),
                    name,
                    properties: Vec::new(),
                });
                self.report_unmapped_attributes(
                    node,
                    ElementKind::Class,
                    &format!("uml:Class {id}"),
                );
                for child in node.children() {
                    self.walk(child, Some(&id));
                }
            }
            "Property" => {
                let name = attr(node, "name").unwrap_or("").to_string();
                let type_ref = node
                    .attributes()
                    .find(|a| a.name() == "type" && a.namespace() != Some(XMI_NS))
                    .map(|a| a.value())
                    .unwrap_or("")
                    .to_string();
                let aggregation = attr(node, "aggregation").unwrap_or("none").to_string();
                let default = attr(node, "default").unwrap_or("").to_string();
                self.report_unmapped_attributes(
                    node,
                    ElementKind::Property,
                    &format!("uml:Property {id}"),
                );
                match enclosing_class {
                    Some(owner) => {
                        if let Some(c) = self.classes.iter_mut().find(|c| c.id == owner) {
                            c.properties.push(RawProperty {
                                name,
                                type_ref,
                                aggregation,
                                default,
                            });
                        }
                    }
                    None => {
                        self.losses.push(Mapping {
                            subject: format!("uml:Property {id}"),
                            verdict: MappingVerdict::Unmappable,
                            note: "uml:Property outside any class".to_string(),
                        });
                    }
                }
            }
            "Dependency" => {
                let name = attr(node, "name").unwrap_or("").to_string();
                let client = attr(node, "client").unwrap_or("").to_string();
                let supplier = attr(node, "supplier").unwrap_or("").to_string();
                self.report_unmapped_attributes(
                    node,
                    ElementKind::Dependency,
                    &format!("uml:Dependency {id}"),
                );
                self.dependencies.push(RawDependency {
                    id: id.clone(),
                    name,
                    client,
                    supplier,
                });
                for child in node.children() {
                    self.walk(child, None);
                }
            }
            "Comment" => {
                let body = attr(node, "body").unwrap_or("").to_string();
                let annotated = attr(node, "annotatedElement").unwrap_or("").to_string();
                self.report_unmapped_attributes(
                    node,
                    ElementKind::Comment,
                    &format!("uml:Comment {id}"),
                );
                let target = if !annotated.is_empty() {
                    annotated
                } else {
                    enclosing_class.unwrap_or("").to_string()
                };
                if target.is_empty() {
                    self.losses.push(Mapping {
                        subject: format!("uml:Comment {id}"),
                        verdict: MappingVerdict::Unmappable,
                        note: "uml:Comment not attached to any element".to_string(),
                    });
                } else {
                    self.comments.push((target, body));
                }
            }
            _ => {
                let element_name = type_full.as_deref().unwrap_or(tag.as_str()).to_string();
                let id_part = if id.is_empty() {
                    "<no xmi:id>".to_string()
                } else {
                    id.clone()
                };
                self.losses.push(Mapping {
                    subject: format!("{element_name} {id_part}"),
                    verdict: MappingVerdict::Unmappable,
                    note: "unknown XMI element: outside the sysml-v1-xmi subset".to_string(),
                });
            }
        }
    }

    fn report_unmapped_attributes(
        &mut self,
        node: roxmltree::Node<'_, '_>,
        kind: ElementKind,
        element_name: &str,
    ) {
        for a in node.attributes() {
            let name = a.name();
            let ns = a.namespace();
            let consumed = match kind {
                ElementKind::Model | ElementKind::Package | ElementKind::Class => {
                    (name == "id" && ns == Some(XMI_NS))
                        || (name == "type" && ns == Some(XMI_NS))
                        || name == "name"
                }
                ElementKind::Property => {
                    (name == "id" && ns == Some(XMI_NS))
                        || (name == "type" && ns == Some(XMI_NS))
                        || (name == "type" && ns != Some(XMI_NS))
                        || name == "name"
                        || name == "aggregation"
                        || name == "default"
                }
                ElementKind::Dependency => {
                    (name == "id" && ns == Some(XMI_NS))
                        || (name == "type" && ns == Some(XMI_NS))
                        || name == "name"
                        || name == "client"
                        || name == "supplier"
                }
                ElementKind::Comment => {
                    (name == "id" && ns == Some(XMI_NS))
                        || (name == "type" && ns == Some(XMI_NS))
                        || name == "name"
                        || name == "body"
                        || name == "annotatedElement"
                }
                ElementKind::Stereotype => {
                    (name == "id" && ns == Some(XMI_NS)) || name.starts_with("base_")
                }
            };
            if !consumed {
                self.losses.push(Mapping {
                    subject: format!("{element_name} attribute '{name}'"),
                    verdict: MappingVerdict::Unmappable,
                    note: "unmapped attribute on a recognised XMI element".to_string(),
                });
            }
        }
    }

    fn build(mut self) -> (OkfRoot, LossReport) {
        let mut id_to_name: HashMap<&str, &str> = HashMap::new();
        for c in &self.classes {
            id_to_name.insert(c.id.as_str(), c.name.as_str());
        }

        // Precompute documentation per target id so the borrow checker does not
        // have to hold a closure over self while losses are being pushed.
        let mut docs: HashMap<String, String> = HashMap::new();
        for (target, body) in &self.comments {
            docs.entry(target.clone()).or_default().push_str(body);
        }

        let mut structure: Vec<Element> = Vec::new();
        for c in &self.classes {
            if self.block_ids.contains(&c.id) {
                let documentation = docs.get(&c.id).cloned().unwrap_or_default();
                let attributes = c
                    .properties
                    .iter()
                    .map(|p| Attribute {
                        name: p.name.clone(),
                        attr_type: resolve_type(&p.type_ref, &id_to_name),
                        aggregation: p.aggregation.clone(),
                        default: p.default.clone(),
                    })
                    .collect();
                structure.push(Element {
                    id: c.id.clone(),
                    name: c.name.clone(),
                    kind: "block".to_string(),
                    stereotypes: vec![model::BLOCK_STEREOTYPE.to_string()],
                    attributes,
                    documentation,
                });
            } else {
                self.losses.push(Mapping {
                    subject: format!("uml:Class {} ({})", c.id, c.name),
                    verdict: MappingVerdict::Unmappable,
                    note: "uml:Class without the Block stereotype; not a block".to_string(),
                });
            }
        }

        let graph_nodes: Vec<GraphNode> = structure
            .iter()
            .map(|el| GraphNode {
                id: el.id.clone(),
                kind: "block".to_string(),
                name: el.name.clone(),
                stereotypes: vec![model::BLOCK_STEREOTYPE.to_string()],
            })
            .collect();

        let mut graph_edges: Vec<GraphEdge> = Vec::new();
        for d in &self.dependencies {
            if let Some(stereotype) = self.dep_stereotypes.get(&d.id) {
                graph_edges.push(GraphEdge {
                    source: d.client.clone(),
                    target: d.supplier.clone(),
                    kind: "dependency".to_string(),
                    label: stereotype.clone(),
                });
            } else {
                self.losses.push(Mapping {
                    subject: format!("uml:Dependency {} ({})", d.id, d.name),
                    verdict: MappingVerdict::Unmappable,
                    note: "uml:Dependency without a recognised Satisfy/Allocate/Refine/Verify stereotype"
                        .to_string(),
                });
            }
        }

        for (pid, pname) in &self.packages {
            self.losses.push(Mapping {
                subject: format!("uml:Package {pid} ({pname})"),
                verdict: MappingVerdict::Lossy,
                note: "OKF has no package/namespace concept; members are promoted to the top-level structure"
                    .to_string(),
            });
        }

        let summary = Summary {
            blocks: structure.len() as u64,
            requirements: 0,
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
            requirements: Vec::new(),
            state_machine: None,
            activities: Vec::new(),
            graph: Some(Graph {
                nodes: graph_nodes,
                edges: graph_edges,
            }),
            provenance: None,
        };

        let loss = LossReport {
            binding: self.info,
            mappings: self.losses,
            artifact_hash: self.artifact_hash,
        };
        (root, loss)
    }
}

fn export_document(root: &OkfRoot) -> Result<Vec<u8>, BindingError> {
    for el in &root.structure {
        if el.kind != "block" {
            return Err(BindingError::Export(format!(
                "cannot export structure element {} of kind '{}': only blocks are in the sysml-v1-xmi subset",
                el.id, el.kind
            )));
        }
    }
    if !root.interfaces.is_empty() {
        return Err(BindingError::Export(
            "cannot export interfaces: outside the sysml-v1-xmi subset".to_string(),
        ));
    }
    if !root.signals.is_empty() {
        return Err(BindingError::Export(
            "cannot export signals: outside the sysml-v1-xmi subset".to_string(),
        ));
    }
    if !root.requirements.is_empty() {
        return Err(BindingError::Export(
            "cannot export requirements: outside the sysml-v1-xmi subset".to_string(),
        ));
    }
    if !root.activities.is_empty() {
        return Err(BindingError::Export(
            "cannot export activities: outside the sysml-v1-xmi subset".to_string(),
        ));
    }
    if root.state_machine.is_some() {
        return Err(BindingError::Export(
            "cannot export a state machine: outside the sysml-v1-xmi subset".to_string(),
        ));
    }

    let mut name_to_id: HashMap<&str, &str> = HashMap::new();
    for el in &root.structure {
        name_to_id.insert(el.name.as_str(), el.id.as_str());
    }

    if let Some(graph) = &root.graph {
        let expected: Vec<&str> = root.structure.iter().map(|e| e.id.as_str()).collect();
        let actual: Vec<&str> = graph.nodes.iter().map(|n| n.id.as_str()).collect();
        if actual != expected {
            return Err(BindingError::Export(
                "graph nodes do not mirror the structure blocks; export would silently lose them"
                    .to_string(),
            ));
        }
        for n in &graph.nodes {
            if n.kind != "block" {
                return Err(BindingError::Export(format!(
                    "cannot export graph node {} of kind '{}': outside the subset",
                    n.id, n.kind
                )));
            }
        }
        for e in &graph.edges {
            if e.kind != "dependency" {
                return Err(BindingError::Export(format!(
                    "cannot export edge {} -> {} of kind '{}': only dependency edges are in the subset",
                    e.source, e.target, e.kind
                )));
            }
            if !model::DEPENDENCY_STEREOTYPES.contains(&e.label.as_str()) {
                return Err(BindingError::Export(format!(
                    "cannot export dependency edge with label '{}': not a Satisfy/Allocate/Refine/Verify stereotype",
                    e.label
                )));
            }
        }
    }

    let mut out = String::new();
    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    out.push_str(
        "<xmi:XMI xmlns:xmi=\"http://www.omg.org/spec/XMI/20131001\" xmlns:uml=\"http://www.omg.org/spec/UML/20131001\">\n",
    );
    out.push_str(&format!(
        "  <uml:Model xmi:id=\"model\" name=\"{}\">\n",
        escape(&root.project)
    ));
    for el in &root.structure {
        out.push_str(&format!(
            "    <packagedElement xmi:type=\"uml:Class\" xmi:id=\"{}\" name=\"{}\">\n",
            escape(&el.id),
            escape(&el.name)
        ));
        out.push_str(&format!(
            "      <Block base_Class=\"{}\"/>\n",
            escape(&el.id)
        ));
        for (i, a) in el.attributes.iter().enumerate() {
            let type_ref = name_to_id
                .get(a.attr_type.as_str())
                .copied()
                .unwrap_or(a.attr_type.as_str());
            out.push_str(&format!(
                "      <ownedAttribute xmi:type=\"uml:Property\" xmi:id=\"{}-attr-{}\" name=\"{}\" type=\"{}\" aggregation=\"{}\"",
                escape(&el.id),
                i,
                escape(&a.name),
                escape(type_ref),
                escape(&a.aggregation)
            ));
            if !a.default.is_empty() {
                out.push_str(&format!(" default=\"{}\"", escape(&a.default)));
            }
            out.push_str("/>\n");
        }
        if !el.documentation.is_empty() {
            out.push_str(&format!(
                "      <ownedComment xmi:type=\"uml:Comment\" xmi:id=\"{}-doc\" body=\"{}\"/>\n",
                escape(&el.id),
                escape(&el.documentation)
            ));
        }
        out.push_str("    </packagedElement>\n");
    }
    if let Some(graph) = &root.graph {
        for (i, e) in graph.edges.iter().enumerate() {
            out.push_str(&format!(
                "    <packagedElement xmi:type=\"uml:Dependency\" xmi:id=\"dep-{i}\" client=\"{}\" supplier=\"{}\">\n",
                escape(&e.source),
                escape(&e.target)
            ));
            out.push_str(&format!(
                "      <{} base_Dependency=\"dep-{i}\"/>\n",
                escape(&e.label)
            ));
            out.push_str("    </packagedElement>\n");
        }
    }
    out.push_str("  </uml:Model>\n");
    out.push_str("</xmi:XMI>\n");
    Ok(out.into_bytes())
}
