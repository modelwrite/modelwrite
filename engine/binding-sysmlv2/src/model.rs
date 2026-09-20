// SPDX-License-Identifier: AGPL-3.0-or-later
//! The subset of SysML v2 textual notation this binding understands, stated as
//! data.
//!
//! This is the binding's mapping matrix: one [Mapping] per source construct,
//! each marked exact, lossy or unmappable. The table is DECLARATION, not gate
//! input: it is published so a person can read the boundary the binding claims
//! before migrating. The gate does not read it; what the gate enforces is the
//! per-import loss report, whose entries name - per instance - what was actually
//! lost or left unmapped.
//!
//! A row's subject is a CATEGORY (the SysML v2 keyword and its syntactic
//! position); a loss-report entry's subject is an INSTANCE (the same keyword
//! plus the qualified name of the one occurrence). The leading construct name
//! is the shared key: match a report entry to its table row by that prefix.

use binding::{Mapping, MappingVerdict};

/// The stereotype carried on a 'part def' element. OKF node kinds are a closed
/// set (block, actor, requirement, ...) with no partDef, so the SysML v2
/// keyword is preserved as the element's stereotype instead of its kind.
pub const PART_DEF: &str = "partDef";
/// The stereotype carried on an 'attribute def' (value type) element.
pub const ATTRIBUTE_DEF: &str = "attributeDef";
/// The stereotype carried on an 'item def' (item definition) element.
pub const ITEM_DEF: &str = "itemDef";
/// The stereotype carried on an 'abstract' definition.
pub const ABSTRACT_PART_DEF: &str = "abstractPartDef";
/// The stereotype carried on an 'individual' definition or usage.
pub const INDIVIDUAL_PART_DEF: &str = "individualPartDef";

/// The subset this binding understands, stated as data. Anything not listed is
/// reported on import as an Unmappable entry naming the construct and its
/// qualified name - never dropped in silence.
pub fn mapping_table() -> Vec<Mapping> {
    vec![
        Mapping {
            subject: "package".to_string(),
            verdict: MappingVerdict::Lossy,
            note: "the outermost package name becomes the OKF project; a nested package is flattened (its members are promoted to the top-level structure and the package itself is named in the loss report), because OKF has no namespace/package concept".to_string(),
        },
        Mapping {
            subject: "part def / attribute def / item def (definition)".to_string(),
            verdict: MappingVerdict::Exact,
            note: "mapped to a block element (kind 'block') whose stereotype names the SysML v2 keyword (partDef/attributeDef/itemDef); its attributes, parts, items and references are carried as attributes, and its documentation from a doc comment".to_string(),
        },
        Mapping {
            subject: "abstract / individual (definition modifier)".to_string(),
            verdict: MappingVerdict::Exact,
            note: "carried as part of the element's stereotype (abstractPartDef / individualPartDef)".to_string(),
        },
        Mapping {
            subject: "specializes / :> (specialization)".to_string(),
            verdict: MappingVerdict::Exact,
            note: "mapped to a 'generalization' graph edge from the child definition to the base; when the base does not resolve to a carried element it is named in the loss report".to_string(),
        },
        Mapping {
            subject: "attribute (feature of a definition)".to_string(),
            verdict: MappingVerdict::Exact,
            note: "mapped to an attribute carrying name, type and default value; the type's multiplicity [..] is folded into the type string because OKF Attribute has no multiplicity slot".to_string(),
        },
        Mapping {
            subject: "part / item (feature of a definition)".to_string(),
            verdict: MappingVerdict::Exact,
            note: "mapped to a composite attribute of the owning block plus a 'part' graph edge from the owning block to the feature's type".to_string(),
        },
        Mapping {
            subject: "ref / ref item (feature of a definition)".to_string(),
            verdict: MappingVerdict::Exact,
            note: "mapped to an attribute of the owning block plus a 'reference' graph edge from the owning block to the feature's type".to_string(),
        },
        Mapping {
            subject: "in / out (feature direction)".to_string(),
            verdict: MappingVerdict::Lossy,
            note: "the feature is carried but its direction (in/out) is dropped and named in the loss report: OKF Attribute has no direction slot".to_string(),
        },
        Mapping {
            subject: "doc (documentation comment)".to_string(),
            verdict: MappingVerdict::Exact,
            note: "carried as the enclosing element's documentation".to_string(),
        },
        Mapping {
            subject: "requirement <'id'> name { ... }".to_string(),
            verdict: MappingVerdict::Exact,
            note: "mapped to a requirement element carrying its name, reqId, documentation, its subjects (as 'subject' graph edges) and asserted constraints (as attributes whose default holds the constraint text)".to_string(),
        },
        Mapping {
            subject: "subject (requirement subject)".to_string(),
            verdict: MappingVerdict::Exact,
            note: "mapped to a 'subject' graph edge from the requirement to the subject's type; a :>> redefinition subject is carried the same way with the redefinition semantics named in the loss report".to_string(),
        },
        Mapping {
            subject: "assert constraint { ... }".to_string(),
            verdict: MappingVerdict::Lossy,
            note: "the constraint expression text is carried as an attribute named 'constraint' (its default holds the text); the typed ConstraintUsage structure is not reconstructed".to_string(),
        },
        Mapping {
            subject: "satisfy <req> by <subject>".to_string(),
            verdict: MappingVerdict::Exact,
            note: "mapped to a 'dependency' graph edge labelled 'satisfy' from the satisfying subject to the requirement; when either endpoint is outside the carried subset the relationship is named in the loss report".to_string(),
        },
        Mapping {
            subject: "import (declaration)".to_string(),
            verdict: MappingVerdict::Exact,
            note: "an import (of a library package, with or without a wildcard) carries no model content; recognised and deliberately not carried, so this is not a loss".to_string(),
        },
        Mapping {
            subject: "line (//) and block (/* */) comments (trivia)".to_string(),
            verdict: MappingVerdict::Exact,
            note: "comments that are not attached to a doc keyword carry no model content and are skipped".to_string(),
        },
        Mapping {
            subject: "Any other SysML v2 construct (calc, state def, transition, action def, actor, usecase, association, boundary, enum, variation, variant, timeslice, #extension, a top-level part/attribute usage, a :>> redefinition, an unrecognised statement)".to_string(),
            verdict: MappingVerdict::Unmappable,
            note: "outside the sysml-v2-textual subset; reported on import as an Unmappable entry naming the construct and its qualified name - never dropped in silence".to_string(),
        },
    ]
}
