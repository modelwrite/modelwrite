// SPDX-License-Identifier: AGPL-3.0-or-later
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;

use server::auth::{AuthConfig, Identity};
use server::store::{sqlite::SqliteStore, Store};

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

/// An authenticated identity pinned to the given subject with the admin role, so it can
/// create projects, commits, branches, gate runs and delete branches.
fn identity(subject: &str) -> Identity {
    Identity {
        subject: subject.to_string(),
        roles: vec!["admin".to_string()],
        projects: vec!["*".to_string()],
    }
}

/// Build the full application router pinned to one identity, returning a handle to the
/// store so a test can prove a refused request recorded nothing. The tempdir is returned so
/// it stays alive for as long as the router (and its open SQLite connection) is used.
fn app_with_identity(identity: Identity) -> (axum::Router, Arc<SqliteStore>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(SqliteStore::open(&dir.path().join("mw.db")).unwrap());
    let state = server::AppState {
        store: store.clone(),
        evidence_dir: dir.path().to_path_buf(),
        auth: AuthConfig::fixed(identity),
    };
    (server::app(state), store, dir)
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

#[tokio::test]
async fn every_unknown_actor_path_now_names_the_identity() {
    let (router, _store, _dir) = app_with_identity(identity("alex"));

    // project.create
    let created = router
        .clone()
        .oneshot(post("/projects", json!({ "name": "coffee" })))
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::CREATED);

    // commit.create (base), then branch.create, gate.run and branch.delete.
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

    let gated = router
        .clone()
        .oneshot(post(
            "/projects/coffee/gate",
            json!({ "reference": base, "candidate": base }),
        ))
        .await
        .unwrap();
    assert_eq!(gated.status(), StatusCode::OK);

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

    // Every previously-"unknown" action now names the verified identity.
    let entries = read_audit(&router).await;
    for action in [
        "project.create",
        "branch.create",
        "gate.run",
        "branch.delete",
    ] {
        let entry = entries
            .iter()
            .find(|e| e["action"] == action)
            .unwrap_or_else(|| panic!("missing audit action {}", action));
        assert_eq!(
            entry["actor"], "alex",
            "{} must name the verified identity, not \"unknown\"",
            action
        );
    }
}

#[tokio::test]
async fn an_authenticated_commit_records_the_identity() {
    let (router, _store, _dir) = app_with_identity(identity("alex"));
    let created = router
        .clone()
        .oneshot(post("/projects", json!({ "name": "coffee" })))
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::CREATED);

    let committed = router
        .clone()
        .oneshot(post(
            "/projects/coffee/commits",
            json!({ "branch": "main", "author": "alex", "message": "base", "okf": model("Block") }),
        ))
        .await
        .unwrap();
    assert_eq!(committed.status(), StatusCode::CREATED);

    let entries = read_audit(&router).await;
    let commit_entry = entries
        .iter()
        .find(|e| e["action"] == "commit.create")
        .expect("a commit.create entry must be recorded");
    assert_eq!(
        commit_entry["actor"], "alex",
        "the commit must be attributed to the verified identity"
    );
}

#[tokio::test]
async fn a_body_naming_another_actor_is_refused_and_records_nothing() {
    let (router, store, _dir) = app_with_identity(identity("alex"));
    // Seed the project directly so a denied commit is observable as "nothing changed".
    store.create_project("coffee", None).unwrap();

    let response = router
        .clone()
        .oneshot(post(
            "/projects/coffee/commits",
            json!({ "branch": "main", "author": "someone-else", "message": "m", "okf": model("Block") }),
        ))
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::FORBIDDEN,
        "a body naming another actor must be refused with 403"
    );

    assert!(
        store.commits_on("coffee", "main").unwrap().is_empty(),
        "a refused commit must write no commit"
    );
    assert!(
        store.audit("coffee", 1000).unwrap().is_empty(),
        "a refused commit must write no audit entry"
    );
}

#[tokio::test]
async fn a_holder_naming_another_actor_is_refused_and_records_nothing() {
    let (router, store, _dir) = app_with_identity(identity("alex"));
    store.create_project("coffee", None).unwrap();

    let response = router
        .clone()
        .oneshot(post(
            "/projects/coffee/locks",
            json!({ "branch": "main", "elements": ["b1"], "holder": "someone-else", "ttlSeconds": 300 }),
        ))
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::FORBIDDEN,
        "a holder naming another actor must be refused with 403"
    );

    assert!(
        store.locks("coffee", 0).unwrap().is_empty(),
        "a refused acquire must create no lock"
    );
    assert!(
        store.audit("coffee", 1000).unwrap().is_empty(),
        "a refused acquire must write no audit entry"
    );
}

#[tokio::test]
async fn open_mode_records_anonymous() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));

    let created = router
        .clone()
        .oneshot(post("/projects", json!({ "name": "coffee" })))
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::CREATED);

    // The body still claims "alex" and is accepted (open mode has no "someone else"), but
    // the audit must record the honest subject: nobody was authenticated.
    let committed = router
        .clone()
        .oneshot(post(
            "/projects/coffee/commits",
            json!({ "branch": "main", "author": "alex", "message": "base", "okf": model("Block") }),
        ))
        .await
        .unwrap();
    assert_eq!(committed.status(), StatusCode::CREATED);

    let entries = read_audit(&router).await;
    assert!(!entries.is_empty(), "open mode must still record actions");
    for entry in &entries {
        assert_eq!(
            entry["actor"], "anonymous",
            "open mode must record the honest anonymous subject"
        );
    }
}
