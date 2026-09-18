// SPDX-License-Identifier: AGPL-3.0-or-later
use binding::{
    artifact_hash, round_trip, Binding, BindingError, BindingInfo, Direction, LossReport, Mapping,
    MappingVerdict,
};
use okf::types::{Element, Graph, GraphNode, OkfRoot, Summary};

fn element(id: &str) -> Element {
    Element {
        id: id.to_string(),
        name: id.to_string(),
        kind: "block".to_string(),
        stereotypes: Vec::new(),
        attributes: Vec::new(),
        documentation: String::new(),
    }
}

fn node(id: &str) -> GraphNode {
    GraphNode {
        id: id.to_string(),
        kind: "block".to_string(),
        name: id.to_string(),
        stereotypes: Vec::new(),
    }
}

fn source_document() -> OkfRoot {
    OkfRoot {
        okf: "1.0".to_string(),
        project: "harness".to_string(),
        exported_at: String::new(),
        summary: Summary::default(),
        structure: vec![element("BLOCK_A"), element("LOST_BLOCK")],
        interfaces: Vec::new(),
        signals: Vec::new(),
        requirements: Vec::new(),
        state_machine: None,
        activities: Vec::new(),
        graph: Some(Graph {
            nodes: vec![node("BLOCK_A"), node("LOST_BLOCK")],
            edges: Vec::new(),
        }),
        provenance: None,
    }
}

fn source_bytes() -> Vec<u8> {
    serde_json::to_vec(&source_document()).expect("fixture must serialize")
}

struct JsonBinding {
    info: BindingInfo,
    drop_id: Option<String>,
    fail_export: bool,
}

impl Binding for JsonBinding {
    fn info(&self) -> BindingInfo {
        self.info.clone()
    }

    fn import(&self, source: &[u8]) -> Result<(OkfRoot, LossReport), BindingError> {
        let mut root: OkfRoot =
            serde_json::from_slice(source).map_err(|e| BindingError::Import(e.to_string()))?;

        let mut mappings = Vec::new();
        if let Some(drop_id) = &self.drop_id {
            let before = root.structure.len();
            root.structure.retain(|e| &e.id != drop_id);
            if root.structure.len() < before {
                mappings.push(Mapping {
                    subject: format!("structure:{}", drop_id),
                    verdict: MappingVerdict::Unmappable,
                    note: "dropped on import by the test binding".to_string(),
                });
            }
        }

        Ok((
            root,
            LossReport {
                binding: self.info.clone(),
                mappings,
                artifact_hash: artifact_hash(source),
            },
        ))
    }

    fn export(&self, root: &OkfRoot) -> Result<Vec<u8>, BindingError> {
        if self.fail_export {
            return Err(BindingError::Export(
                "test binding refuses to export".to_string(),
            ));
        }
        serde_json::to_vec(root).map_err(|e| BindingError::Export(e.to_string()))
    }
}

fn lossless() -> JsonBinding {
    JsonBinding {
        info: BindingInfo {
            id: "test-json".to_string(),
            version: "1.0".to_string(),
            direction: Direction::ImportAndExport,
            description: "lossless JSON passthrough".to_string(),
        },
        drop_id: None,
        fail_export: false,
    }
}

fn dropping() -> JsonBinding {
    JsonBinding {
        info: BindingInfo {
            id: "test-json-broken".to_string(),
            version: "1.0".to_string(),
            direction: Direction::ImportAndExport,
            description: "drops an element on import".to_string(),
        },
        drop_id: Some("LOST_BLOCK".to_string()),
        fail_export: false,
    }
}

fn lying() -> JsonBinding {
    JsonBinding {
        info: BindingInfo {
            id: "test-json-lying".to_string(),
            version: "1.0".to_string(),
            direction: Direction::ImportAndExport,
            description: "claims export but fails".to_string(),
        },
        drop_id: None,
        fail_export: true,
    }
}

fn viewer() -> JsonBinding {
    JsonBinding {
        info: BindingInfo {
            id: "test-json-viewer".to_string(),
            version: "1.0".to_string(),
            direction: Direction::ImportOnly,
            description: "reader only".to_string(),
        },
        drop_id: None,
        fail_export: false,
    }
}

#[test]
fn a_lossless_binding_reports_a_lossless_result() {
    let outcome = round_trip(&lossless(), &source_bytes()).expect("lossless round trip");
    assert!(outcome.loss_report.is_lossless());
    assert!(outcome.loss_report.blocking().is_empty());
    assert!(outcome.diff.equal);
    assert!(outcome.diff.missing_elements.is_empty());
    assert_eq!(outcome.artifact_hash, artifact_hash(&source_bytes()));
}

#[test]
fn a_binding_that_drops_an_element_on_import_is_named_by_the_engine_diff() {
    let outcome = round_trip(&dropping(), &source_bytes()).expect("import must succeed");
    assert!(
        outcome
            .diff
            .missing_elements
            .iter()
            .any(|e| e == "structure:LOST_BLOCK"),
        "the engine diff must name the dropped element, got {:?}",
        outcome.diff.missing_elements
    );
    assert!(!outcome.diff.equal);
    // No mapping is ever silent: the binding's own report names it too.
    assert!(!outcome.loss_report.is_lossless());
    assert!(outcome
        .loss_report
        .blocking()
        .iter()
        .any(|m| m.subject == "structure:LOST_BLOCK"));
}

#[test]
fn a_binding_that_claims_export_but_fails_is_rejected() {
    let err = round_trip(&lying(), &source_bytes()).expect_err("export failure must be rejected");
    match err {
        BindingError::Export(_) => {}
        other => panic!("expected Export error, got {:?}", other),
    }
}

#[test]
fn a_viewer_is_refused_a_round_trip() {
    let err = round_trip(&viewer(), &source_bytes()).expect_err("a viewer cannot round-trip");
    match err {
        BindingError::Viewer { id } => assert_eq!(id, "test-json-viewer"),
        other => panic!("expected Viewer error, got {:?}", other),
    }
}

#[test]
fn artifact_hash_is_a_stable_content_address() {
    let bytes = source_bytes();
    let a = artifact_hash(&bytes);
    let b = artifact_hash(&bytes);
    assert_eq!(a, b);
    assert_eq!(a.len(), 64);
    assert_ne!(a, artifact_hash(b"different"));
}
