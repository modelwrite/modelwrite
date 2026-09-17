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

fn tiny_okf() -> serde_json::Value {
    serde_json::json!({
        "project": "tiny",
        "exportedAt": "2026-09-17T00:00:00Z",
        "summary": {},
        "stateMachine": { "name": "tiny sm", "regions": [] },
        "graph": { "nodes": [{ "id": "b1", "kind": "block", "name": "B1" }], "edges": [] }
    })
}

#[tokio::test]
async fn a_project_can_be_created_listed_and_is_not_duplicated() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));

    let created = router
        .clone()
        .oneshot(post("/projects", serde_json::json!({ "name": "coffee" })))
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::CREATED);

    let duplicate = router
        .clone()
        .oneshot(post("/projects", serde_json::json!({ "name": "coffee" })))
        .await
        .unwrap();
    assert_eq!(duplicate.status(), StatusCode::CONFLICT);

    let listed = router
        .oneshot(
            Request::builder()
                .uri("/projects")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let value = json_body(listed).await;
    assert_eq!(value[0]["name"], "coffee");
}

#[tokio::test]
async fn a_commit_stores_the_model_and_moves_the_branch() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    router
        .clone()
        .oneshot(post("/projects", serde_json::json!({ "name": "coffee" })))
        .await
        .unwrap();

    let committed = router
        .clone()
        .oneshot(post(
            "/projects/coffee/commits",
            serde_json::json!({
                "branch": "main",
                "author": "alex",
                "message": "import the model",
                "okf": tiny_okf()
            }),
        ))
        .await
        .unwrap();
    assert_eq!(committed.status(), StatusCode::CREATED);
    let commit = json_body(committed).await;
    assert!(commit["hash"].as_str().unwrap().len() == 64);
    assert!(commit["parents"].as_array().unwrap().is_empty());

    let hash = commit["hash"].as_str().unwrap().to_string();
    let fetched = router
        .oneshot(
            Request::builder()
                .uri(format!("/projects/coffee/commits/{}", hash))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(fetched.status(), StatusCode::OK);
    // Byte for byte, not structurally: comparing two parsed values would let a
    // key-reordering or reformatting regression pass unnoticed.
    let bytes = fetched.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(
        bytes.as_ref(),
        serde_json::to_vec(&tiny_okf()).unwrap().as_slice(),
        "the stored model must come back byte for byte"
    );
}

#[tokio::test]
async fn an_invalid_model_is_rejected_with_its_errors() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    router
        .clone()
        .oneshot(post("/projects", serde_json::json!({ "name": "coffee" })))
        .await
        .unwrap();

    let mut okf = tiny_okf();
    okf["graph"] = serde_json::Value::Null;
    let rejected = router
        .oneshot(post(
            "/projects/coffee/commits",
            serde_json::json!({
                "branch": "main",
                "author": "alex",
                "message": "broken",
                "okf": okf
            }),
        ))
        .await
        .unwrap();
    assert_eq!(rejected.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body = json_body(rejected).await;
    assert!(body["error"]
        .as_str()
        .unwrap()
        .contains("graph section is missing"));
}

#[tokio::test]
async fn committing_an_unknown_project_is_not_found() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    let response = router
        .oneshot(post(
            "/projects/missing/commits",
            serde_json::json!({
                "branch": "main",
                "author": "alex",
                "message": "nothing",
                "okf": tiny_okf()
            }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}
#[tokio::test]
async fn branch_creation_reports_unknown_commits_and_duplicates() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    router
        .clone()
        .oneshot(post("/projects", serde_json::json!({ "name": "coffee" })))
        .await
        .unwrap();
    let committed = router
        .clone()
        .oneshot(post(
            "/projects/coffee/commits",
            serde_json::json!({ "branch": "main", "author": "alex", "message": "m", "okf": tiny_okf() }),
        ))
        .await
        .unwrap();
    let hash = json_body(committed).await["hash"]
        .as_str()
        .unwrap()
        .to_string();

    let unknown = router
        .clone()
        .oneshot(post(
            "/projects/coffee/branches",
            serde_json::json!({ "name": "review", "from": "not-a-commit" }),
        ))
        .await
        .unwrap();
    assert_eq!(unknown.status(), StatusCode::NOT_FOUND);

    let created = router
        .clone()
        .oneshot(post(
            "/projects/coffee/branches",
            serde_json::json!({ "name": "review", "from": hash }),
        ))
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::CREATED);

    let duplicate = router
        .oneshot(post(
            "/projects/coffee/branches",
            serde_json::json!({ "name": "review", "from": hash }),
        ))
        .await
        .unwrap();
    assert_eq!(duplicate.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn commits_can_be_listed_per_branch() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    router
        .clone()
        .oneshot(post("/projects", serde_json::json!({ "name": "coffee" })))
        .await
        .unwrap();
    router
        .clone()
        .oneshot(post(
            "/projects/coffee/commits",
            serde_json::json!({ "branch": "main", "author": "alex", "message": "m", "okf": tiny_okf() }),
        ))
        .await
        .unwrap();

    let main = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/projects/coffee/commits?branch=main")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(json_body(main).await.as_array().unwrap().len(), 1);

    let other = router
        .oneshot(
            Request::builder()
                .uri("/projects/coffee/commits?branch=nothing-here")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(json_body(other).await.as_array().unwrap().is_empty());
}
