// SPDX-License-Identifier: AGPL-3.0-or-later
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;

fn state(dir: &std::path::Path) -> server::AppState {
    let store = server::store::sqlite::SqliteStore::open(&dir.join("mw.db")).unwrap();
    server::AppState {
        store: std::sync::Arc::new(store),
        evidence_dir: dir.to_path_buf(),
        auth: server::auth::AuthConfig::Open,
    }
}

fn post(uri: &str, body: Value) -> Request<Body> {
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

async fn json_body(response: axum::response::Response) -> Value {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

async fn read_audit(router: &axum::Router) -> Vec<Value> {
    let response = router
        .clone()
        .oneshot(get("/projects/coffee/audit"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    json_body(response).await.as_array().unwrap().clone()
}

fn model(block_name: &str) -> Value {
    json!({
        "project": "coffee",
        "exportedAt": "2026-09-17T00:00:00Z",
        "summary": {},
        "stateMachine": {"name": "sm", "regions": []},
        "requirements": [{
            "id": "r1", "name": "r1", "kind": "requirement",
            "stereotypes": ["Requirement"], "attributes": [], "documentation": "",
            "reqId": "1.1", "reqText": "text"
        }],
        "structure": [{
            "id": "b1", "name": block_name, "kind": "block",
            "stereotypes": ["Block"], "attributes": [], "documentation": ""
        }],
        "graph": {
            "nodes": [
                {"id": "b1", "kind": "block", "name": block_name},
                {"id": "r1", "kind": "requirement", "name": "r1"}
            ],
            "edges": [{"source": "b1", "target": "r1", "kind": "dependency", "label": "Satisfy"}]
        }
    })
}

async fn commit(router: &axum::Router, branch: &str, message: &str, okf: Value) -> String {
    let response = router
        .clone()
        .oneshot(post(
            "/projects/coffee/commits",
            json!({ "branch": branch, "author": "alex", "message": message, "okf": okf }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED, "commit {}", message);
    json_body(response).await["hash"]
        .as_str()
        .unwrap()
        .to_string()
}

#[tokio::test]
async fn every_mutation_is_recorded() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));

    // project.create
    let created = router
        .clone()
        .oneshot(post("/projects", json!({ "name": "coffee" })))
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::CREATED);

    // commit.create
    let base = commit(&router, "main", "base", model("Block")).await;

    // branch.create
    let branched = router
        .clone()
        .oneshot(post(
            "/projects/coffee/branches",
            json!({ "name": "feature", "from": base }),
        ))
        .await
        .unwrap();
    assert_eq!(branched.status(), StatusCode::CREATED);

    // Another commit on the feature branch, so the merge has a divergent side.
    let _feature_tip = commit(&router, "feature", "rename", model("Renamed")).await;

    // merge.clean
    let merged = router
        .clone()
        .oneshot(post(
            "/projects/coffee/merge",
            json!({ "branch": "main", "other": "feature", "author": "alex", "message": "merge feature" }),
        ))
        .await
        .unwrap();
    assert_eq!(merged.status(), StatusCode::CREATED);

    // branch.reset
    let reset = router
        .clone()
        .oneshot(post(
            "/projects/coffee/branches/main/reset",
            json!({ "to": base, "author": "alex", "message": "revert to base" }),
        ))
        .await
        .unwrap();
    assert_eq!(reset.status(), StatusCode::CREATED);

    // lock.acquire
    let acquired = router
        .clone()
        .oneshot(post(
            "/projects/coffee/locks",
            json!({ "branch": "main", "elements": ["b1"], "holder": "alex", "ttlSeconds": 300 }),
        ))
        .await
        .unwrap();
    assert_eq!(acquired.status(), StatusCode::CREATED);
    let lock_id = json_body(acquired).await[0]["id"]
        .as_str()
        .unwrap()
        .to_string();

    // lock.release
    let released = router
        .clone()
        .oneshot(post(
            "/projects/coffee/locks/release",
            json!({ "holder": "alex", "ids": [lock_id] }),
        ))
        .await
        .unwrap();
    assert_eq!(released.status(), StatusCode::OK);

    // gate.run (a run is recorded whether or not the gate passes)
    let gated = router
        .clone()
        .oneshot(post(
            "/projects/coffee/gate",
            json!({ "reference": base, "candidate": base }),
        ))
        .await
        .unwrap();
    assert_eq!(gated.status(), StatusCode::OK);

    // branch.delete
    let deleted = router
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/projects/coffee/branches/feature")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(deleted.status(), StatusCode::NO_CONTENT);

    let entries = read_audit(&router).await;
    let actions: Vec<&str> = entries
        .iter()
        .map(|e| e["action"].as_str().unwrap())
        .collect();
    for action in [
        "project.create",
        "commit.create",
        "branch.create",
        "branch.delete",
        "branch.reset",
        "merge.clean",
        "lock.acquire",
        "lock.release",
        "gate.run",
    ] {
        assert!(
            actions.contains(&action),
            "missing audit action {} in {:?}",
            action,
            actions
        );
    }

    // Every entry carries a non-empty actor, subject and detail.
    for entry in &entries {
        let actor = entry["actor"].as_str().unwrap();
        let subject = entry["subject"].as_str().unwrap();
        let detail = entry["detail"].as_str().unwrap();
        assert!(
            !actor.is_empty(),
            "audit actor must not be empty: {:?}",
            entry
        );
        assert!(
            !subject.is_empty(),
            "audit subject must not be empty: {:?}",
            entry
        );
        assert!(
            !detail.is_empty(),
            "audit detail must not be empty: {:?}",
            entry
        );
    }
}

#[tokio::test]
async fn a_failed_gate_is_recorded() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    router
        .clone()
        .oneshot(post("/projects", json!({ "name": "coffee" })))
        .await
        .unwrap();

    let expected: Value = serde_json::from_str(&test_support::load_okf_expected()).unwrap();
    let broken: Value = serde_json::from_str(&test_support::load_okf_broken()).unwrap();

    let reference = commit(&router, "main", "import the exported model", expected).await;
    let branched = router
        .clone()
        .oneshot(post(
            "/projects/coffee/branches",
            json!({ "name": "corrupted", "from": reference }),
        ))
        .await
        .unwrap();
    assert_eq!(branched.status(), StatusCode::CREATED);
    let candidate = commit(&router, "corrupted", "drop a requirement", broken).await;

    let gated = router
        .clone()
        .oneshot(post(
            "/projects/coffee/gate",
            json!({ "reference": reference, "candidate": candidate }),
        ))
        .await
        .unwrap();
    assert_eq!(gated.status(), StatusCode::OK);
    assert_eq!(json_body(gated).await["passed"], false);

    let entries = read_audit(&router).await;
    let gate = entries
        .iter()
        .find(|e| e["action"] == "gate.run")
        .expect("a gate.run entry must be recorded");
    let detail = gate["detail"].as_str().unwrap();
    assert!(
        detail.contains("failed"),
        "the failed gate detail must say so: {}",
        detail
    );
}

#[tokio::test]
async fn a_conflicting_merge_is_recorded() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    router
        .clone()
        .oneshot(post("/projects", json!({ "name": "coffee" })))
        .await
        .unwrap();

    let base = commit(&router, "main", "base", model("Block")).await;
    let branched = router
        .clone()
        .oneshot(post(
            "/projects/coffee/branches",
            json!({ "name": "feature", "from": base }),
        ))
        .await
        .unwrap();
    assert_eq!(branched.status(), StatusCode::CREATED);

    // Diverge both branches on the same element, so the merge conflicts.
    let _theirs = commit(&router, "feature", "theirs", model("Theirs")).await;
    let _ours = commit(&router, "main", "ours", model("Ours")).await;

    let conflicted = router
        .clone()
        .oneshot(post(
            "/projects/coffee/merge",
            json!({ "branch": "main", "other": "feature", "author": "alex", "message": "merge" }),
        ))
        .await
        .unwrap();
    assert_eq!(conflicted.status(), StatusCode::CONFLICT);

    let entries = read_audit(&router).await;
    let conflict = entries
        .iter()
        .find(|e| e["action"] == "merge.conflict")
        .expect("a merge.conflict entry must be recorded");
    let detail = conflict["detail"].as_str().unwrap();
    assert!(
        detail.contains("main"),
        "the conflict detail must name the target branch: {}",
        detail
    );
    assert!(
        detail.contains("feature"),
        "the conflict detail must name the other branch: {}",
        detail
    );
}

#[tokio::test]
async fn entries_are_never_rewritten() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    router
        .clone()
        .oneshot(post("/projects", json!({ "name": "coffee" })))
        .await
        .unwrap();

    let _ = commit(&router, "main", "first", model("Block")).await;

    let first = read_audit(&router).await;
    assert_eq!(first.len(), 2, "expected project.create and commit.create");

    // Two more actions append to the log.
    let _ = commit(&router, "main", "second", model("Renamed")).await;
    let _ = commit(&router, "main", "third", model("Third")).await;

    let second = read_audit(&router).await;
    assert_eq!(second.len(), 4, "four entries after two more commits");

    // The earlier entries must be BYTE IDENTICAL: same ids, timestamps, actors, subjects
    // and details. Newest-first order means the earlier two are now the tail.
    assert_eq!(&second[2..], &first[..]);
    // Pointedly, the ids did not change: appending never rewrites a row.
    assert_eq!(second[2]["id"], first[0]["id"]);
    assert_eq!(second[3]["id"], first[1]["id"]);
}

#[tokio::test]
async fn the_log_is_newest_first_and_capped() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    router
        .clone()
        .oneshot(post("/projects", json!({ "name": "coffee" })))
        .await
        .unwrap();
    let _ = commit(&router, "main", "first", model("Block")).await;
    let _ = commit(&router, "main", "second", model("Renamed")).await;

    let response = router
        .clone()
        .oneshot(get("/projects/coffee/audit?limit=2"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let entries = json_body(response).await;
    let entries = entries.as_array().unwrap();
    assert_eq!(entries.len(), 2, "limit=2 must return exactly two entries");
    assert!(
        entries[0]["id"].as_i64().unwrap() > entries[1]["id"].as_i64().unwrap(),
        "the log must be newest first"
    );
}
