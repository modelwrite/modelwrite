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
        .acquire_locks(
            "coffee",
            "main",
            &["b1".to_string()],
            "alex",
            30,
            1000,
            None,
        )
        .unwrap();

    // Still inside the lease: a second holder is refused.
    match store.acquire_locks("coffee", "main", &["b1".to_string()], "bob", 30, 1000, None) {
        Err(StoreError::Locked { holder, .. }) => {
            assert_eq!(holder, "alex", "the refusal must name the holder");
        }
        other => panic!("expected a lock refusal at now=1000, got {:?}", other),
    }

    // Past the lease (1030): the lease is dead and the second holder succeeds.
    let acquired = store
        .acquire_locks("coffee", "main", &["b1".to_string()], "bob", 30, 1031, None)
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
#[tokio::test]
async fn locks_can_be_released_with_delete_or_post() {
    // The interface promises both: DELETE is the HTTP verb for removing a claim, and POST
    // exists because a request body on DELETE is unusual. Both must be wired and both must
    // release exactly the same way.
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    router
        .clone()
        .oneshot(post("/projects", serde_json::json!({ "name": "coffee" })))
        .await
        .unwrap();

    // DELETE with a body.
    let acquired = router
        .clone()
        .oneshot(post(
            "/projects/coffee/locks",
            serde_json::json!({ "branch": "main", "elements": ["b1", "b1"], "holder": "alex", "ttlSeconds": 600 }),
        ))
        .await
        .unwrap();
    assert_eq!(acquired.status(), StatusCode::CREATED);
    let locks = json_body(acquired).await;
    assert_eq!(
        locks.as_array().unwrap().len(),
        1,
        "a repeated element is one lease, not two"
    );
    let id = locks[0]["id"].as_str().unwrap().to_string();

    let released = router
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/projects/coffee/locks")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({ "holder": "alex", "ids": [id] }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(released.status(), StatusCode::OK);
    assert_eq!(json_body(released).await["released"], 1);

    let listed = router.oneshot(get("/projects/coffee/locks")).await.unwrap();
    assert!(json_body(listed).await.as_array().unwrap().is_empty());
}

fn model_with_nodes(ids: &[&str]) -> serde_json::Value {
    let nodes: Vec<serde_json::Value> = ids
        .iter()
        .map(|id| serde_json::json!({ "id": id, "kind": "block", "name": id.to_uppercase() }))
        .collect();
    serde_json::json!({
        "project": "coffee",
        "exportedAt": "2026-09-17T00:00:00Z",
        "summary": {},
        "stateMachine": { "name": "sm", "regions": [] },
        "graph": { "nodes": nodes, "edges": [] }
    })
}

#[tokio::test]
async fn a_commit_is_refused_when_another_holder_locks_a_touched_element() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    seed_project(&router).await;

    // Establish the branch tip with two elements, so a later commit can change one while
    // leaving the other - and its lock - untouched.
    let seeded = router
        .clone()
        .oneshot(post(
            "/projects/coffee/commits",
            serde_json::json!({
                "branch": "main",
                "author": "alex",
                "message": "seed",
                "okf": model_with_nodes(&["b1", "b2"])
            }),
        ))
        .await
        .unwrap();
    assert_eq!(seeded.status(), StatusCode::CREATED);
    let tip = json_body(seeded).await["hash"]
        .as_str()
        .unwrap()
        .to_string();

    let locked = router
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
    assert_eq!(locked.status(), StatusCode::CREATED);

    // B renames b1: the locked element is the one being CHANGED, so the commit is refused.
    let refused = router
        .clone()
        .oneshot(post(
            "/projects/coffee/commits",
            serde_json::json!({
                "branch": "main",
                "author": "bob",
                "message": "rename b1",
                "holder": "bob",
                "okf": model_with_nodes(&["b1-renamed", "b2"])
            }),
        ))
        .await
        .unwrap();
    assert_eq!(refused.status(), StatusCode::CONFLICT);
    let error = json_body(refused).await;
    let message = error["error"].as_str().unwrap();
    assert!(
        message.contains("b1"),
        "the 409 must name the locked element"
    );
    assert!(message.contains("alex"), "the 409 must name the holder");

    // The refused commit must write NOTHING: the branch tip is unchanged.
    let branches = router
        .clone()
        .oneshot(get("/projects/coffee/branches"))
        .await
        .unwrap();
    let branches = json_body(branches).await;
    let main = branches
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["name"] == "main")
        .unwrap();
    assert_eq!(main["tip"].as_str().unwrap(), tip.as_str());

    // A model that changes only b2 - an element nobody locked - succeeds even though the
    // document still CONTAINS the locked b1: a lock guards change, not mere presence.
    let ok = router
        .clone()
        .oneshot(post(
            "/projects/coffee/commits",
            serde_json::json!({
                "branch": "main",
                "author": "bob",
                "message": "rename b2",
                "holder": "bob",
                "okf": model_with_nodes(&["b1", "b2-renamed"])
            }),
        ))
        .await
        .unwrap();
    assert_eq!(ok.status(), StatusCode::CREATED);
}

#[tokio::test]
async fn the_holder_may_commit_its_own_locked_element() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    seed_project(&router).await;

    let seeded = router
        .clone()
        .oneshot(post(
            "/projects/coffee/commits",
            serde_json::json!({
                "branch": "main",
                "author": "alex",
                "message": "seed",
                "okf": model_with_nodes(&["b1"])
            }),
        ))
        .await
        .unwrap();
    assert_eq!(seeded.status(), StatusCode::CREATED);

    let locked = router
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
    assert_eq!(locked.status(), StatusCode::CREATED);

    // The same holder that owns the lock may commit the change to that element.
    let committed = router
        .oneshot(post(
            "/projects/coffee/commits",
            serde_json::json!({
                "branch": "main",
                "author": "alex",
                "message": "rename b1",
                "holder": "alex",
                "okf": model_with_nodes(&["b1-renamed"])
            }),
        ))
        .await
        .unwrap();
    assert_eq!(committed.status(), StatusCode::CREATED);
}

#[tokio::test]
async fn a_commit_without_a_holder_is_refused_when_it_touches_a_locked_element() {
    // A writer that omits the holder field is still checked: it cannot be the holder, so a
    // live lease on a touched element refuses it. This is the overwrite locks exist to
    // prevent, and it must not depend on the writer cooperating.
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    seed_project(&router).await;

    let seeded = router
        .clone()
        .oneshot(post(
            "/projects/coffee/commits",
            serde_json::json!({
                "branch": "main",
                "author": "alex",
                "message": "seed",
                "okf": model_with_nodes(&["b1", "b2"])
            }),
        ))
        .await
        .unwrap();
    assert_eq!(seeded.status(), StatusCode::CREATED);
    let tip = json_body(seeded).await["hash"]
        .as_str()
        .unwrap()
        .to_string();

    let locked = router
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
    assert_eq!(locked.status(), StatusCode::CREATED);

    // No holder field: the commit still refuses to change the locked b1.
    let refused = router
        .clone()
        .oneshot(post(
            "/projects/coffee/commits",
            serde_json::json!({
                "branch": "main",
                "author": "bob",
                "message": "rename b1",
                "okf": model_with_nodes(&["b1-renamed", "b2"])
            }),
        ))
        .await
        .unwrap();
    assert_eq!(refused.status(), StatusCode::CONFLICT);
    let message = json_body(refused).await["error"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(message.contains("b1"), "the 409 must name the element");
    assert!(message.contains("alex"), "the 409 must name the holder");
    assert!(
        message.contains("supply the holder"),
        "the 409 must tell the holder to supply the field: {}",
        message
    );

    // Nothing landed, and the refusal is recorded.
    let branches = router
        .clone()
        .oneshot(get("/projects/coffee/branches"))
        .await
        .unwrap();
    let main = json_body(branches).await;
    let main = main
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["name"] == "main")
        .unwrap();
    assert_eq!(main["tip"].as_str().unwrap(), tip.as_str());

    let audit = router.oneshot(get("/projects/coffee/audit")).await.unwrap();
    let entries = json_body(audit).await;
    assert!(
        entries
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["action"] == "commit.refused"),
        "a refused overwrite must be recorded as commit.refused"
    );
}

#[tokio::test]
async fn a_reset_that_would_change_a_locked_element_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    seed_project(&router).await;

    let first = router
        .clone()
        .oneshot(post(
            "/projects/coffee/commits",
            serde_json::json!({
                "branch": "main",
                "author": "alex",
                "message": "one",
                "okf": model_with_nodes(&["b1"])
            }),
        ))
        .await
        .unwrap();
    assert_eq!(first.status(), StatusCode::CREATED);
    let first_hash = json_body(first).await["hash"].as_str().unwrap().to_string();

    let second = router
        .clone()
        .oneshot(post(
            "/projects/coffee/commits",
            serde_json::json!({
                "branch": "main",
                "author": "alex",
                "message": "two",
                "okf": model_with_nodes(&["b1-renamed"])
            }),
        ))
        .await
        .unwrap();
    assert_eq!(second.status(), StatusCode::CREATED);

    let locked = router
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
    assert_eq!(locked.status(), StatusCode::CREATED);

    // Resetting back to the first commit restores b1, so it changes a locked element.
    let refused = router
        .oneshot(post(
            "/projects/coffee/branches/main/reset",
            serde_json::json!({ "to": first_hash, "author": "alex", "message": "revert" }),
        ))
        .await
        .unwrap();
    assert_eq!(refused.status(), StatusCode::CONFLICT);
    assert!(
        json_body(refused).await["error"]
            .as_str()
            .unwrap()
            .contains("b1"),
        "the reset refusal must name the locked element"
    );
}

#[tokio::test]
async fn a_merge_that_would_change_a_locked_element_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    seed_project(&router).await;

    let base = router
        .clone()
        .oneshot(post(
            "/projects/coffee/commits",
            serde_json::json!({
                "branch": "main",
                "author": "alex",
                "message": "base",
                "okf": model_with_nodes(&["b1"])
            }),
        ))
        .await
        .unwrap();
    assert_eq!(base.status(), StatusCode::CREATED);
    let base_hash = json_body(base).await["hash"].as_str().unwrap().to_string();

    let branched = router
        .clone()
        .oneshot(post(
            "/projects/coffee/branches",
            serde_json::json!({ "name": "feature", "from": base_hash }),
        ))
        .await
        .unwrap();
    assert_eq!(branched.status(), StatusCode::CREATED);

    let renamed = router
        .clone()
        .oneshot(post(
            "/projects/coffee/commits",
            serde_json::json!({
                "branch": "feature",
                "author": "alex",
                "message": "rename",
                "okf": model_with_nodes(&["b1-renamed"])
            }),
        ))
        .await
        .unwrap();
    assert_eq!(renamed.status(), StatusCode::CREATED);

    let locked = router
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
    assert_eq!(locked.status(), StatusCode::CREATED);

    // The merge would move main's b1 to b1-renamed, changing the locked element.
    let refused = router
        .oneshot(post(
            "/projects/coffee/merge",
            serde_json::json!({ "branch": "main", "other": "feature", "author": "alex", "message": "merge" }),
        ))
        .await
        .unwrap();
    assert_eq!(refused.status(), StatusCode::CONFLICT);
    assert!(
        json_body(refused).await["error"]
            .as_str()
            .unwrap()
            .contains("b1"),
        "the merge refusal must name the locked element"
    );
}

#[tokio::test]
async fn a_denied_lock_is_recorded() {
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

    let denied = router
        .clone()
        .oneshot(post(
            "/projects/coffee/locks",
            serde_json::json!({
                "branch": "main",
                "elements": ["b1"],
                "holder": "bob",
                "ttlSeconds": 300
            }),
        ))
        .await
        .unwrap();
    assert_eq!(denied.status(), StatusCode::CONFLICT);

    let audit = router.oneshot(get("/projects/coffee/audit")).await.unwrap();
    let entries = json_body(audit).await;
    assert!(
        entries
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["action"] == "lock.denied"),
        "a refused acquire must be recorded as lock.denied"
    );
}

#[tokio::test]
async fn locks_validate_the_holder_and_allow_okf_element_ids() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    seed_project(&router).await;

    // An empty holder could never be the holder on a later write, so it must be rejected
    // rather than acquiring a lease that blocks everyone.
    let empty_holder = router
        .clone()
        .oneshot(post(
            "/projects/coffee/locks",
            serde_json::json!({
                "branch": "main",
                "elements": ["b1"],
                "holder": "",
                "ttlSeconds": 300
            }),
        ))
        .await
        .unwrap();
    assert_eq!(empty_holder.status(), StatusCode::BAD_REQUEST);

    // Element ids may contain characters validate_name rejects: an element that can be
    // CHANGED must also be LOCKED.
    let special = router
        .clone()
        .oneshot(post(
            "/projects/coffee/locks",
            serde_json::json!({
                "branch": "main",
                "elements": ["sysml/block:1"],
                "holder": "alex",
                "ttlSeconds": 300
            }),
        ))
        .await
        .unwrap();
    assert_eq!(special.status(), StatusCode::CREATED);

    // A control character is still rejected.
    let control = router
        .clone()
        .oneshot(post(
            "/projects/coffee/locks",
            serde_json::json!({
                "branch": "main",
                "elements": ["badid"],
                "holder": "alex",
                "ttlSeconds": 300
            }),
        ))
        .await
        .unwrap();
    assert_eq!(control.status(), StatusCode::BAD_REQUEST);
}
#[tokio::test]
async fn the_holder_may_finish_its_own_work_through_merge_and_reset() {
    // Enforcing locks on every write path is only useful if the person holding the lock can
    // still do their job. A holder must be able to commit, merge and revert the elements it
    // holds; everyone else must be refused. This test drives both halves.
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    seed_project(&router).await;

    let base = model_with_nodes(&["b1"]);
    let committed = router
        .clone()
        .oneshot(post(
            "/projects/coffee/commits",
            serde_json::json!({ "branch": "main", "author": "alex", "message": "base", "okf": base, "holder": "alex" }),
        ))
        .await
        .unwrap();
    assert_eq!(committed.status(), StatusCode::CREATED);
    let base_hash = json_body(committed).await["hash"]
        .as_str()
        .unwrap()
        .to_string();

    // Alex takes the element and works on a branch.
    router
        .clone()
        .oneshot(post(
            "/projects/coffee/locks",
            serde_json::json!({ "branch": "main", "elements": ["b1"], "holder": "alex", "ttlSeconds": 600 }),
        ))
        .await
        .unwrap();
    router
        .clone()
        .oneshot(post(
            "/projects/coffee/branches",
            serde_json::json!({ "name": "feature", "from": base_hash }),
        ))
        .await
        .unwrap();

    // model_with_nodes builds a graph-only document, so the change is to the graph node.
    let mut changed = model_with_nodes(&["b1"]);
    changed["graph"]["nodes"][0]["name"] = serde_json::json!("Renamed");
    let on_feature = router
        .clone()
        .oneshot(post(
            "/projects/coffee/commits",
            serde_json::json!({ "branch": "feature", "author": "alex", "message": "rename", "okf": changed, "holder": "alex" }),
        ))
        .await
        .unwrap();
    assert_eq!(
        on_feature.status(),
        StatusCode::CREATED,
        "the holder may change what it holds"
    );

    // Sam tries to merge that in and is refused: alex holds b1.
    let refused = router
        .clone()
        .oneshot(post(
            "/projects/coffee/merge",
            serde_json::json!({ "branch": "main", "other": "feature", "author": "sam", "message": "merge", "holder": "sam" }),
        ))
        .await
        .unwrap();
    assert_eq!(refused.status(), StatusCode::CONFLICT);

    // Alex merges its own work and succeeds.
    let merged = router
        .clone()
        .oneshot(post(
            "/projects/coffee/merge",
            serde_json::json!({ "branch": "main", "other": "feature", "author": "alex", "message": "merge", "holder": "alex" }),
        ))
        .await
        .unwrap();
    assert_eq!(
        merged.status(),
        StatusCode::CREATED,
        "the holder may merge what it holds"
    );

    // And alex may revert its own branch back, again because it holds the element.
    let reverted = router
        .oneshot(post(
            "/projects/coffee/branches/main/reset",
            serde_json::json!({ "to": base_hash, "author": "alex", "message": "revert", "holder": "alex" }),
        ))
        .await
        .unwrap();
    assert_eq!(
        reverted.status(),
        StatusCode::CREATED,
        "the holder may revert what it holds"
    );
}
