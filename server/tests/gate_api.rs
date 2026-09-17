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

fn model(name: &str) -> serde_json::Value {
    serde_json::json!({
        "project": name,
        "exportedAt": "2026-09-17T00:00:00Z",
        "summary": {},
        "stateMachine": { "name": "sm", "regions": [] },
        "graph": {
            "nodes": [
                { "id": "b1", "kind": "block", "name": "Block" },
                { "id": "r1", "kind": "requirement", "name": "Req" }
            ],
            "edges": [
                { "source": "b1", "target": "r1", "kind": "dependency", "label": "Satisfy" }
            ]
        },
        "requirements": [
            { "id": "r1", "name": "Req", "kind": "requirement", "stereotypes": ["Requirement"], "attributes": [], "documentation": "", "reqId": "1.1", "reqText": "shall satisfy" }
        ]
    })
}

async fn seed_project(router: &axum::Router) -> (String, String) {
    router
        .clone()
        .oneshot(post("/projects", serde_json::json!({ "name": "coffee" })))
        .await
        .unwrap();

    let first = router
        .clone()
        .oneshot(post(
            "/projects/coffee/commits",
            serde_json::json!({ "branch": "main", "author": "alex", "message": "model", "okf": model("coffee") }),
        ))
        .await
        .unwrap();
    assert_eq!(first.status(), StatusCode::CREATED);
    let first = json_body(first).await;
    let reference = first["hash"].as_str().unwrap().to_string();

    let mut changed = model("coffee");
    changed["project"] = serde_json::json!("coffee v2");
    let second = router
        .clone()
        .oneshot(post(
            "/projects/coffee/commits",
            serde_json::json!({ "branch": "main", "author": "alex", "message": "changed", "okf": changed }),
        ))
        .await
        .unwrap();
    let second = json_body(second).await;
    let candidate = second["hash"].as_str().unwrap().to_string();

    (reference, candidate)
}

#[tokio::test]
async fn gating_a_commit_against_itself_passes_and_records_evidence() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    let (reference, _candidate) = seed_project(&router).await;

    let response = router
        .clone()
        .oneshot(post(
            "/projects/coffee/gate",
            serde_json::json!({ "reference": reference, "candidate": reference }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let evidence = json_body(response).await;
    assert_eq!(evidence["passed"], true);
    assert_eq!(evidence["roundtrip"]["equal"], true);

    let files: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.starts_with("server-"))
        .collect();
    assert_eq!(
        files.len(),
        1,
        "expected one evidence file, got {:?}",
        files
    );
}

#[tokio::test]
async fn gating_a_changed_model_fails_and_is_still_a_successful_run() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    let (reference, candidate) = seed_project(&router).await;

    let response = router
        .clone()
        .oneshot(post(
            "/projects/coffee/gate",
            serde_json::json!({ "reference": reference, "candidate": candidate }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let evidence = json_body(response).await;
    assert_eq!(evidence["passed"], false);
    assert!(!evidence["roundtrip"]["equal"].as_bool().unwrap());
    assert!(evidence["failures"]
        .as_array()
        .unwrap()
        .iter()
        .any(|f| f.as_str().unwrap().contains("roundtrip")));

    let runs = router
        .oneshot(get("/projects/coffee/gate-runs"))
        .await
        .unwrap();
    let recorded = json_body(runs).await;
    assert_eq!(recorded.as_array().unwrap().len(), 1);
    assert_eq!(recorded[0]["passed"], false);
}

#[tokio::test]
async fn gating_an_unknown_commit_is_not_found() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    let (reference, _candidate) = seed_project(&router).await;
    let response = router
        .oneshot(post(
            "/projects/coffee/gate",
            serde_json::json!({ "reference": reference, "candidate": "missing" }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}
