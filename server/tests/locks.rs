// SPDX-License-Identifier: AGPL-3.0-or-later
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

use server::store::{sqlite::SqliteStore, Store, StoreError};

fn state(dir: &std::path::Path) -> server::AppState {
    let store = SqliteStore::open(&dir.join("mw.db")).unwrap();
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

fn get(uri: &str) -> Request<Body> {
    Request::builder().uri(uri).body(Body::empty()).unwrap()
}

async fn json_body(response: axum::response::Response) -> serde_json::Value {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

async fn seed_project(router: &axum::Router) {
    let response = router
        .clone()
        .oneshot(post("/projects", serde_json::json!({ "name": "coffee" })))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
}

#[tokio::test]
async fn a_lock_can_be_acquired_listed_and_released() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    seed_project(&router).await;

    let acquired = router
        .clone()
        .oneshot(post(
            "/projects/coffee/locks",
            serde_json::json!({
                "branch": "main",
                "elements": ["b1", "b2"],
                "holder": "alex",
                "ttlSeconds": 300
            }),
        ))
        .await
        .unwrap();
    assert_eq!(acquired.status(), StatusCode::CREATED);
    let acquired = json_body(acquired).await;
    let acquired = acquired.as_array().unwrap();
    assert_eq!(acquired.len(), 2);
    let ids: Vec<String> = acquired
        .iter()
        .map(|l| l["id"].as_str().unwrap().to_string())
        .collect();

    let listed = router
        .clone()
        .oneshot(get("/projects/coffee/locks"))
        .await
        .unwrap();
    assert_eq!(listed.status(), StatusCode::OK);
    let listed = json_body(listed).await;
    let listed = listed.as_array().unwrap();
    assert_eq!(listed.len(), 2);
    let elements: Vec<&str> = listed
        .iter()
        .map(|l| l["element"].as_str().unwrap())
        .collect();
    assert!(elements.contains(&"b1"));
    assert!(elements.contains(&"b2"));

    let released = router
        .clone()
        .oneshot(post(
            "/projects/coffee/locks/release",
            serde_json::json!({ "holder": "alex", "ids": ids }),
        ))
        .await
        .unwrap();
    assert_eq!(released.status(), StatusCode::OK);
    let body = json_body(released).await;
    assert_eq!(body["released"].as_i64(), Some(2));

    let listed = router.oneshot(get("/projects/coffee/locks")).await.unwrap();
    assert_eq!(json_body(listed).await.as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn a_second_holder_is_refused_and_nothing_is_acquired() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    seed_project(&router).await;

    let a = router
        .clone()
        .oneshot(post(
            "/projects/coffee/locks",
            serde_json::json!({
                "branch": "main",
                "elements": ["b1"],
                "holder": "alex",
                "ttlSeconds": 300
            }),
        ))
        .await
        .unwrap();
    assert_eq!(a.status(), StatusCode::CREATED);

    let refused = router
        .clone()
        .oneshot(post(
            "/projects/coffee/locks",
            serde_json::json!({
                "branch": "main",
                "elements": ["b1", "b2"],
                "holder": "bob",
                "ttlSeconds": 300
            }),
        ))
        .await
        .unwrap();
    assert_eq!(refused.status(), StatusCode::CONFLICT);
    let error = json_body(refused).await;
    assert!(error["error"].as_str().unwrap().contains("alex"));

    // The refused request must leave NOTHING acquired: only A's b1 lock remains.
    let listed = router.oneshot(get("/projects/coffee/locks")).await.unwrap();
    let listed = json_body(listed).await;
    let locks = listed.as_array().unwrap();
    assert_eq!(
        locks.len(),
        1,
        "the refused request must leave nothing acquired"
    );
    assert_eq!(locks[0]["element"], "b1");
    assert_eq!(locks[0]["holder"], "alex");
}

#[tokio::test]
async fn the_same_holder_can_extend_its_own_lease() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    seed_project(&router).await;

    let first = router
        .clone()
        .oneshot(post(
            "/projects/coffee/locks",
            serde_json::json!({
                "branch": "main",
                "elements": ["b1"],
                "holder": "alex",
                "ttlSeconds": 30
            }),
        ))
        .await
        .unwrap();
    assert_eq!(first.status(), StatusCode::CREATED);
    let first = json_body(first).await;
    let first_expiry = first[0]["expiresAt"].as_i64().unwrap();

    let second = router
        .clone()
        .oneshot(post(
            "/projects/coffee/locks",
            serde_json::json!({
                "branch": "main",
                "elements": ["b1"],
                "holder": "alex",
                "ttlSeconds": 300
            }),
        ))
        .await
        .unwrap();
    assert_eq!(second.status(), StatusCode::CREATED);
    let second = json_body(second).await;
    let second_expiry = second[0]["expiresAt"].as_i64().unwrap();
    assert!(
        second_expiry > first_expiry,
        "re-acquiring must extend the lease"
    );

    let listed = router.oneshot(get("/projects/coffee/locks")).await.unwrap();
    let locks = json_body(listed).await;
    assert_eq!(
        locks.as_array().unwrap().len(),
        1,
        "exactly one lock must exist for b1"
    );
}

#[test]
fn an_expired_lease_stops_blocking() {
    // The HTTP layer always passes the real clock, so expiry is driven through the store
    // directly with an injected clock: no sleeping, and the boundary is exact.
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::open(&dir.path().join("mw.db")).unwrap();

    // A acquires b1 at now = 1000 with ttl 30, so the lease expires at 1030.
    store
        .acquire_locks("coffee", "main", &["b1".to_string()], "alex", 30, 1000)
        .unwrap();

    // Still inside the lease: a second holder is refused.
    match store.acquire_locks("coffee", "main", &["b1".to_string()], "bob", 30, 1000) {
        Err(StoreError::Conflict(_)) => {}
        other => panic!("expected a conflict at now=1000, got {:?}", other),
    }

    // Past the lease (1030): the lease is dead and the second holder succeeds.
    let acquired = store
        .acquire_locks("coffee", "main", &["b1".to_string()], "bob", 30, 1031)
        .unwrap();
    assert_eq!(acquired.len(), 1);
    assert_eq!(acquired[0].holder, "bob");
}

#[tokio::test]
async fn only_the_holder_can_release() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    seed_project(&router).await;

    let acquired = router
        .clone()
        .oneshot(post(
            "/projects/coffee/locks",
            serde_json::json!({
                "branch": "main",
                "elements": ["b1"],
                "holder": "alex",
                "ttlSeconds": 300
            }),
        ))
        .await
        .unwrap();
    assert_eq!(acquired.status(), StatusCode::CREATED);
    let acquired = json_body(acquired).await;
    let id = acquired[0]["id"].as_str().unwrap().to_string();

    let released = router
        .clone()
        .oneshot(post(
            "/projects/coffee/locks/release",
            serde_json::json!({ "holder": "bob", "ids": [id] }),
        ))
        .await
        .unwrap();
    assert_eq!(released.status(), StatusCode::OK);
    let body = json_body(released).await;
    assert_eq!(body["released"].as_i64(), Some(0));

    let listed = router.oneshot(get("/projects/coffee/locks")).await.unwrap();
    let locks = json_body(listed).await;
    let locks = locks.as_array().unwrap();
    assert_eq!(locks.len(), 1);
    assert_eq!(locks[0]["holder"], "alex");
}
