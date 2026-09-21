// SPDX-License-Identifier: AGPL-3.0-or-later
//! The SysML v2 textual-notation binding wired into the server as a VIEWER (ImportOnly).
//!
//! Three facts are pinned here, at the server's import surface:
//!   1. The registry lists sysml-v2-textual@1.0 with Direction::ImportOnly, so the workbench
//!      import page offers it and the generated binding list on the site is TRUE.
//!   2. Importing a real .sysml file returns OKF plus its NAMED loss report: the subset
//!      boundaries (a top-level part usage, a SYSMOD #derivation extension, :>> redefinition
//!      subjects, an unsatisfiable satisfy) come back as Unmappable entries, and a viewer
//!      import records `fidelity: null` (no round trip was measured) rather than a fake
//!      round-trip diff.
//!   3. An export or round trip through the viewer is REFUSED with the direction named,
//!      never a panic or a 500.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use binding::Direction;
use http_body_util::BodyExt;
use server::store::Store;
use tower::ServiceExt;

fn state(dir: &std::path::Path) -> server::AppState {
    let store = server::store::sqlite::SqliteStore::open(&dir.join("mw.db")).unwrap();
    server::AppState {
        store: std::sync::Arc::new(store),
        evidence_dir: dir.to_path_buf(),
        auth: server::auth::AuthConfig::Open,
    }
}

fn post(uri: &str, body: serde_json::Value) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

async fn json_body(response: axum::response::Response) -> serde_json::Value {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

/// The committed fixture: the smallest real GfSE community model, copied byte-for-byte from
/// sample/examples/sysml-v2/gfse-models/models/SE_Models/Drone_BaseArchitecture.sysml
/// (BSD 3-Clause, Gesellschaft für Systems Engineering e.V., 2024 - see fixtures/README.md).
fn fixture() -> String {
    std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/Drone_BaseArchitecture.sysml"
    ))
    .expect("fixture must exist")
}

async fn create_project(router: &axum::Router, name: &str) {
    let response = router
        .clone()
        .oneshot(post("/projects", serde_json::json!({ "name": name })))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
}

#[test]
fn the_sysml_v2_viewer_is_in_the_registry_list() {
    let bindings = server::binding_registry::bindings();
    let viewer = bindings
        .iter()
        .find(|b| b.id == "sysml-v2-textual")
        .expect("sysml-v2-textual must be registered");
    assert_eq!(viewer.version, "1.0");
    assert_eq!(viewer.direction, Direction::ImportOnly);
    assert!(
        viewer.description.contains(".sysml"),
        "the description names the notation, got: {}",
        viewer.description
    );
}

#[tokio::test]
async fn importing_a_real_sysml_file_returns_okf_and_named_losses() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    create_project(&router, "drone").await;

    let artifact = fixture();

    // First import without acceptance: the viewer's blocking losses come back BY NAME.
    let refused = router
        .clone()
        .oneshot(post(
            "/projects/drone/import",
            serde_json::json!({
                "binding": "sysml-v2-textual@1.0",
                "branch": "main",
                "author": "alex",
                "message": "import drone",
                "artifact": artifact
            }),
        ))
        .await
        .unwrap();
    assert_eq!(
        refused.status(),
        StatusCode::UNPROCESSABLE_ENTITY,
        "{:?}",
        refused
    );
    let refused_body = json_body(refused).await;
    let blocking = refused_body["blocking"].as_array().expect("blocking array");
    let subjects: Vec<&str> = blocking
        .iter()
        .map(|m| m["subject"].as_str().unwrap())
        .collect();
    assert!(
        subjects.iter().any(|s| s.starts_with("part drone")),
        "the top-level part usage is a named Unmappable loss, got {subjects:?}"
    );
    assert!(
        subjects.iter().any(|s| s.contains("derivation")),
        "the #derivation extension is named, got {subjects:?}"
    );
    assert!(
        subjects.iter().any(|s| s.contains("satisfy")),
        "the unresolvable satisfy is named, got {subjects:?}"
    );
    assert!(
        blocking.iter().all(|m| m["verdict"] == "Unmappable"),
        "the viewer's blocking losses are all Unmappable, got {blocking:?}"
    );

    // Accept every blocking loss by its entry identity, exactly as the workbench form does.
    let accept: Vec<String> = blocking
        .iter()
        .map(|m| {
            format!(
                "{} [{}]",
                m["subject"].as_str().unwrap(),
                m["verdict"].as_str().unwrap().to_lowercase()
            )
        })
        .collect();

    let committed = router
        .clone()
        .oneshot(post(
            "/projects/drone/import",
            serde_json::json!({
                "binding": "sysml-v2-textual@1.0",
                "branch": "main",
                "author": "alex",
                "message": "import drone",
                "artifact": artifact,
                "acceptLosses": accept
            }),
        ))
        .await
        .unwrap();
    assert_eq!(committed.status(), StatusCode::CREATED, "{:?}", committed);
    let body = json_body(committed).await;

    // The named loss report travels with the commit and states the binding's direction.
    let report = &body["lossReport"];
    assert_eq!(report["binding"]["id"], "sysml-v2-textual");
    assert_eq!(report["binding"]["version"], "1.0");
    assert_eq!(report["binding"]["direction"], "ImportOnly");
    let mappings = report["mappings"].as_array().expect("loss mappings");
    assert!(
        mappings.iter().any(|m| {
            m["subject"].as_str().unwrap().starts_with("part drone") && m["verdict"] == "Unmappable"
        }),
        "the top-level usage is named Unmappable in the report"
    );

    // A viewer has no round trip to measure, so fidelity is null - never a fake diff.
    assert_eq!(body["fidelity"], serde_json::Value::Null);

    // The commit provenance names the binding it migrated through.
    let provenance = &body["commit"]["provenance"];
    assert_eq!(provenance["kind"], "imported");
    assert_eq!(provenance["bindingId"], "sysml-v2-textual");
    assert_eq!(provenance["bindingVersion"], "1.0");

    // The imported OKF is committed: one block (the Drone part def) and four requirements.
    let commit_hash = body["commit"]["hash"].as_str().expect("commit hash");
    let store = server::store::sqlite::SqliteStore::open(&dir.path().join("mw.db")).unwrap();
    let commit = store.commit("drone", commit_hash).unwrap().expect("commit");
    let okf_bytes = store.blob(&commit.okf_hash).unwrap().expect("okf blob");
    let okf: okf::types::OkfRoot = serde_json::from_slice(&okf_bytes).unwrap();
    assert_eq!(okf.structure.len(), 1, "one Drone block");
    assert_eq!(okf.requirements.len(), 4, "four requirements");
}

#[test]
fn export_and_round_trip_through_the_viewer_are_refused_with_the_direction_named() {
    let binding =
        server::binding_registry::resolve("sysml-v2-textual", "1.0").expect("the viewer resolves");

    let root: okf::types::OkfRoot = serde_json::from_value(serde_json::json!({
        "project": "Demo",
        "summary": {},
        "stateMachine": { "name": "sm", "regions": [] },
        "graph": { "nodes": [], "edges": [] }
    }))
    .unwrap();

    // Export is refused with a message naming the viewer and its direction.
    match binding.export(&root) {
        Err(binding::BindingError::Export(message)) => {
            assert!(message.contains("ImportOnly"), "message: {}", message);
            assert!(message.contains("viewer"), "message: {}", message);
        }
        other => panic!("expected an Export refusal, got {:?}", other),
    }

    // The round-trip harness refuses a viewer outright, naming its id.
    let source = serde_json::to_vec(&root).unwrap();
    match binding::round_trip(binding.as_ref(), &source) {
        Err(binding::BindingError::Viewer { id }) => assert_eq!(id, "sysml-v2-textual"),
        other => panic!("expected a Viewer refusal, got {:?}", other),
    }
}
