// SPDX-License-Identifier: AGPL-3.0-or-later
//! The subset of SysML v1 XMI this binding understands, stated as data.
//!
//! This is the binding's mapping matrix: one Mapping per source construct,
//! each marked exact, lossy or unmappable. The gate and the workbench read this
//! data rather than trusting prose, so the subset a binding claims is auditable.

use binding::{Mapping, MappingVerdict};

/// The stereotype on a uml:Class that marks a SysML block.
pub const BLOCK_STEREOTYPE: &str = "Block";

/// The stereotypes on a uml:Dependency that this binding carries as graph edges.
pub const DEPENDENCY_STEREOTYPES: &[&str] = &["Satisfy", "Allocate", "Refine", "Verify"];

/// The subset this binding understands, stated as data.
///
/// Every construct here is one the reader recognises and maps. Everything that is
/// not listed is reported on import as an Unmappable entry naming the XMI element
/// and its xmi:id - never dropped in silence.
pub fn mapping_table() -> Vec<Mapping> {
    vec![
        Mapping {
            subject: "uml:Package (packagedElement)".to_string(),
            verdict: MappingVerdict::Lossy,
            note: "OKF has no package/namespace concept; members are promoted to the top-level structure and the package itself is named in the loss report so the flattening is never silent".to_string(),
        },
        Mapping {
            subject: "uml:Class with the Block stereotype".to_string(),
            verdict: MappingVerdict::Exact,
            note: "mapped to a block element carrying its properties and documentation".to_string(),
        },
        Mapping {
            subject: "uml:Property with an aggregation".to_string(),
            verdict: MappingVerdict::Exact,
            note: "mapped to a block attribute; aggregation (none/shared/composite) and default carried verbatim".to_string(),
        },
        Mapping {
            subject: "uml:Dependency with a Satisfy/Allocate/Refine/Verify stereotype".to_string(),
            verdict: MappingVerdict::Exact,
            note: "mapped to a graph edge of kind 'dependency' whose label is the stereotype name".to_string(),
        },
        Mapping {
            subject: "uml:Comment".to_string(),
            verdict: MappingVerdict::Exact,
            note: "mapped to the element's documentation".to_string(),
        },
        Mapping {
            subject: "xmi:id".to_string(),
            verdict: MappingVerdict::Exact,
            note: "carried verbatim as the element id".to_string(),
        },
    ]
}
