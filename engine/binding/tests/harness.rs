// SPDX-License-Identifier: AGPL-3.0-or-later
use binding::{
    artifact_hash, round_trip, Binding, BindingError, BindingInfo, Direction, LossReport,
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
        references: Vec::new(),
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

        // The drop is SILENT on purpose: nothing is added to the loss report. A fixture that
        // recorded its own drop could only prove that the harness copies a report it was
        // given - it could not tell the engine's round-trip diff from a fabricated one. A silent
        // dropper is the only shape that can distinguish them, and it is also the shape real
        // bindings have when they are WRONG: an exporter that loses an element does not
        // usually announce it.
        let mappings = Vec::new();
        if let Some(drop_id) = &self.drop_id {
            root.structure.retain(|e| &e.id != drop_id);
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
fn a_silently_lossy_binding_is_caught_by_the_engine_and_not_by_its_own_report() {
    // THE TEST THAT PROVES THE ROUND-TRIP HARNESS DIFFS RATHER THAN BELIEVES.
    //
    // This fixture drops an element and says NOTHING about it. The binding therefore reports a
    // LOSSLESS import, and the only thing that can contradict it is the engine's own diff
    // against the original on the OKF->XMI->OKF journey. If the harness were ever weakened to
    // build its diff from the binding's report - which is the easy, tempting implementation -
    // this test fails on the very first assertion, while a fixture that recorded its own drop
    // would have passed.
    let outcome = round_trip(&dropping(), &source_bytes()).expect("import must succeed");

    assert!(
        outcome.loss_report.is_lossless(),
        "the fixture is silent: its report claims nothing was lost"
    );
    assert!(
        !outcome.diff.equal,
        "the engine must disagree with the binding's own claim"
    );
    assert!(
        outcome
            .diff
            .missing_elements
            .iter()
            .any(|e| e == "structure:LOST_BLOCK"),
        "the engine diff must NAME the silently dropped element, got {:?}",
        outcome.diff.missing_elements
    );
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
