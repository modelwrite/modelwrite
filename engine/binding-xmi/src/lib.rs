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
use okf::types::{
    Attribute, Element, Graph, GraphEdge, GraphNode, OkfRoot, Requirement, StateMachine, Summary,
};

/// The binding identity the gate and the workbench use to select this reader.
pub const BINDING_ID: &str = "sysml-v1-xmi";
/// The SysML v1 version this binding reads.
pub const BINDING_VERSION: &str = "2.4";

const XMI_NS: &str = "http://www.omg.org/spec/XMI/20131001";
const UML_NS: &str = "http://www.omg.org/spec/UML/20131001";

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
                "SysML v1 (UML profile) XMI: blocks, requirements, properties and Satisfy/Allocate traceability"
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

/// True when an attribute is the UML attribute of the given local name: either
/// unqualified (the normal form in these hand-written documents) or in the UML
/// namespace. This is deliberately not namespace-blind, so a foreign attribute
/// that happens to share a local name with a UML one is never read as UML.
fn is_uml_attr(a: &roxmltree::Attribute, name: &str) -> bool {
    a.name() == name && (a.namespace().is_none() || a.namespace() == Some(UML_NS))
}

/// A UML attribute matched by namespace and local name (see [is_uml_attr]).
fn attr<'a, 'input>(node: roxmltree::Node<'a, 'input>, name: &str) -> Option<&'a str> {
    for a in node.attributes() {
        if is_uml_attr(&a, name) {
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

/// The first base_* reference on a stereotype application element, matched only
/// when unqualified so a foreign base_* attribute is never mistaken for it.
fn base_ref<'a, 'input>(node: roxmltree::Node<'a, 'input>) -> Option<&'a str> {
    for a in node.attributes() {
        if a.name().starts_with("base_") && a.namespace().is_none() {
            return Some(a.value());
        }
    }
    None
}

/// An unqualified attribute (MagicDraw writes stereotype properties such as a
/// requirement's Id and Text unqualified); matching only the unqualified form
/// keeps a foreign-namespaced attribute of the same name from being mistaken for it.
fn plain_attr<'a, 'input>(node: roxmltree::Node<'a, 'input>, name: &str) -> Option<&'a str> {
    node.attributes()
        .find(|a| a.name() == name && a.namespace().is_none())
        .map(|a| a.value())
}

/// The xmi:idref carried by a child element of the given local name. MagicDraw
/// serialises a uml:Abstraction's client and supplier as child elements
/// (<client xmi:idref=.../>, <supplier xmi:idref=.../>) rather than attributes,
/// so the reference is read from the child, not the attribute.
fn child_ref<'a, 'input>(node: roxmltree::Node<'a, 'input>, name: &str) -> Option<&'a str> {
    for child in node.children() {
        if child.is_element() && child.tag_name().name() == name {
            if let Some(r) = attr_ns(child, Some(XMI_NS), "idref") {
                return Some(r);
            }
        }
    }
    None
}

/// A human label for an attribute, including its namespace when it is not the
/// (implicit) unqualified form. This keeps a foreign attribute with a colliding
/// local name distinguishable from the UML attribute it shadows.
fn attribute_label(a: &roxmltree::Attribute) -> String {
    match a.namespace() {
        Some(ns) => format!("{} (namespace {ns})", a.name()),
        None => a.name().to_string(),
    }
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
            // XML attribute-value normalisation turns a literal LF/CR/TAB into a
            // space, which would silently corrupt text carrying a real newline (a
            // requirement body, a block's documentation). Encode them as character
            // references so they survive the OKF -> XMI -> OKF round trip byte-exact.
            '\n' => out.push_str("&#10;"),
            '\r' => out.push_str("&#13;"),
            '\t' => out.push_str("&#9;"),
            _ => out.push(c),
        }
    }
    out
}

/// The href reference carried by a declaration element (a profile application or
/// package import), preferring a `pathmap://` URI - the Eclipse-internal
/// reference - over other hrefs when both are present.
fn first_child_href(node: roxmltree::Node<'_, '_>) -> String {
    let mut fallback = String::new();
    for child in node.children() {
        if !child.is_element() {
            continue;
        }
        if let Some(href) = attr(child, "href") {
            if fallback.is_empty() {
                fallback = href.to_string();
            }
            if href.starts_with("pathmap://") {
                return href.to_string();
            }
        }
    }
    fallback
}

/// The note for a recognised declaration, naming the reference it carries and
/// calling out `pathmap://` as unresolvable outside Eclipse.
fn declaration_note(kind: &str, href: &str) -> String {
    let mut note =
        format!("declaration: {kind} recognised and not carried — it carries no model content");
    if !href.is_empty() {
        note.push_str(&format!(" (reference {href})"));
        if href.starts_with("pathmap://") {
            note.push_str("; pathmap:// is Eclipse-internal and cannot resolve outside the IDE");
        }
    }
    note
}

/// The kind of a recognised XMI element, used to decide which attributes are
/// understood (and therefore which are reported as unmapped).
#[derive(Clone, Copy)]
enum ElementKind {
    Model,
    Package,
    Class,
    Property,
    Port,
    Dependency,
    Abstraction,
    Association,
    Comment,
    Stereotype,
    Requirement,
}

struct RawProperty {
    id: String,
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
    /// The UML base element type: "Dependency" (hand-written fixtures) or
    /// "Abstraction" (MagicDraw). Used to name the id-drop in the loss report.
    element: String,
}

/// A uml:Association between two elements, reduced to its ordered member ends.
/// The graph edge's source is the type of member_ends[0] and its target the type
/// of member_ends[1] (the ownedEnd) - the golden corpus direction.
struct RawAssociation {
    id: String,
    name: String,
    member_ends: Vec<String>,
}

/// The SysML requirement properties carried by a <sysml:Requirement> stereotype
/// application: the requirement's own id (reqId) and its body text (reqText).
struct RequirementMeta {
    req_id: String,
    req_text: String,
}

struct RawComment {
    id: String,
    body: String,
    annotated: String,
    enclosing_class: Option<String>,
}

struct Importer {
    info: BindingInfo,
    artifact_hash: String,
    project: String,
    found_model: bool,
    classes: Vec<RawClass>,
    block_ids: HashSet<String>,
    interface_block_ids: HashSet<String>,
    constraint_block_ids: HashSet<String>,
    req_meta: HashMap<String, RequirementMeta>,
    dependencies: Vec<RawDependency>,
    dep_stereotypes: HashMap<String, String>,
    associations: Vec<RawAssociation>,
    packages: Vec<(String, String)>,
    comments: Vec<RawComment>,
    /// property xmi:id -> the stereotype applied to it (PartProperty or
    /// ReferenceProperty), which decides the graph edge emitted for it.
    part_property_ids: HashSet<String>,
    reference_property_ids: HashSet<String>,
    /// property xmi:id of a uml:Port, so it also emits a 'part' edge.
    port_property_ids: HashSet<String>,
    /// property xmi:id -> its type reference, for resolving association member ends.
    property_types: HashMap<String, String>,
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
            interface_block_ids: HashSet::new(),
            constraint_block_ids: HashSet::new(),
            req_meta: HashMap::new(),
            dependencies: Vec::new(),
            dep_stereotypes: HashMap::new(),
            associations: Vec::new(),
            packages: Vec::new(),
            comments: Vec::new(),
            part_property_ids: HashSet::new(),
            reference_property_ids: HashSet::new(),
            port_property_ids: HashSet::new(),
            property_types: HashMap::new(),
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
            self.report_root_attributes(root);
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
            if tag == model::INTERFACEBLOCK_STEREOTYPE {
                if let Some(base) = base_ref(node) {
                    self.interface_block_ids.insert(base.to_string());
                    self.report_unmapped_attributes(
                        node,
                        ElementKind::Stereotype,
                        &format!("InterfaceBlock {id}"),
                    );
                    return;
                }
            }
            if tag == model::CONSTRAINTBLOCK_STEREOTYPE {
                if let Some(base) = base_ref(node) {
                    self.constraint_block_ids.insert(base.to_string());
                    self.report_unmapped_attributes(
                        node,
                        ElementKind::Stereotype,
                        &format!("ConstraintBlock {id}"),
                    );
                    return;
                }
            }
            if tag == model::PART_STEREOTYPE || tag == model::REFERENCE_STEREOTYPE {
                if let Some(base) = base_ref(node) {
                    if tag == model::PART_STEREOTYPE {
                        self.part_property_ids.insert(base.to_string());
                    } else {
                        self.reference_property_ids.insert(base.to_string());
                    }
                    self.report_unmapped_attributes(
                        node,
                        ElementKind::Stereotype,
                        &format!("{tag} {id}"),
                    );
                    return;
                }
            }
            if model::CONSUMED_PROPERTY_STEREOTYPES.contains(&tag.as_str()) {
                if let Some(_base) = base_ref(node) {
                    // ValueProperty/FlowProperty mark the property's SysML role,
                    // which the carried attribute (name/type/aggregation) already
                    // expresses; consumed exactly like the Block stereotype.
                    self.report_unmapped_attributes(
                        node,
                        ElementKind::Stereotype,
                        &format!("{tag} {id}"),
                    );
                    return;
                }
            }
            if tag == "DiagramInfo" || tag == "auxiliaryResource" {
                // DiagramInfo/auxiliaryResource are MagicDraw stereotype
                // applications carrying diagram author/date and auxiliary-file
                // metadata: hand-placed bookkeeping, never model content. OKF
                // deliberately does not carry diagram layout, so these are
                // declarations, not content losses - but still named.
                let kind = if tag == "DiagramInfo" {
                    "diagram-info metadata".to_string()
                } else {
                    "auxiliary-resource metadata".to_string()
                };
                self.losses.push(Mapping {
                    subject: format!("{tag} {id}"),
                    verdict: MappingVerdict::Exact,
                    note: format!("declaration: {kind} recognised and not carried — it carries no model content"),
                });
                return;
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
            if tag == model::REQUIREMENT_STEREOTYPE {
                if let Some(base) = base_ref(node) {
                    let req_id = plain_attr(node, "Id").unwrap_or("").to_string();
                    let req_text = plain_attr(node, "Text").unwrap_or("").to_string();
                    self.req_meta
                        .insert(base.to_string(), RequirementMeta { req_id, req_text });
                    self.report_unmapped_attributes(
                        node,
                        ElementKind::Requirement,
                        &format!("Requirement {id}"),
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
                self.report_root_metadata(node, &id);
                self.report_unmapped_attributes(
                    node,
                    ElementKind::Model,
                    &format!("uml:Model {id}"),
                );
                // CRITICAL 2: the model id has no OKF slot (project carries only
                // the name), so it is named rather than dropped in silence.
                if !id.is_empty() {
                    self.losses.push(Mapping {
                        subject: format!("uml:Model {id}"),
                        verdict: MappingVerdict::Lossy,
                        note: "uml:Model xmi:id dropped: OKF has no project id slot; the name is carried as project".to_string(),
                    });
                }
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
                    .find(|a| is_uml_attr(a, "type"))
                    .map(|a| a.value())
                    .unwrap_or("")
                    .to_string();
                let aggregation = attr(node, "aggregation").unwrap_or("none").to_string();
                let default = attr(node, "default").unwrap_or("").to_string();
                self.property_types.insert(id.clone(), type_ref.clone());
                self.report_unmapped_attributes(
                    node,
                    ElementKind::Property,
                    &format!("uml:Property {id}"),
                );
                match enclosing_class {
                    Some(owner) => {
                        if let Some(c) = self.classes.iter_mut().find(|c| c.id == owner) {
                            c.properties.push(RawProperty {
                                id: id.clone(),
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
            "Port" => {
                // A uml:Port IS a uml:Property (UML: Port extends Property) and
                // MagicDraw serialises a proxy port as <ownedAttribute
                // xmi:type='uml:Port' ... aggregation='composite' type=InterfaceBlock>.
                // It is carried exactly like a property - an attribute of its
                // owning block typed by its InterfaceBlock - and additionally as a
                // 'part' edge (owner -> port type), the golden corpus shape.
                let name = attr(node, "name").unwrap_or("").to_string();
                let type_ref = node
                    .attributes()
                    .find(|a| is_uml_attr(a, "type"))
                    .map(|a| a.value())
                    .unwrap_or("")
                    .to_string();
                let aggregation = attr(node, "aggregation").unwrap_or("none").to_string();
                let default = attr(node, "default").unwrap_or("").to_string();
                self.port_property_ids.insert(id.clone());
                self.property_types.insert(id.clone(), type_ref.clone());
                self.report_unmapped_attributes(node, ElementKind::Port, &format!("uml:Port {id}"));
                match enclosing_class {
                    Some(owner) => {
                        if let Some(c) = self.classes.iter_mut().find(|c| c.id == owner) {
                            c.properties.push(RawProperty {
                                id: id.clone(),
                                name,
                                type_ref,
                                aggregation,
                                default,
                            });
                        }
                    }
                    None => {
                        self.losses.push(Mapping {
                            subject: format!("uml:Port {id}"),
                            verdict: MappingVerdict::Unmappable,
                            note: "uml:Port outside any class".to_string(),
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
                    element: "Dependency".to_string(),
                });
                for child in node.children() {
                    self.walk(child, None);
                }
            }
            "Abstraction" => {
                // MagicDraw applies Satisfy/Allocate/Refine/Verify to a uml:Abstraction
                // whose client/supplier are child elements, not attributes.
                let name = attr(node, "name").unwrap_or("").to_string();
                let client = child_ref(node, "client").unwrap_or("").to_string();
                let supplier = child_ref(node, "supplier").unwrap_or("").to_string();
                self.report_unmapped_attributes(
                    node,
                    ElementKind::Abstraction,
                    &format!("uml:Abstraction {id}"),
                );
                self.dependencies.push(RawDependency {
                    id: id.clone(),
                    name,
                    client,
                    supplier,
                    element: "Abstraction".to_string(),
                });
                // The client/supplier child elements are carried as the edge
                // endpoints; walking them would misreport them as unknown elements.
            }
            "Association" => {
                // A uml:Association is a relationship between two blocks; carried
                // as a graph edge of kind "association". MagicDraw serialises it
                // as two <memberEnd xmi:idref=.../> plus an <ownedEnd
                // xmi:type='uml:Property' type=.../>. Edge direction follows the
                // golden corpus: source = type of the FIRST memberEnd, target =
                // type of the SECOND memberEnd (the ownedEnd).
                let name = attr(node, "name").unwrap_or("").to_string();
                self.report_unmapped_attributes(
                    node,
                    ElementKind::Association,
                    &format!("uml:Association {id}"),
                );
                let mut member_ends: Vec<String> = Vec::new();
                for child in node.children() {
                    if !child.is_element() {
                        continue;
                    }
                    match child.tag_name().name() {
                        "memberEnd" => {
                            if let Some(r) = attr_ns(child, Some(XMI_NS), "idref") {
                                member_ends.push(r.to_string());
                            }
                        }
                        "ownedEnd" => {
                            // The ownedEnd carries the second end's type directly;
                            // record it so the edge can resolve both ends.
                            if let (Some(oid), Some(ty)) =
                                (attr_ns(child, Some(XMI_NS), "id"), attr(child, "type"))
                            {
                                self.property_types.insert(oid.to_string(), ty.to_string());
                            }
                        }
                        _ => {}
                    }
                }
                self.associations.push(RawAssociation {
                    id: id.clone(),
                    name,
                    member_ends,
                });
                // The memberEnd/ownedEnd children are carried as the edge
                // endpoints; walking them would misreport them as unknown elements.
            }
            "Extension" => {
                // An xmi:Extension container holds MagicDraw serialization and
                // diagram-representation metadata (plugins, resources,
                // stereotypesHREFS, ownedDiagram geometry): never model content.
                let id_part = if id.is_empty() {
                    "<no xmi:id>".to_string()
                } else {
                    id.clone()
                };
                self.losses.push(Mapping {
                    subject: format!("Extension {id_part}"),
                    verdict: MappingVerdict::Exact,
                    note: "declaration: xmi:Extension serialization/diagram metadata recognised and not carried — it carries no model content".to_string(),
                });
            }
            "Documentation" => {
                // xmi:Documentation records the exporting tool and version: file
                // metadata, not model content.
                let id_part = if id.is_empty() {
                    "<no xmi:id>".to_string()
                } else {
                    id.clone()
                };
                self.losses.push(Mapping {
                    subject: format!("Documentation {id_part}"),
                    verdict: MappingVerdict::Exact,
                    note: "declaration: xmi:Documentation exporter metadata recognised and not carried — it carries no model content".to_string(),
                });
            }
            "Comment" => {
                let body = attr(node, "body").unwrap_or("").to_string();
                let annotated = attr(node, "annotatedElement").unwrap_or("").to_string();
                self.report_unmapped_attributes(
                    node,
                    ElementKind::Comment,
                    &format!("uml:Comment {id}"),
                );
                // CRITICAL 2: the comment id has no OKF slot (documentation is a
                // bare string), so it is named rather than dropped in silence.
                if !id.is_empty() {
                    self.losses.push(Mapping {
                        subject: format!("uml:Comment {id}"),
                        verdict: MappingVerdict::Lossy,
                        note: "uml:Comment xmi:id dropped: OKF documentation has no id slot"
                            .to_string(),
                    });
                }
                self.comments.push(RawComment {
                    id: id.clone(),
                    body,
                    annotated,
                    enclosing_class: enclosing_class.map(|s| s.to_string()),
                });
            }
            "ProfileApplication" => {
                // A profile application is a DECLARATION, not model content: it
                // applies a profile and carries nothing to migrate. Not carrying
                // it is not a loss, so it is recorded Exact, never blocking.
                let href = first_child_href(node);
                self.losses.push(Mapping {
                    subject: format!("uml:ProfileApplication {id}"),
                    verdict: MappingVerdict::Exact,
                    note: declaration_note("profile application", &href),
                });
            }
            "PackageImport" => {
                // A package import is a DECLARATION, not model content: it
                // imports a library and carries nothing to migrate.
                let href = first_child_href(node);
                self.losses.push(Mapping {
                    subject: format!("uml:PackageImport {id}"),
                    verdict: MappingVerdict::Exact,
                    note: declaration_note("package import", &href),
                });
            }
            "EAnnotation" | "EPackage" => {
                // An EMF annotation (and its EPackage reference) is a
                // DECLARATION: metadata that carries no model content.
                let element_name = type_full.as_deref().unwrap_or(tag.as_str());
                self.losses.push(Mapping {
                    subject: format!("{element_name} {id}"),
                    verdict: MappingVerdict::Exact,
                    note: "declaration: annotation metadata recognised and not carried — it carries no model content".to_string(),
                });
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

    fn report_root_metadata(&mut self, node: roxmltree::Node<'_, '_>, id: &str) {
        // Root metadata (xmi:version and its kin) is a DECLARATION: it describes
        // the serialization, not the model, so not carrying it is not a loss.
        for a in node.attributes() {
            if a.name() == "version" && a.namespace() == Some(XMI_NS) {
                self.losses.push(Mapping {
                    subject: format!("uml:Model {id} attribute '{}'", attribute_label(&a)),
                    verdict: MappingVerdict::Exact,
                    note: "declaration: root metadata (XMI schema version) recognised and not carried — it carries no model content".to_string(),
                });
            }
        }
    }

    fn report_root_attributes(&mut self, node: roxmltree::Node<'_, '_>) {
        let tag = node.tag_name().name().to_string();
        for a in node.attributes() {
            if a.name() == "version" && a.namespace() == Some(XMI_NS) {
                // Root metadata is a DECLARATION, not a loss.
                self.losses.push(Mapping {
                    subject: format!("{tag} root attribute '{}'", attribute_label(&a)),
                    verdict: MappingVerdict::Exact,
                    note: "declaration: root metadata (XMI schema version) recognised and not carried — it carries no model content".to_string(),
                });
            } else {
                self.losses.push(Mapping {
                    subject: format!("{tag} root attribute '{}'", attribute_label(&a)),
                    verdict: MappingVerdict::Unmappable,
                    note: "unmapped attribute on the XMI root element".to_string(),
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
            let uml = ns.is_none() || ns == Some(UML_NS);
            let xmi = ns == Some(XMI_NS);
            let consumed = match kind {
                ElementKind::Model => {
                    (name == "id" && xmi)
                        || (name == "type" && xmi)
                        || (name == "name" && uml)
                        || (name == "version" && xmi) // root metadata, a declaration
                }
                ElementKind::Package | ElementKind::Class => {
                    (name == "id" && xmi) || (name == "type" && xmi) || (name == "name" && uml)
                }
                ElementKind::Property => {
                    (name == "id" && xmi)
                        || (name == "type" && xmi)
                        || (name == "type" && uml)
                        || (name == "name" && uml)
                        || (name == "aggregation" && uml)
                        || (name == "default" && uml)
                        // The association attribute is a back-reference to the
                        // owning uml:Association, which is carried as an edge; it
                        // adds nothing beyond that edge.
                        || (name == "association" && uml)
                }
                ElementKind::Port => {
                    // A Port is a Property; the same attributes are consumed.
                    // visibility and isConjugated have no OKF slot and are named.
                    (name == "id" && xmi)
                        || (name == "type" && xmi)
                        || (name == "type" && uml)
                        || (name == "name" && uml)
                        || (name == "aggregation" && uml)
                        || (name == "default" && uml)
                }
                ElementKind::Association => {
                    // memberEnd/ownedEnd are child elements, not attributes.
                    (name == "id" && xmi) || (name == "type" && xmi) || (name == "name" && uml)
                }
                ElementKind::Dependency => {
                    (name == "id" && xmi)
                        || (name == "type" && xmi)
                        || (name == "name" && uml)
                        || (name == "client" && uml)
                        || (name == "supplier" && uml)
                }
                ElementKind::Abstraction => {
                    // client/supplier are child elements, not attributes; the
                    // only attributes are the element's identity and name.
                    (name == "id" && xmi) || (name == "type" && xmi) || (name == "name" && uml)
                }
                ElementKind::Requirement => {
                    // A <sysml:Requirement> stereotype application carries its
                    // base_Class reference and the requirement's Id and Text.
                    (name == "id" && xmi)
                        || (name.starts_with("base_") && ns.is_none())
                        || (name == "Id" && ns.is_none())
                        || (name == "Text" && ns.is_none())
                }
                ElementKind::Comment => {
                    // NOTE: 'name' is deliberately absent: a uml:Comment name has
                    // no OKF slot and is reported rather than consumed in silence.
                    (name == "id" && xmi)
                        || (name == "type" && xmi)
                        || (name == "body" && uml)
                        || (name == "annotatedElement" && uml)
                }
                ElementKind::Stereotype => {
                    (name == "id" && xmi) || (name.starts_with("base_") && ns.is_none())
                }
            };
            if !consumed {
                self.losses.push(Mapping {
                    subject: format!("{element_name} attribute '{}'", attribute_label(&a)),
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

        // The emitted blocks are the classes carrying the Block, InterfaceBlock
        // or ConstraintBlock stereotype. InterfaceBlock/ConstraintBlock classes are
        // real SysML blocks (a port's type, a constraint block) and are carried as
        // blocks so ports and property types resolve to graph nodes - the golden
        // corpus shape.
        let emitted: HashSet<&str> = self
            .classes
            .iter()
            .filter(|c| {
                self.block_ids.contains(&c.id)
                    || self.interface_block_ids.contains(&c.id)
                    || self.constraint_block_ids.contains(&c.id)
            })
            .map(|c| c.id.as_str())
            .collect();

        // The emitted requirements are the classes carrying the Requirement stereotype.
        let req_class_ids: HashSet<&str> = self.req_meta.keys().map(|k| k.as_str()).collect();

        // A label for every recognised element, so a comment targeting a non-block
        // can say what it targeted rather than just "dangling".
        let mut known: HashMap<&str, String> = HashMap::new();
        for (pid, pname) in &self.packages {
            known.insert(pid.as_str(), format!("uml:Package {pname}"));
        }
        for d in &self.dependencies {
            known.insert(d.id.as_str(), format!("uml:{} {}", d.element, d.name));
        }
        for c in &self.classes {
            if emitted.contains(c.id.as_str()) {
                known.insert(c.id.as_str(), format!("block {}", c.name));
            } else if req_class_ids.contains(c.id.as_str()) {
                known.insert(c.id.as_str(), format!("requirement {}", c.name));
            } else {
                known.insert(c.id.as_str(), format!("uml:Class {} (not a block)", c.name));
            }
            for p in &c.properties {
                known.insert(p.id.as_str(), format!("uml:Property {}", p.name));
            }
        }

        // Resolve each comment: attach its body to every target that is an emitted
        // block, and report every target that is not. A body is never dropped in
        // silence (CRITICAL 1).
        let mut docs: HashMap<String, String> = HashMap::new();
        for comment in &self.comments {
            let targets: Vec<&str> = if !comment.annotated.is_empty() {
                comment.annotated.split_whitespace().collect()
            } else if let Some(owner) = &comment.enclosing_class {
                vec![owner.as_str()]
            } else {
                Vec::new()
            };
            if targets.is_empty() {
                self.losses.push(Mapping {
                    // MINOR: the subject names WHAT is lost (the body), distinct from the
                    // id-drop Lossy entry ("uml:Comment <id>"). Acceptance keys on the subject,
                    // so two different losses must never share one.
                    subject: format!("uml:Comment {} body", comment.id),
                    verdict: MappingVerdict::Unmappable,
                    note: "uml:Comment body dropped: not attached to any element".to_string(),
                });
                continue;
            }
            for target in targets {
                if emitted.contains(target) {
                    docs.entry(target.to_string())
                        .or_default()
                        .push_str(&comment.body);
                } else if let Some(label) = known.get(target) {
                    self.losses.push(Mapping {
                        subject: format!("uml:Comment {} body (target {target})", comment.id),
                        verdict: MappingVerdict::Unmappable,
                        note: format!(
                            "uml:Comment body dropped: annotatedElement {target} ({label}) is not an emitted block"
                        ),
                    });
                } else {
                    self.losses.push(Mapping {
                        subject: format!("uml:Comment {} body (target {target})", comment.id),
                        verdict: MappingVerdict::Unmappable,
                        note: format!(
                            "uml:Comment body dropped: annotatedElement {target} is a dangling id"
                        ),
                    });
                }
            }
        }

        let mut structure: Vec<Element> = Vec::new();
        let mut requirements: Vec<Requirement> = Vec::new();
        for c in &self.classes {
            if emitted.contains(c.id.as_str()) {
                let documentation = docs.get(&c.id).cloned().unwrap_or_default();
                for p in &c.properties {
                    // CRITICAL 2: the property id has no OKF slot (Attribute has
                    // none), so it is named rather than dropped in silence.
                    if !p.id.is_empty() {
                        let construct = if self.port_property_ids.contains(&p.id) {
                            "uml:Port"
                        } else {
                            "uml:Property"
                        };
                        self.losses.push(Mapping {
                            subject: format!("{construct} {}", p.id),
                            verdict: MappingVerdict::Lossy,
                            note: format!(
                                "{construct} xmi:id dropped: OKF Attribute has no id slot"
                            ),
                        });
                    }
                }
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
                let stereotype = if self.interface_block_ids.contains(&c.id) {
                    model::INTERFACEBLOCK_STEREOTYPE
                } else if self.constraint_block_ids.contains(&c.id) {
                    model::CONSTRAINTBLOCK_STEREOTYPE
                } else {
                    model::BLOCK_STEREOTYPE
                };
                structure.push(Element {
                    id: c.id.clone(),
                    name: c.name.clone(),
                    kind: "block".to_string(),
                    stereotypes: vec![stereotype.to_string()],
                    attributes,
                    documentation,
                });
            } else if let Some(meta) = self.req_meta.get(&c.id) {
                // A class carrying the Requirement stereotype is a requirement: its
                // own id (reqId) and body (reqText) come from the stereotype
                // application, while its name and element id come from the class.
                for p in &c.properties {
                    if !p.id.is_empty() {
                        let construct = if self.port_property_ids.contains(&p.id) {
                            "uml:Port"
                        } else {
                            "uml:Property"
                        };
                        self.losses.push(Mapping {
                            subject: format!("{construct} {}", p.id),
                            verdict: MappingVerdict::Lossy,
                            note: format!(
                                "{construct} xmi:id dropped: OKF Attribute has no id slot"
                            ),
                        });
                    }
                }
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
                requirements.push(Requirement {
                    id: c.id.clone(),
                    name: c.name.clone(),
                    kind: "requirement".to_string(),
                    stereotypes: vec![model::REQUIREMENT_STEREOTYPE.to_string()],
                    attributes,
                    documentation: String::new(),
                    req_id: meta.req_id.clone(),
                    req_text: meta.req_text.clone(),
                });
            } else {
                self.losses.push(Mapping {
                    subject: format!("uml:Class {} ({})", c.id, c.name),
                    verdict: MappingVerdict::Unmappable,
                    note: "uml:Class without the Block or Requirement stereotype; not carried"
                        .to_string(),
                });
                // I3: each property of the non-block class is named individually,
                // rather than passed over.
                for p in &c.properties {
                    self.losses.push(Mapping {
                        subject: format!("uml:Property {} ({})", p.id, p.name),
                        verdict: MappingVerdict::Unmappable,
                        note: "property of a non-block uml:Class; not carried".to_string(),
                    });
                }
            }
        }

        let mut graph_nodes: Vec<GraphNode> = structure
            .iter()
            .map(|el| GraphNode {
                id: el.id.clone(),
                kind: "block".to_string(),
                name: el.name.clone(),
                stereotypes: el.stereotypes.clone(),
            })
            .collect();
        // Every requirement is also a graph node, so a Satisfy/Allocate edge can
        // resolve both endpoints (OKF spec: "every element as a node").
        for r in &requirements {
            graph_nodes.push(GraphNode {
                id: r.id.clone(),
                kind: "requirement".to_string(),
                name: r.name.clone(),
                stereotypes: r.stereotypes.clone(),
            });
        }

        let mut graph_edges: Vec<GraphEdge> = Vec::new();
        for d in &self.dependencies {
            if let Some(stereotype) = self.dep_stereotypes.get(&d.id) {
                // CRITICAL 2: the dependency id has no OKF slot (GraphEdge has
                // none), so it is named rather than dropped in silence.
                if !d.id.is_empty() {
                    self.losses.push(Mapping {
                        subject: format!("uml:{} {}", d.element, d.id),
                        verdict: MappingVerdict::Lossy,
                        note: format!(
                            "uml:{} xmi:id dropped: OKF GraphEdge has no id slot",
                            d.element
                        ),
                    });
                }
                // I1: a stereotyped dependency's name is dropped (the edge label
                // carries only the stereotype), so a non-empty name is named.
                if !d.name.is_empty() {
                    self.losses.push(Mapping {
                        subject: format!("uml:{} {} ({})", d.element, d.id, d.name),
                        verdict: MappingVerdict::Lossy,
                        note: format!(
                            "uml:{} name dropped: the graph edge label carries only the stereotype",
                            d.element
                        ),
                    });
                }
                graph_edges.push(GraphEdge {
                    source: d.client.clone(),
                    target: d.supplier.clone(),
                    kind: "dependency".to_string(),
                    label: stereotype.clone(),
                });
            } else {
                self.losses.push(Mapping {
                    subject: format!("uml:{} {} ({})", d.element, d.id, d.name),
                    verdict: MappingVerdict::Unmappable,
                    note: format!(
                        "uml:{} without a recognised Satisfy/Allocate/Refine/Verify stereotype",
                        d.element
                    ),
                });
            }
        }

        // Association edges: source = type of the FIRST memberEnd, target = type
        // of the SECOND memberEnd (the ownedEnd). This is the golden corpus
        // direction: for a block--part association the edge points from the part's
        // type back to the owning block.
        for a in &self.associations {
            if a.member_ends.len() != 2 {
                self.losses.push(Mapping {
                    subject: format!("uml:Association {} ({})", a.id, a.name),
                    verdict: MappingVerdict::Unmappable,
                    note: "uml:Association with other than two member ends; not carried"
                        .to_string(),
                });
                continue;
            }
            let src = self.property_types.get(&a.member_ends[0]);
            let tgt = self.property_types.get(&a.member_ends[1]);
            let (s, t) = match (src, tgt) {
                (Some(s), Some(t)) => (s, t),
                _ => {
                    self.losses.push(Mapping {
                        subject: format!("uml:Association {}", a.id),
                        verdict: MappingVerdict::Unmappable,
                        note: "uml:Association with an unresolvable member end".to_string(),
                    });
                    continue;
                }
            };
            let s_carried = emitted.contains(s.as_str()) || req_class_ids.contains(s.as_str());
            let t_carried = emitted.contains(t.as_str()) || req_class_ids.contains(t.as_str());
            if !s_carried || !t_carried {
                self.losses.push(Mapping {
                    subject: format!("uml:Association {}", a.id),
                    verdict: MappingVerdict::Unmappable,
                    note: "uml:Association between elements outside the carried subset (an end is not a carried block or requirement)".to_string(),
                });
                continue;
            }
            // CRITICAL 2: the association id has no OKF slot (GraphEdge has none).
            if !a.id.is_empty() {
                self.losses.push(Mapping {
                    subject: format!("uml:Association {}", a.id),
                    verdict: MappingVerdict::Lossy,
                    note: "uml:Association xmi:id dropped: OKF GraphEdge has no id slot"
                        .to_string(),
                });
            }
            graph_edges.push(GraphEdge {
                source: s.clone(),
                target: t.clone(),
                kind: "association".to_string(),
                label: String::new(),
            });
        }

        // Part/reference edges: a PartProperty (or a port) and a ReferenceProperty
        // are carried as the owning block's attribute PLUS a graph edge from the
        // owning block to the property's type - the golden corpus direction
        // (owner --part/reference--> type).
        for c in &self.classes {
            if !emitted.contains(c.id.as_str()) {
                continue;
            }
            for p in &c.properties {
                let kind = if self.part_property_ids.contains(&p.id)
                    || self.port_property_ids.contains(&p.id)
                {
                    "part"
                } else if self.reference_property_ids.contains(&p.id) {
                    "reference"
                } else {
                    continue;
                };
                if p.type_ref.is_empty() {
                    continue;
                }
                graph_edges.push(GraphEdge {
                    source: c.id.clone(),
                    target: p.type_ref.clone(),
                    kind: kind.to_string(),
                    label: String::new(),
                });
            }
        }

        for (pid, pname) in &self.packages {
            self.losses.push(Mapping {
                subject: format!("uml:Package {pid} ({pname})"),
                verdict: MappingVerdict::Lossy,
                note: "OKF has no package/namespace concept; members are promoted to the top-level structure and the package (with its xmi:id) is dropped"
                    .to_string(),
            });
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
            // I3: OKF requires the stateMachine section to be present even when the model
            // has no state machine. The binding emits the empty section itself, so the document
            // it produces passes OKF validation UNCHANGED - the measured document is the committed
            // document, never mutated afterwards.
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
        // I2: the export carries exactly one recognised block stereotype (Block,
        // InterfaceBlock or ConstraintBlock). Any other stereotype set would be
        // silently normalised on the way out, so refuse rather than normalise.
        let block_stereotypes = [
            model::BLOCK_STEREOTYPE,
            model::INTERFACEBLOCK_STEREOTYPE,
            model::CONSTRAINTBLOCK_STEREOTYPE,
        ];
        if el.stereotypes.len() != 1 || !block_stereotypes.contains(&el.stereotypes[0].as_str()) {
            return Err(BindingError::Export(format!(
                "cannot export structure element {}: stereotypes {:?} are outside the subset (only {:?} is carried)",
                el.id,
                el.stereotypes,
                block_stereotypes
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
    for r in &root.requirements {
        if r.kind != "requirement" {
            return Err(BindingError::Export(format!(
                "cannot export requirement {} of kind '{}': only requirements are in the subset",
                r.id, r.kind
            )));
        }
        // I2: the export carries exactly one stereotype (Requirement). Any other
        // stereotype set would be silently normalised on the way out, so refuse.
        if r.stereotypes.len() != 1 || r.stereotypes[0] != model::REQUIREMENT_STEREOTYPE {
            return Err(BindingError::Export(format!(
                "cannot export requirement {}: stereotypes {:?} are outside the subset (only {:?} is carried)",
                r.id,
                r.stereotypes,
                [model::REQUIREMENT_STEREOTYPE]
            )));
        }
    }
    if !root.activities.is_empty() {
        return Err(BindingError::Export(
            "cannot export activities: outside the sysml-v1-xmi subset".to_string(),
        ));
    }
    // I3: an EMPTY state machine is the binding's own output (OKF requires the section even
    // when there is no state machine to describe), so export must accept it rather than refuse
    // the subset it just emitted. A state machine WITH states is genuinely outside the subset
    // and is still refused.
    if let Some(sm) = &root.state_machine {
        if sm.regions.iter().any(|region| !region.states.is_empty()) {
            return Err(BindingError::Export(
                "cannot export a state machine with states: outside the sysml-v1-xmi subset"
                    .to_string(),
            ));
        }
    }

    // name_to_id maps an element NAME to its id and is used only for the type of a
    // NON-part/reference attribute, where a duplicate name cannot corrupt the round
    // trip: the attribute stores the name, and re-import resolves the written id back
    // to that same name. Part/reference attributes are matched by name via id_to_name
    // and written with the edge's precise target id (see the block-emission loop).
    let mut name_to_id: HashMap<&str, &str> = HashMap::new();
    let mut id_to_name: HashMap<&str, &str> = HashMap::new();
    for el in &root.structure {
        name_to_id.insert(el.name.as_str(), el.id.as_str());
        id_to_name.insert(el.id.as_str(), el.name.as_str());
    }
    for r in &root.requirements {
        name_to_id.insert(r.name.as_str(), r.id.as_str());
        id_to_name.insert(r.id.as_str(), r.name.as_str());
    }

    // Part/reference edges are reconstructed during block emission: each one
    // becomes a PartProperty/ReferenceProperty stereotype application on the
    // source block's attribute typed by the target. Each value is the list of
    // target ids still to be reconstructed for that source block, so export
    // refuses rather than silently drop an edge it cannot rebuild. Matching is
    // by the target's NAME (see below): a global name->id map is ambiguous once
    // two carried elements share a name, which a real vendor model does freely.
    let mut part_edges: HashMap<String, Vec<String>> = HashMap::new();
    let mut reference_edges: HashMap<String, Vec<String>> = HashMap::new();

    if let Some(graph) = &root.graph {
        // The graph must mirror the structure blocks followed by the requirements,
        // exactly and in order, or export would silently lose or reorder them.
        let mut expected: Vec<(&str, &str, &str, &[String])> = Vec::new();
        for e in &root.structure {
            expected.push((
                e.id.as_str(),
                e.kind.as_str(),
                e.name.as_str(),
                e.stereotypes.as_slice(),
            ));
        }
        for r in &root.requirements {
            expected.push((
                r.id.as_str(),
                r.kind.as_str(),
                r.name.as_str(),
                r.stereotypes.as_slice(),
            ));
        }
        let actual: Vec<&str> = graph.nodes.iter().map(|n| n.id.as_str()).collect();
        let expected_ids: Vec<&str> = expected.iter().map(|(id, _, _, _)| *id).collect();
        if actual != expected_ids {
            return Err(BindingError::Export(
                "graph nodes do not mirror the structure blocks and requirements; export would silently lose them"
                    .to_string(),
            ));
        }
        // I2: a graph node must mirror its element exactly - kind, name and
        // stereotypes - or export would silently normalise it.
        for (n, (_, kind, name, stereotypes)) in graph.nodes.iter().zip(expected.iter()) {
            if n.kind.as_str() != *kind {
                return Err(BindingError::Export(format!(
                    "cannot export graph node {} of kind '{}': expected kind '{}'",
                    n.id, n.kind, kind
                )));
            }
            if n.name.as_str() != *name {
                return Err(BindingError::Export(format!(
                    "cannot export graph node {}: name '{}' does not match its element name '{}'",
                    n.id, n.name, name
                )));
            }
            if n.stereotypes.as_slice() != *stereotypes {
                return Err(BindingError::Export(format!(
                    "cannot export graph node {}: stereotypes {:?} do not match its element stereotypes {:?}",
                    n.id, n.stereotypes, stereotypes
                )));
            }
        }
        // Edge endpoints must resolve to carried nodes, and part/reference
        // edges must be reconstructible from a block attribute - export refuses
        // rather than silently dropping an edge it cannot rebuild.
        let node_ids: HashSet<&str> = graph.nodes.iter().map(|n| n.id.as_str()).collect();
        let block_ids: HashSet<&str> = root.structure.iter().map(|el| el.id.as_str()).collect();
        for e in &graph.edges {
            match e.kind.as_str() {
                "dependency" => {
                    if !model::DEPENDENCY_STEREOTYPES.contains(&e.label.as_str()) {
                        return Err(BindingError::Export(format!(
                            "cannot export dependency edge with label '{}': not a Satisfy/Allocate/Refine/Verify stereotype",
                            e.label
                        )));
                    }
                }
                "association" => {
                    if !node_ids.contains(e.source.as_str())
                        || !node_ids.contains(e.target.as_str())
                    {
                        return Err(BindingError::Export(format!(
                            "cannot export association edge {} -> {}: an endpoint is not a carried block or requirement",
                            e.source, e.target
                        )));
                    }
                }
                "part" | "reference" => {
                    if !block_ids.contains(e.source.as_str())
                        || !node_ids.contains(e.target.as_str())
                    {
                        return Err(BindingError::Export(format!(
                            "cannot export {} edge {} -> {}: source must be a block and target a carried element",
                            e.kind, e.source, e.target
                        )));
                    }
                    if e.kind == "part" {
                        part_edges
                            .entry(e.source.clone())
                            .or_default()
                            .push(e.target.clone());
                    } else {
                        reference_edges
                            .entry(e.source.clone())
                            .or_default()
                            .push(e.target.clone());
                    }
                }
                other => {
                    return Err(BindingError::Export(format!(
                        "cannot export edge {} -> {} of kind '{}': outside the subset",
                        e.source, e.target, other
                    )));
                }
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
            "      <{} base_Class=\"{}\"/>\n",
            el.stereotypes[0],
            escape(&el.id)
        ));
        for (i, a) in el.attributes.iter().enumerate() {
            // A part/reference attribute carries its type as the TARGET NAME (the
            // import resolved the id to a name for the OKF "type" field), while the
            // edge holds the precise target id. Match the attribute to a pending edge
            // by name, then write the edge precise id as the type - a global
            // name->id map would pick the wrong id once two elements share a name.
            let mut type_ref = name_to_id
                .get(a.attr_type.as_str())
                .copied()
                .unwrap_or(a.attr_type.as_str())
                .to_string();
            let mut applied: Option<&str> = None;
            if let Some(pending) = part_edges.get_mut(&el.id) {
                if let Some(pos) = pending
                    .iter()
                    .position(|t| id_to_name.get(t.as_str()).copied() == Some(a.attr_type.as_str()))
                {
                    type_ref = pending.remove(pos);
                    applied = Some(model::PART_STEREOTYPE);
                }
            }
            if applied.is_none() {
                if let Some(pending) = reference_edges.get_mut(&el.id) {
                    if let Some(pos) = pending.iter().position(|t| {
                        id_to_name.get(t.as_str()).copied() == Some(a.attr_type.as_str())
                    }) {
                        type_ref = pending.remove(pos);
                        applied = Some(model::REFERENCE_STEREOTYPE);
                    }
                }
            }
            let prop_id = format!("{}-attr-{}", el.id, i);
            out.push_str(&format!(
                "      <ownedAttribute xmi:type=\"uml:Property\" xmi:id=\"{}\" name=\"{}\" type=\"{}\" aggregation=\"{}\"",
                escape(&prop_id),
                escape(&a.name),
                escape(&type_ref),
                escape(&a.aggregation)
            ));
            if !a.default.is_empty() {
                out.push_str(&format!(" default=\"{}\"", escape(&a.default)));
            }
            out.push_str("/>\n");
            if let Some(stereotype) = applied {
                out.push_str(&format!(
                    "      <{} xmi:id=\"{}-app\" base_Property=\"{}\"/>\n",
                    stereotype,
                    escape(&prop_id),
                    escape(&prop_id)
                ));
            }
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
    // Every part/reference edge must now have been rebuilt as a stereotype
    // application on a matching attribute; a remainder means the edge's target
    // has no attribute to carry it, which export refuses rather than drops.
    if part_edges.values().any(|v| !v.is_empty()) || reference_edges.values().any(|v| !v.is_empty())
    {
        return Err(BindingError::Export(
            "cannot reconstruct a part/reference edge: no block attribute is typed by its target"
                .to_string(),
        ));
    }
    for r in &root.requirements {
        out.push_str(&format!(
            "    <packagedElement xmi:type=\"uml:Class\" xmi:id=\"{}\" name=\"{}\">\n",
            escape(&r.id),
            escape(&r.name)
        ));
        out.push_str(&format!(
            "      <Requirement base_Class=\"{}\"",
            escape(&r.id)
        ));
        if !r.req_id.is_empty() {
            out.push_str(&format!(" Id=\"{}\"", escape(&r.req_id)));
        }
        if !r.req_text.is_empty() {
            out.push_str(&format!(" Text=\"{}\"", escape(&r.req_text)));
        }
        out.push_str("/>\n");
        for (i, a) in r.attributes.iter().enumerate() {
            let type_ref = name_to_id
                .get(a.attr_type.as_str())
                .copied()
                .unwrap_or(a.attr_type.as_str());
            out.push_str(&format!(
                "      <ownedAttribute xmi:type=\"uml:Property\" xmi:id=\"{}-attr-{}\" name=\"{}\" type=\"{}\" aggregation=\"{}\"",
                escape(&r.id),
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
        if !r.documentation.is_empty() {
            out.push_str(&format!(
                "      <ownedComment xmi:type=\"uml:Comment\" xmi:id=\"{}-doc\" body=\"{}\"/>\n",
                escape(&r.id),
                escape(&r.documentation)
            ));
        }
        out.push_str("    </packagedElement>\n");
    }
    if let Some(graph) = &root.graph {
        for (i, e) in graph.edges.iter().enumerate() {
            match e.kind.as_str() {
                "dependency" => {
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
                "association" => {
                    // A self-contained association: the two member ends are carried
                    // as ownedEnd properties typed by the edge's endpoints, so the
                    // re-import rebuilds source = type of end 0, target = type of
                    // end 1 (the golden corpus direction).
                    out.push_str(&format!(
                        "    <packagedElement xmi:type=\"uml:Association\" xmi:id=\"assoc-{i}\">\n"
                    ));
                    out.push_str(&format!(
                        "      <memberEnd xmi:idref=\"assoc-{i}-end-0\"/>\n"
                    ));
                    out.push_str(&format!(
                        "      <memberEnd xmi:idref=\"assoc-{i}-end-1\"/>\n"
                    ));
                    out.push_str(&format!(
                        "      <ownedEnd xmi:type=\"uml:Property\" xmi:id=\"assoc-{i}-end-0\" type=\"{}\"/>\n",
                        escape(&e.source)
                    ));
                    out.push_str(&format!(
                        "      <ownedEnd xmi:type=\"uml:Property\" xmi:id=\"assoc-{i}-end-1\" type=\"{}\"/>\n",
                        escape(&e.target)
                    ));
                    out.push_str("    </packagedElement>\n");
                }
                "part" | "reference" => {
                    // Already emitted as a PartProperty/ReferenceProperty stereotype
                    // application on the source block's attribute.
                }
                other => {
                    return Err(BindingError::Export(format!(
                        "cannot export edge {} -> {} of kind '{}': outside the subset",
                        e.source, e.target, other
                    )));
                }
            }
        }
    }
    out.push_str("  </uml:Model>\n");
    out.push_str("</xmi:XMI>\n");
    Ok(out.into_bytes())
}
