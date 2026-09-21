// SPDX-License-Identifier: AGPL-3.0-or-later
//! The subset of Capella / Arcadia this binding understands, stated as data.
//!
//! This is the binding's mapping matrix: one [Mapping] per source construct,
//! each marked exact, lossy or unmappable. The table is DECLARATION, not gate
//! input: it is published so a person can read the boundary the binding claims
//! before migrating. The gate does not read it; what the gate enforces is the
//! per-import loss report, whose entries name - per instance - what was actually
//! lost or left unmapped.
//!
//! A row's subject is a CATEGORY (the Capella element type, e.g. "SystemFunction");
//! a loss-report entry's subject is an INSTANCE (the same type plus the element's
//! id and name). The leading construct name is the shared key: match a report
//! entry to its table row by that prefix.

use binding::{Mapping, MappingVerdict};

/// Component element types carried as OKF blocks. In the STPA control structure
/// these are the controllers and the controlled processes.
pub const COMPONENT_TYPES: &[&str] = &[
    "SystemComponent",
    "LogicalComponent",
    "PhysicalComponent",
    "Entity",
];

/// Function element types carried as OKF blocks. In the STPA control structure
/// these are the control actions (and the behaviours the control actions reach).
pub const FUNCTION_TYPES: &[&str] = &[
    "SystemFunction",
    "LogicalFunction",
    "PhysicalFunction",
    "OperationalActivity",
];

/// The element type of a flow between functions (the directed control action /
/// feedback of the control structure), carried as a "dependency" graph edge.
pub const FUNCTIONAL_EXCHANGE_TYPES: &[&str] = &["FunctionalExchange"];

/// The element types of a flow between components, carried as a "dependency"
/// graph edge.
pub const COMPONENT_EXCHANGE_TYPES: &[&str] = &["ComponentExchange", "CommunicationMean"];

/// The allocation that links a component to a function it performs: carried as a
/// "part" graph edge from the component to the function. This is the STPA
/// controller-owns-control-function link.
pub const ALLOCATION_TYPE: &str = "ComponentFunctionalAllocation";

/// Port element types carried as attributes of their owning function/component
/// (the control-interface points of the control structure).
pub const PORT_TYPES: &[&str] = &[
    "FunctionInputPort",
    "FunctionOutputPort",
    "ComponentPort",
    "PhysicalPort",
];

/// A system-level constraint (the STPA system constraint / hazard mitigation),
/// carried as a requirement whose reqText is the constraint's expression body.
pub const CONSTRAINT_TYPE: &str = "Constraint";

/// A classic Capella requirement (re:Requirement), carried as a requirement.
pub const REQUIREMENT_TYPE: &str = "Requirement";

/// Model roots and the Arcadia layer containers: recognised and deliberately not
/// carried (they hold the project and layer names, not model content).
pub const LAYER_TYPES: &[&str] = &[
    "Project",
    "SystemEngineering",
    "OperationalAnalysis",
    "SystemAnalysis",
    "LogicalArchitecture",
    "PhysicalArchitecture",
    "EPBSArchitecture",
];

/// Metadata and bookkeeping types recognised and deliberately not carried (they
/// carry no STPA-relevant model content).
pub const DECLARATION_TYPES: &[&str] = &[
    "ModelInformation",
    "KeyValue",
    "EnumerationPropertyType",
    "EnumerationPropertyLiteral",
    "CompliancyDefinition",
    "CompliancyDefinitionPkg",
    "OpaqueExpression",
    "LiteralNumericValue",
];

/// True when the type names a carried component (a controller / controlled process).
pub fn is_component(t: &str) -> bool {
    COMPONENT_TYPES.contains(&t)
}

/// True when the type names a carried function (a control action).
pub fn is_function(t: &str) -> bool {
    FUNCTION_TYPES.contains(&t)
}

/// True when the type names a recognised-but-not-carried metadata/bookkeeping
/// construct or a layer root. (Ports are handled separately: they are CARRIED as
/// attributes of their owner, so they are not declarations.)
pub fn is_declaration(t: &str) -> bool {
    LAYER_TYPES.contains(&t) || DECLARATION_TYPES.contains(&t)
}

/// True when the type names a package (a namespace whose members are promoted).
pub fn is_package(t: &str) -> bool {
    t.ends_with("Pkg") && !is_declaration(t)
}

/// The subset this binding understands, stated as data. Anything not listed is
/// reported on import as an Unmappable entry naming the element type, id and
/// name - never dropped in silence.
pub fn mapping_table() -> Vec<Mapping> {
    vec![
        Mapping {
            subject: "Project (capellamodeller:Project)".to_string(),
            verdict: MappingVerdict::Exact,
            note: "the project name is carried as the OKF project; the root itself carries no model content and is a declaration".to_string(),
        },
        Mapping {
            subject: "SystemComponent / LogicalComponent / PhysicalComponent / Entity".to_string(),
            verdict: MappingVerdict::Exact,
            note: "mapped to a block (kind 'block') whose stereotype names the Capella type; an Entity with actor=true (or any actor=true element) adds an 'actor' stereotype. Its ports are carried as attributes".to_string(),
        },
        Mapping {
            subject: "SystemFunction / LogicalFunction / PhysicalFunction / OperationalActivity".to_string(),
            verdict: MappingVerdict::Exact,
            note: "mapped to a block (kind 'block') whose stereotype names the Capella type; its input/output ports are carried as attributes; a function nested in another function adds a 'contains' edge".to_string(),
        },
        Mapping {
            subject: "FunctionalExchange".to_string(),
            verdict: MappingVerdict::Lossy,
            note: "mapped to a 'dependency' graph edge from the source function to the target function, labelled with the exchange name. The exchange's own id has no OKF GraphEdge slot and is named; OKF has no flow-kind slot, so the control-action vs feedback distinction is carried only by the directed source->target and the name".to_string(),
        },
        Mapping {
            subject: "ComponentExchange / CommunicationMean".to_string(),
            verdict: MappingVerdict::Lossy,
            note: "mapped to a 'dependency' graph edge from the source component to the target component, labelled with the exchange name. The exchange's id (and its kind, e.g. FLOW) has no OKF slot and is named".to_string(),
        },
        Mapping {
            subject: "ComponentFunctionalAllocation".to_string(),
            verdict: MappingVerdict::Exact,
            note: "mapped to a 'part' graph edge (label 'allocated') from the component to the function it performs: the STPA controller-owns-control-function link. The allocation's id has no OKF slot and is named".to_string(),
        },
        Mapping {
            subject: "FunctionInputPort / FunctionOutputPort / ComponentPort / PhysicalPort".to_string(),
            verdict: MappingVerdict::Lossy,
            note: "carried as an attribute of the owning function/component (name + port type); the port's id has no OKF Attribute slot and is named".to_string(),
        },
        Mapping {
            subject: "Constraint".to_string(),
            verdict: MappingVerdict::Exact,
            note: "mapped to a requirement carrying its name and, as reqText, the text of its OpaqueExpression specification body (the system-level constraint / hazard mitigation)".to_string(),
        },
        Mapping {
            subject: "Requirement (re:Requirement)".to_string(),
            verdict: MappingVerdict::Exact,
            note: "mapped to a requirement carrying its name, id and text".to_string(),
        },
        Mapping {
            subject: "a package (any *Pkg: FunctionPkg, ComponentPkg, DataPkg, InterfacePkg, CapabilityPkg, ...)".to_string(),
            verdict: MappingVerdict::Lossy,
            note: "OKF has no namespace/package concept; members are promoted to the top level and the package itself (id and name) is named in the loss report so the flattening is never silent".to_string(),
        },
        Mapping {
            subject: "the Arcadia layer roots (SystemEngineering, OperationalAnalysis, SystemAnalysis, LogicalArchitecture, PhysicalArchitecture, EPBSArchitecture) and project metadata (ModelInformation, KeyValue, EnumerationPropertyType, OpaqueExpression)".to_string(),
            verdict: MappingVerdict::Exact,
            note: "recognised and deliberately not carried: these hold layer names and bookkeeping, not STPA-relevant model content (a declaration, not a loss)".to_string(),
        },
        Mapping {
            subject: "Any other Capella element (TransfoLink, Part, Property, Class, ExchangeItem, Association, StateMachine/State/Mode/Region/StateTransition, Scenario/SequenceMessage, FunctionalChain*, Capability*/Mission*, the realization/traceability links, REC/RPL CatalogElement*, ConfigurationItem, PhysicalLink, Enumeration, ...)".to_string(),
            verdict: MappingVerdict::Unmappable,
            note: "outside the capella-arcadia subset; reported on import as an Unmappable entry naming the element type, id and name - never dropped in silence".to_string(),
        },
    ]
}
