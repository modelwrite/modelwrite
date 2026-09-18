// SPDX-License-Identifier: AGPL-3.0-or-later
//! Pins what the mapping table is for: it is DECLARATION, not gate input.
//!
//! The trait's doc states two things: (1) `mapping_table` is a published boundary a
//! person reads before migrating, not a thing the gate reads; and (2) the gate enforces
//! the per-import `LossReport`, whose blocking entries name, per instance, what was
//! actually lost. This test holds the first claim to the code: two bindings that differ
//! ONLY in their declared table must produce the SAME loss report, so the declaration
//! cannot be what import behaves on.

use binding::{Binding, BindingError, BindingInfo, Direction, LossReport, Mapping, MappingVerdict};
use okf::types::OkfRoot;

const SOURCE: &str = r#"{
  "okf": "1.0",
  "project": "declaration",
  "summary": {},
  "structure": [],
  "interfaces": [],
  "signals": [],
  "requirements": [],
  "activities": [],
  "graph": {"nodes": [], "edges": []}
}"#;

/// A binding whose declared table is configurable, and whose import always reports one
/// Lossy drop regardless of what it declared. The declaration and the report are
/// independent by construction, so a test can hold the two apart.
struct DeclaringBinding {
    declared: Vec<Mapping>,
}

impl Binding for DeclaringBinding {
    fn info(&self) -> BindingInfo {
        BindingInfo {
            id: "declaring".to_string(),
            version: "1.0".to_string(),
            direction: Direction::ImportAndExport,
            description: "declares a configurable table".to_string(),
        }
    }

    fn mapping_table(&self) -> Vec<Mapping> {
        self.declared.clone()
    }

    fn import(&self, source: &[u8]) -> Result<(OkfRoot, LossReport), BindingError> {
        let root: OkfRoot =
            serde_json::from_slice(source).map_err(|e| BindingError::Import(e.to_string()))?;
        let mappings = vec![Mapping {
            subject: "uml:Model model-1".to_string(),
            verdict: MappingVerdict::Lossy,
            note: "an id OKF has no slot for".to_string(),
        }];
        Ok((
            root,
            LossReport {
                binding: self.info(),
                mappings,
                artifact_hash: binding::artifact_hash(source),
            },
        ))
    }

    fn export(&self, root: &OkfRoot) -> Result<Vec<u8>, BindingError> {
        serde_json::to_vec(root).map_err(|e| BindingError::Export(e.to_string()))
    }
}

#[test]
fn the_mapping_table_is_declaration_and_does_not_change_import_behaviour() {
    let bytes = SOURCE.as_bytes();

    // One binding declares nothing; the other declares a row. They differ in NOTHING else.
    let silent = DeclaringBinding {
        declared: Vec::new(),
    };
    let declared = DeclaringBinding {
        declared: vec![Mapping {
            subject: "uml:Model".to_string(),
            verdict: MappingVerdict::Lossy,
            note: "declared, not enforced".to_string(),
        }],
    };

    let (_, silent_report) = silent.import(bytes).expect("import succeeds");
    let (_, declared_report) = declared.import(bytes).expect("import succeeds");

    // The declaration does not reach import: the two loss reports are identical, so the
    // table cannot be what the gate acts on. A binding that declares nothing still imports
    // and still reports its loss - the declaration has no effect on import behaviour.
    assert_eq!(silent_report, declared_report);
    assert!(!silent_report.blocking().is_empty());

    // The table is still visible to a reader, whatever it declares.
    assert!(silent.mapping_table().is_empty());
    assert_eq!(declared.mapping_table().len(), 1);
}
