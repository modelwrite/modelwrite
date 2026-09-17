// SPDX-License-Identifier: AGPL-3.0-or-later
//! The merge endpoint end to end: a clean merge writes a two-parent commit, a conflict
//! writes nothing and names the conflicts, and the branch tips are left as they were.

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
                {"id": "b1", "kind": "block", "name": "Block"},
                {"id": "r1", "kind": "requirement", "name": "r1"}
            ],
            "edges": [{"source": "b1", "target": "r1", "kind": "dependency", "label": "Satisfy"}]
        }
    })
}

async fn seed(router: &axum::Router) -> String {
    router
        .clone()
        .oneshot(post("/projects", json!({ "name": "coffee" })))
        .await
        .unwrap();
    let committed = router
        .clone()
        .oneshot(post(
            "/projects/coffee/commits",
            json!({ "branch": "main", "author": "alex", "message": "base", "okf": model("Block") }),
        ))
        .await
        .unwrap();
    let hash = json_body(committed).await["hash"]
        .as_str()
        .unwrap()
        .to_string();
    router
        .clone()
        .oneshot(post(
            "/projects/coffee/branches",
            json!({ "name": "feature", "from": hash }),
        ))
        .await
        .unwrap();
    hash
}

#[tokio::test]
async fn a_clean_merge_writes_a_two_parent_commit() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    let _base = seed(&router).await;

    // The feature branch renames the block; main is untouched.
    router
        .clone()
        .oneshot(post(
            "/projects/coffee/commits",
            json!({ "branch": "feature", "author": "alex", "message": "rename", "okf": model("Renamed") }),
        ))
        .await
        .unwrap();

    let merged = router
        .clone()
        .oneshot(post(
            "/projects/coffee/merge",
            json!({ "branch": "main", "other": "feature", "author": "alex", "message": "merge feature" }),
        ))
        .await
        .unwrap();
    assert_eq!(merged.status(), StatusCode::CREATED);
    let body = json_body(merged).await;
    assert_eq!(body["commit"]["parents"].as_array().unwrap().len(), 2);

    // The merged model is what the branch now serves, and it carries the rename.
    let tip = body["commit"]["hash"].as_str().unwrap().to_string();
    let fetched = router
        .oneshot(get(&format!("/projects/coffee/commits/{}", tip)))
        .await
        .unwrap();
    let served = json_body(fetched).await;
    assert_eq!(served["structure"][0]["name"], "Renamed");
}

#[tokio::test]
async fn a_conflicting_merge_writes_nothing_and_names_every_conflict() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    seed(&router).await;

    router
        .clone()
        .oneshot(post(
            "/projects/coffee/commits",
            json!({ "branch": "feature", "author": "alex", "message": "theirs", "okf": model("Theirs") }),
        ))
        .await
        .unwrap();
    router
        .clone()
        .oneshot(post(
            "/projects/coffee/commits",
            json!({ "branch": "main", "author": "alex", "message": "ours", "okf": model("Ours") }),
        ))
        .await
        .unwrap();

    let conflicted = router
        .clone()
        .oneshot(post(
            "/projects/coffee/merge",
            json!({ "branch": "main", "other": "feature", "author": "alex", "message": "merge" }),
        ))
        .await
        .unwrap();
    assert_eq!(conflicted.status(), StatusCode::CONFLICT);
    let body = json_body(conflicted).await;
    let conflicts = body["conflicts"].as_array().unwrap();
    assert_eq!(conflicts.len(), 1);
    assert_eq!(conflicts[0]["kind"], "bothModified");

    // Nothing was written: main still points at our commit, not at a merge.
    let history = router
        .oneshot(get("/projects/coffee/commits?branch=main"))
        .await
        .unwrap();
    let history = json_body(history).await;
    let commits = history.as_array().unwrap();
    assert_eq!(
        commits.len(),
        2,
        "the base commit and ours, and no merge commit"
    );
    for commit in commits {
        assert!(
            commit["parents"].as_array().unwrap().len() < 2,
            "a conflict must not leave a merge commit"
        );
    }
}

#[tokio::test]
async fn merging_an_unknown_branch_is_not_found() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    seed(&router).await;
    let response = router
        .oneshot(post(
            "/projects/coffee/merge",
            json!({ "branch": "main", "other": "nope", "author": "alex", "message": "merge" }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}
