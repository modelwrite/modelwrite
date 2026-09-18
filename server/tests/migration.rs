// SPDX-License-Identifier: AGPL-3.0-or-later
//! Migration as a gated operation: the import endpoint retains the source artifact
//! byte-for-byte, refuses a blocking loss until it is accepted by name, measures fidelity
//! with the engine's harness, and records provenance on the commit.

use axum::body::Body;
use axum::http::{Request, StatusCode};
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

fn get(uri: &str) -> Request<Body> {
    Request::builder().uri(uri).body(Body::empty()).unwrap()
}

async fn json_body(response: axum::response::Response) -> serde_json::Value {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

/// Read a hand-written SysML v1 XMI fixture from the binding's corpus. These are the same
/// synthetic documents the reader's own tests use, so the import behaviour is pinned to the
/// same hand-checked expectations.
fn fixture(name: &str) -> Vec<u8> {
    let path = format!(
        "{}/../engine/binding-xmi/fixtures/{}",
        env!("CARGO_MANIFEST_DIR"),
        name
    );
    std::fs::read(path).expect("fixture must exist")
}

fn raw_artifact(name: &str) -> serde_json::Value {
    let bytes = fixture(name);
    serde_json::json!({ "artifact": String::from_utf8(bytes).unwrap() })
}

fn import_body(name: &str, accept_losses: &[&str]) -> serde_json::Value {
    let mut body = raw_artifact(name);
    body["binding"] = serde_json::json!("sysml-v1-xmi@2.4");
    body["branch"] = serde_json::json!("main");
    body["author"] = serde_json::json!("alex");
    body["message"] = serde_json::json!(format!("import {}", name));
    body["acceptLosses"] = serde_json::json!(accept_losses);
    body
}

async fn create_project(router: &axum::Router, name: &str) {
    let response = router
        .clone()
        .oneshot(post("/projects", serde_json::json!({ "name": name })))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
}

#[tokio::test]
async fn importing_the_fixture_commits_with_provenance_and_retains_the_artifact() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    create_project(&router, "coffee").await;

    let bytes = fixture("coffee-grinder.xmi");
    let expected_hash = server::store::blob_hash(&bytes);

    let response = router
        .clone()
        .oneshot(post(
            "/projects/coffee/import",
            import_body(
                "coffee-grinder.xmi",
                &["uml:Package pkg-structure (Structure)"],
            ),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED, "{:?}", response);
    let body = json_body(response).await;

    // The commit's provenance names the artifact hash and the binding, and the accepted loss.
    let provenance = &body["commit"]["provenance"];
    assert_eq!(provenance["artifactHash"], expected_hash.as_str());
    assert_eq!(provenance["bindingId"], "sysml-v1-xmi");
    assert_eq!(provenance["bindingVersion"], "2.4");
    assert_eq!(
        provenance["acceptedLosses"],
        serde_json::json!(["uml:Package pkg-structure (Structure)"])
    );

    // The engine's fidelity measurement says the round trip lost nothing.
    assert_eq!(body["fidelity"]["equal"], true);

    // The source artifact is retained byte-for-byte, retrievable by its content address.
    let store = server::store::sqlite::SqliteStore::open(&dir.path().join("mw.db")).unwrap();
    let retained = store.blob(&expected_hash).unwrap().expect("blob retained");
    assert_eq!(
        retained, bytes,
        "the retained artifact must be byte-identical"
    );
}

#[tokio::test]
async fn an_unmapped_element_is_refused_until_named_and_then_recorded_in_the_audit() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    create_project(&router, "coffee").await;

    // Without acceptance the import is refused, and the blocking entries come back by name.
    let refused = router
        .clone()
        .oneshot(post(
            "/projects/coffee/import",
            import_body("unknown-element.xmi", &[]),
        ))
        .await
        .unwrap();
    assert_eq!(refused.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let refused_body = json_body(refused).await;
    let blocking = refused_body["blocking"].as_array().unwrap();
    let subjects: Vec<&str> = blocking
        .iter()
        .map(|m| m["subject"].as_str().unwrap())
        .collect();
    assert!(subjects.contains(&"uml:StateMachine sm-1"));
    assert!(subjects.contains(&"uml:Class block-1 attribute 'visibility'"));

    // Naming both losses as accepted lets the import commit.
    let accepted = router
        .clone()
        .oneshot(post(
            "/projects/coffee/import",
            import_body(
                "unknown-element.xmi",
                &[
                    "uml:StateMachine sm-1",
                    "uml:Class block-1 attribute 'visibility'",
                ],
            ),
        ))
        .await
        .unwrap();
    assert_eq!(accepted.status(), StatusCode::CREATED);
    let accepted_body = json_body(accepted).await;
    assert_eq!(
        accepted_body["commit"]["provenance"]["bindingId"],
        "sysml-v1-xmi"
    );

    // The audit trail records the acceptance: the verified actor and the named losses.
    let audit = router
        .clone()
        .oneshot(get("/projects/coffee/audit"))
        .await
        .unwrap();
    let entries = json_body(audit).await;
    let accept = entries
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["action"] == "import.accept")
        .expect("an import.accept entry must exist");
    assert_eq!(accept["actor"], "anonymous");
    let detail = accept["detail"].as_str().unwrap();
    assert!(
        detail.contains("uml:StateMachine sm-1"),
        "detail: {}",
        detail
    );
    assert!(detail.contains("sysml-v1-xmi@2.4"), "detail: {}", detail);
}

#[tokio::test]
async fn an_unknown_binding_is_a_clean_error_not_a_500() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    create_project(&router, "coffee").await;

    let mut body = raw_artifact("coffee-grinder.xmi");
    body["binding"] = serde_json::json!("no-such-binding@9.9");
    body["branch"] = serde_json::json!("main");
    body["message"] = serde_json::json!("import");
    let response = router
        .clone()
        .oneshot(post("/projects/coffee/import", body))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = json_body(response).await;
    assert!(body["error"].as_str().unwrap().contains("unknown binding"));
}

#[tokio::test]
async fn the_report_is_retrievable_after_a_refused_import() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    create_project(&router, "coffee").await;

    let bytes = fixture("unknown-element.xmi");
    let artifact_hash = server::store::blob_hash(&bytes);

    let refused = router
        .clone()
        .oneshot(post(
            "/projects/coffee/import",
            import_body("unknown-element.xmi", &[]),
        ))
        .await
        .unwrap();
    assert_eq!(refused.status(), StatusCode::UNPROCESSABLE_ENTITY);

    // The report for the retained artifact is retrievable even though nothing committed.
    let report = router
        .clone()
        .oneshot(get(&format!(
            "/projects/coffee/import/{}/report",
            artifact_hash
        )))
        .await
        .unwrap();
    assert_eq!(report.status(), StatusCode::OK);
    let body = json_body(report).await;
    assert_eq!(body["artifactHash"], artifact_hash.as_str());
    assert_eq!(body["bindingId"], "sysml-v1-xmi");
    let mappings = body["lossReport"]["mappings"].as_array().unwrap();
    assert!(mappings
        .iter()
        .any(|m| m["subject"] == "uml:StateMachine sm-1"));
}

#[tokio::test]
async fn a_base64_artifact_imports_the_same_as_a_raw_body() {
    use base64::Engine as _;
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    create_project(&router, "coffee").await;

    let bytes = fixture("coffee-grinder.xmi");
    let encoded = base64::engine::general_purpose::STANDARD.encode(&bytes);
    let mut body = serde_json::json!({ "artifact": encoded });
    body["binding"] = serde_json::json!("sysml-v1-xmi@2.4");
    body["branch"] = serde_json::json!("main");
    body["author"] = serde_json::json!("alex");
    body["message"] = serde_json::json!("import base64");
    body["acceptLosses"] = serde_json::json!(["uml:Package pkg-structure (Structure)"]);

    let response = router
        .clone()
        .oneshot(post("/projects/coffee/import", body))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let body = json_body(response).await;
    assert_eq!(
        body["commit"]["provenance"]["artifactHash"],
        server::store::blob_hash(&bytes).as_str()
    );
}
