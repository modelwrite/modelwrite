// SPDX-License-Identifier: AGPL-3.0-or-later
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
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

/// A minimal model that is valid (project, state machine, graph) and, when gated against
/// itself, PASSES: one block satisfying one requirement, no isolated nodes, full coverage.
fn model(name: &str) -> serde_json::Value {
    serde_json::json!({
        "project": name,
        "exportedAt": "2026-09-19T00:00:00Z",
        "summary": {},
        "stateMachine": { "name": "sm", "regions": [] },
        "requirements": [
            { "id": "r1", "name": "Req", "kind": "requirement", "stereotypes": ["Requirement"], "attributes": [], "documentation": "", "reqId": "1.1", "reqText": "shall satisfy" }
        ],
        "graph": {
            "nodes": [
                { "id": "b1", "kind": "block", "name": "Block" },
                { "id": "r1", "kind": "requirement", "name": "Req" }
            ],
            "edges": [
                { "source": "b1", "target": "r1", "kind": "dependency", "label": "Satisfy" }
            ]
        }
    })
}

async fn create_project(router: &axum::Router, name: &str) {
    let response = router
        .clone()
        .oneshot(post("/projects", serde_json::json!({ "name": name })))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
}

async fn commit(router: &axum::Router, project: &str, okf: serde_json::Value) -> String {
    let response = router
        .clone()
        .oneshot(post(
            &format!("/projects/{}/commits", project),
            serde_json::json!({ "branch": "main", "author": "alex", "message": "model", "okf": okf }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    json_body(response).await["hash"]
        .as_str()
        .unwrap()
        .to_string()
}

#[tokio::test]
async fn a_platform_model_reference_resolves_and_is_listed() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    create_project(&router, "radar").await;
    let radar_hash = commit(&router, "radar", model("radar")).await;

    create_project(&router, "ship").await;
    let mut platform = model("ship");
    platform["references"] = serde_json::json!([
        { "project": "radar", "revision": radar_hash, "role": "radar" }
    ]);
    let ship_hash = commit(&router, "ship", platform).await;

    // The reference is listed exactly as the document carries it: typed, pinned, committed.
    let listed = router
        .clone()
        .oneshot(get(&format!(
            "/projects/ship/commits/{}/references",
            ship_hash
        )))
        .await
        .unwrap();
    assert_eq!(listed.status(), StatusCode::OK);
    let body = json_body(listed).await;
    assert_eq!(
        body["references"],
        serde_json::json!([{ "project": "radar", "revision": radar_hash, "role": "radar" }]),
    );

    // And the reference resolves: the named project exists and the revision is a commit of it.
    let resolved = router
        .oneshot(get(&format!(
            "/projects/ship/commits/{}/references/resolve",
            ship_hash
        )))
        .await
        .unwrap();
    assert_eq!(resolved.status(), StatusCode::OK);
    let body = json_body(resolved).await;
    assert_eq!(body["resolves"], true);
    assert_eq!(body["references"][0]["resolves"], true);
    assert_eq!(body["references"][0]["reason"], serde_json::Value::Null);
}

#[tokio::test]
async fn a_stale_revision_is_a_named_validation_error() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    create_project(&router, "radar").await;
    let _radar_hash = commit(&router, "radar", model("radar")).await;

    create_project(&router, "ship").await;
    let mut platform = model("ship");
    let stale = "0".repeat(64);
    platform["references"] = serde_json::json!([
        { "project": "radar", "revision": stale, "role": "radar" }
    ]);
    let response = router
        .oneshot(post(
            "/projects/ship/commits",
            serde_json::json!({ "branch": "main", "author": "alex", "message": "model", "okf": platform }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body = json_body(response).await;
    let error = body["error"].as_str().unwrap();
    assert!(
        error.contains("not a commit of project radar"),
        "the error must name the stale hash reason: {}",
        error
    );
    assert!(
        error.contains("does not resolve"),
        "the error must say the reference does not resolve: {}",
        error
    );
}

#[tokio::test]
async fn a_missing_project_is_a_named_validation_error() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    create_project(&router, "ship").await;
    let mut platform = model("ship");
    platform["references"] = serde_json::json!([
        { "project": "no-such-subsystem", "revision": "a".repeat(64), "role": "radar" }
    ]);
    let response = router
        .oneshot(post(
            "/projects/ship/commits",
            serde_json::json!({ "branch": "main", "author": "alex", "message": "model", "okf": platform }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body = json_body(response).await;
    let error = body["error"].as_str().unwrap();
    assert!(
        error.contains("project no-such-subsystem does not exist"),
        "the error must name the missing project: {}",
        error
    );
}

#[tokio::test]
async fn the_single_model_gate_is_unchanged_by_references() {
    // S0 adds the reference primitive; it must not weaken or replace the single-model gate.
    // A platform model whose references resolve still gates exactly like any other model:
    // round-trip equal when gated against itself, references treated as ordinary content.
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    create_project(&router, "radar").await;
    let radar_hash = commit(&router, "radar", model("radar")).await;

    create_project(&router, "ship").await;
    let mut platform = model("ship");
    platform["references"] = serde_json::json!([
        { "project": "radar", "revision": radar_hash, "role": "radar" }
    ]);
    let ship_hash = commit(&router, "ship", platform).await;

    let run = router
        .oneshot(post(
            "/projects/ship/gate",
            serde_json::json!({ "reference": ship_hash, "candidate": ship_hash }),
        ))
        .await
        .unwrap();
    assert_eq!(run.status(), StatusCode::OK);
    let evidence = json_body(run).await;
    assert_eq!(evidence["passed"], true);
    assert_eq!(evidence["roundtrip"]["equal"], true);
}
