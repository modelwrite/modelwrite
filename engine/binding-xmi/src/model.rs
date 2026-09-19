// SPDX-License-Identifier: AGPL-3.0-or-later
//! The subset of SysML v1 XMI this binding understands, stated as data.
//!
//! This is the binding's mapping matrix: one Mapping per source construct,
//! each marked exact, lossy or unmappable. The table is DECLARATION, not gate
//! input: it is published so a person can read the boundary the binding claims
//! before migrating. The gate does not read it; what the gate enforces is the
//! per-import loss report, whose entries name - per instance - what was actually
//! lost or left unmapped.

use binding::{Mapping, MappingVerdict};

/// The stereotype on a uml:Class that marks a SysML block.
pub const BLOCK_STEREOTYPE: &str = "Block";

/// The stereotype on a uml:Class that marks a SysML requirement.
pub const REQUIREMENT_STEREOTYPE: &str = "Requirement";

/// The stereotypes on a uml:Dependency or uml:Abstraction that this binding
/// carries as graph edges. MagicDraw applies Satisfy/Allocate/Refine/Verify to a
/// uml:Abstraction (base_Abstraction), while hand-written fixtures use a
/// uml:Dependency (base_Dependency).
pub const DEPENDENCY_STEREOTYPES: &[&str] = &["Satisfy", "Allocate", "Refine", "Verify"];

/// The stereotype on a uml:Class that marks a SysML interface block: the type of
/// a proxy/flow port. Carried as a block (kind "block", stereotype
/// "InterfaceBlock") so a port's type resolves to a real graph node, exactly as
/// the golden corpus carries it.
pub const INTERFACEBLOCK_STEREOTYPE: &str = "InterfaceBlock";

/// The stereotype on a uml:Class that marks a SysML constraint block. Carried as
/// a block so its value properties survive; the golden corpus carries it the same
/// way (stereotype "ConstraintBlock").
pub const CONSTRAINTBLOCK_STEREOTYPE: &str = "ConstraintBlock";

/// The property stereotype that marks a SysML part property (a composition of the
/// owning block). Carried as an attribute of the owning block PLUS a graph edge of
/// kind "part" from the owning block to the part's type.
pub const PART_STEREOTYPE: &str = "PartProperty";

/// The property stereotype that marks a SysML reference property. Carried as an
/// attribute of the owning block PLUS a graph edge of kind "reference" from the
/// owning block to the referenced type.
pub const REFERENCE_STEREOTYPE: &str = "ReferenceProperty";

/// Property stereotypes this binding recognises and consumes: their meaning is
/// carried by the property's attribute (name/type/aggregation) or port edge, so
/// the stereotype application itself adds no OKF structure. (PartProperty and
/// ReferenceProperty are NOT here - they additionally produce a graph edge and
/// are tracked.) ProxyPort/FullPort/FlowPort mark a uml:Port, which is carried as
/// an attribute plus a 'part' edge regardless of the specific port kind.
pub const CONSUMED_PROPERTY_STEREOTYPES: &[&str] = &[
    "ValueProperty",
    "FlowProperty",
    "ProxyPort",
    "FullPort",
    "FlowPort",
];

/// The subset this binding understands, stated as data.
///
/// The matrix carries both the positive set (what is understood) and the
/// negative set (what is not), so a reader can see the boundary from the table
/// alone. Anything not listed is reported on import as an Unmappable entry
/// naming the XMI element and its xmi:id - never dropped in silence.
///
/// This table is declaration, not gate input: nothing here is enforced. The gate
/// enforces the per-import loss report instead. Relating the two: a row's
/// `subject` is a CATEGORY (for example `uml:Package (packagedElement)`, naming
/// the construct and its syntactic position), while a loss-report entry's
/// `subject` is an INSTANCE (for example `uml:Package pkg-structure (Structure)`,
/// naming that same construct plus the xmi:id and name of the one occurrence in
/// that artifact). The leading construct name (`uml:Package`) is the shared key:
/// match a report entry to its table row by that prefix, and the row's verdict
/// and note are the general rule the instance entry instantiates.
pub fn mapping_table() -> Vec<Mapping> {
    vec![
        Mapping {
            subject: "uml:Package (packagedElement)".to_string(),
            verdict: MappingVerdict::Lossy,
            note: "OKF has no package/namespace concept; members are promoted to the top-level structure and the package itself (with its xmi:id) is named in the loss report so the flattening is never silent".to_string(),
        },
        Mapping {
            subject: "uml:Class with the Block stereotype".to_string(),
            verdict: MappingVerdict::Exact,
            note: "mapped to a block element carrying its properties and documentation".to_string(),
        },
        Mapping {
            subject: "uml:Class with the Requirement stereotype".to_string(),
            verdict: MappingVerdict::Exact,
            note: "mapped to a requirement element carrying its name, reqId and reqText (MagicDraw applies the stereotype as a sibling <sysml:Requirement base_Class=... Id=... Text=.../> element)".to_string(),
        },
        Mapping {
            subject: "uml:Property with an aggregation".to_string(),
            verdict: MappingVerdict::Exact,
            note: "mapped to a block attribute; aggregation (none/shared/composite) and default carried verbatim (the property's xmi:id has no OKF slot and is named in the loss report)".to_string(),
        },
        Mapping {
            subject: "uml:Port (proxy/flow port on a block)".to_string(),
            verdict: MappingVerdict::Exact,
            note: "a Port IS a Property (UML: Port extends Property): carried as a composite attribute of its owning block typed by its InterfaceBlock, plus a 'part' edge from the owning block to the port type. The port's visibility and isConjugated flag have no OKF slot and are named in the loss report".to_string(),
        },
        Mapping {
            subject: "uml:Class with the InterfaceBlock or ConstraintBlock stereotype".to_string(),
            verdict: MappingVerdict::Exact,
            note: "mapped to a block element (kind 'block') carrying its own stereotype name, so a port's type resolves to a graph node; its properties and documentation are carried as on a Block".to_string(),
        },
        Mapping {
            subject: "uml:Association between two carried blocks".to_string(),
            verdict: MappingVerdict::Exact,
            note: "mapped to a graph edge of kind 'association' from the type of the first memberEnd to the type of the second memberEnd (the ownedEnd), following the golden corpus direction; the association's xmi:id has no OKF slot and is named in the loss report. An association whose ends are not both carried blocks (e.g. Actor/UseCase) is reported Unmappable".to_string(),
        },
        Mapping {
            subject: "uml:Property with a PartProperty or ReferenceProperty stereotype".to_string(),
            verdict: MappingVerdict::Exact,
            note: "carried as the owning block's attribute PLUS a graph edge: PartProperty -> kind 'part', ReferenceProperty -> kind 'reference', both from the owning block to the property's type (the golden corpus direction)".to_string(),
        },
        Mapping {
            subject: "uml:Dependency or uml:Abstraction with a Satisfy/Allocate/Refine/Verify stereotype".to_string(),
            verdict: MappingVerdict::Exact,
            note: "mapped to a graph edge of kind 'dependency' whose label is the stereotype name (the dependency's xmi:id and name have no OKF slot and are named in the loss report)".to_string(),
        },
        Mapping {
            subject: "uml:Comment".to_string(),
            verdict: MappingVerdict::Lossy,
            note: "a body annotating a block is carried as the block's documentation; a body whose annotatedElement is not an emitted block (package, property, dependency, non-block class, dangling id or multi-target list) is named in the loss report; the comment's xmi:id and name have no OKF slot and are named in the loss report".to_string(),
        },
        Mapping {
            subject: "xmi:id on a block (element and graph node)".to_string(),
            verdict: MappingVerdict::Exact,
            note: "carried verbatim as the OKF element id and graph node id".to_string(),
        },
        Mapping {
            subject: "xmi:id on a requirement (element and graph node)".to_string(),
            verdict: MappingVerdict::Exact,
            note: "carried verbatim as the OKF requirement id and graph node id".to_string(),
        },
        Mapping {
            subject: "xmi:id on uml:Model, uml:Package, uml:Property, uml:Port, uml:Association, uml:Dependency, uml:Abstraction or uml:Comment".to_string(),
            verdict: MappingVerdict::Lossy,
            note: "OKF's project, package, Attribute, GraphEdge and documentation have no id slot; each dropped id is named in the loss report".to_string(),
        },
        Mapping {
            subject: "uml:ProfileApplication (declaration)".to_string(),
            verdict: MappingVerdict::Exact,
            note: "a profile application carries no model content; recognised and deliberately not carried, so this is not a loss. Its applied profile is an Eclipse pathmap:// reference that cannot resolve outside the IDE".to_string(),
        },
        Mapping {
            subject: "uml:PackageImport (declaration)".to_string(),
            verdict: MappingVerdict::Exact,
            note: "a package import carries no model content; recognised and deliberately not carried, so this is not a loss. Its imported library is an Eclipse pathmap:// reference that cannot resolve outside the IDE".to_string(),
        },
        Mapping {
            subject: "ecore:EAnnotation (declaration)".to_string(),
            verdict: MappingVerdict::Exact,
            note: "an EMF annotation (and its EPackage reference) carries no model content; recognised and deliberately not carried, so this is not a loss".to_string(),
        },
        Mapping {
            subject: "XMI root metadata, e.g. xmi:version (declaration)".to_string(),
            verdict: MappingVerdict::Exact,
            note: "the schema version and other root metadata carry no model content; recognised and deliberately not carried, so this is not a loss".to_string(),
        },
        Mapping {
            subject: "xmi:Documentation (declaration)".to_string(),
            verdict: MappingVerdict::Exact,
            note: "the xmi:Documentation section records the exporting tool and version (file metadata), not model content; recognised and deliberately not carried, so this is not a loss".to_string(),
        },
        Mapping {
            subject: "xmi:Extension (declaration)".to_string(),
            verdict: MappingVerdict::Exact,
            note: "an xmi:Extension container holds MagicDraw serialization and diagram-representation metadata (plugins, resources, stereotypesHREFS, ownedDiagram geometry) - not model content; recognised and deliberately not carried, so this is not a loss".to_string(),
        },
        Mapping {
            subject: "MagicDraw DiagramInfo stereotype application (declaration)".to_string(),
            verdict: MappingVerdict::Exact,
            note: "a DiagramInfo application carries diagram author/date metadata for a uml:Diagram - hand-placed diagram bookkeeping OKF deliberately does not carry; recognised and deliberately not carried, so this is not a loss".to_string(),
        },
        Mapping {
            subject: "MagicDraw auxiliaryResource stereotype application (declaration)".to_string(),
            verdict: MappingVerdict::Exact,
            note: "an auxiliaryResource application marks an auxiliary file resource (file metadata), not model content; recognised and deliberately not carried, so this is not a loss".to_string(),
        },
        Mapping {
            subject: "Any other XMI element or attribute (e.g. uml:Signal, uml:Generalization, uml:Connector, a foreign-namespaced attribute)".to_string(),
            verdict: MappingVerdict::Unmappable,
            note: "outside the sysml-v1-xmi subset; reported on import as an Unmappable entry naming the element (and its xmi:id) or attribute - never dropped in silence".to_string(),
        },
    ]
}
