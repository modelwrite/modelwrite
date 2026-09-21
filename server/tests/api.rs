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

fn tiny_okf() -> serde_json::Value {
    serde_json::json!({
        "project": "tiny",
        "exportedAt": "2026-09-17T00:00:00Z",
        "summary": { "graphNodes": 1 },
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
    // The document is committed in its ONE canonical form (the summary re-derived and the
    // field order fixed), so the fetched document equals the canonical serialisation of the
    // posted document - never the caller's serialisation order. The canonical bytes are
    // checked byte for byte in the cross-path test; here the GET route re-serialises the blob
    // through a serde_json::Value, so the comparison is structural.
    let root: okf::types::OkfRoot = serde_json::from_value(tiny_okf()).unwrap();
    let canonical: serde_json::Value =
        serde_json::from_slice(&okf::hash::canonical_bytes(&root)).unwrap();
    assert_eq!(json_body(fetched).await, canonical);
}

#[tokio::test]
async fn a_commit_regenerates_a_stale_summary_to_match_its_content() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    router
        .clone()
        .oneshot(post("/projects", serde_json::json!({ "name": "coffee" })))
        .await
        .unwrap();

    // A document whose summary under-counts its graph: one node, claimed zero.
    let mut okf = tiny_okf();
    okf["summary"] = serde_json::json!({ "graphNodes": 0 });
    okf["structure"] = serde_json::json!([
        { "id": "b1", "name": "B1", "kind": "block", "stereotypes": [], "attributes": [], "documentation": "" }
    ]);

    let committed = router
        .clone()
        .oneshot(post(
            "/projects/coffee/commits",
            serde_json::json!({
                "branch": "main",
                "author": "alex",
                "message": "stale summary",
                "okf": okf
            }),
        ))
        .await
        .unwrap();
    assert_eq!(committed.status(), StatusCode::CREATED);
    let hash = json_body(committed).await["hash"]
        .as_str()
        .unwrap()
        .to_string();

    let fetched = router
        .oneshot(
            Request::builder()
                .uri(format!("/projects/coffee/commits/{}", hash))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let stored = json_body(fetched).await;
    // The summary is DERIVED, not trusted: the stored model's counts agree with its content.
    assert_eq!(stored["summary"]["graphNodes"], 1);
    assert_eq!(stored["summary"]["blocks"], 1);
    assert_eq!(stored["summary"]["graphEdges"], 0);
    assert_eq!(stored["summary"]["requirements"], 0);
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
#[tokio::test]
async fn names_that_could_escape_a_path_are_rejected() {
    // Project names reach the evidence file path, so the charset is closed rather than
    // trusted: a separator or a drive letter must never be accepted.
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    for name in ["..", "a/b", "a\\b", "has space", "colon:name", ""] {
        let response = router
            .clone()
            .oneshot(post("/projects", serde_json::json!({ "name": name })))
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::BAD_REQUEST,
            "name {:?} must be rejected",
            name
        );
    }

    let accepted = router
        .oneshot(post(
            "/projects",
            serde_json::json!({ "name": "coffee-machine.v2" }),
        ))
        .await
        .unwrap();
    assert_eq!(accepted.status(), StatusCode::CREATED);
}

#[tokio::test]
async fn branches_can_be_listed_and_deleted_without_losing_commits() {
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
    router
        .clone()
        .oneshot(post(
            "/projects/coffee/branches",
            serde_json::json!({ "name": "review", "from": hash }),
        ))
        .await
        .unwrap();

    let listed = router
        .clone()
        .oneshot(get("/projects/coffee/branches"))
        .await
        .unwrap();
    let branches = json_body(listed).await;
    let names: Vec<String> = branches
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["name"].as_str().unwrap().to_string())
        .collect();
    assert!(names.contains(&"main".to_string()));
    assert!(names.contains(&"review".to_string()));

    let deleted = router
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/projects/coffee/branches/review")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(deleted.status(), StatusCode::NO_CONTENT);

    // The commit the deleted branch pointed at must still be readable.
    let still_there = router
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/projects/coffee/commits/{}", hash))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(still_there.status(), StatusCode::OK);

    let gone = router
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/projects/coffee/branches/review")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(gone.status(), StatusCode::NOT_FOUND);
}
#[tokio::test]
async fn branch_routes_report_an_unknown_project() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));

    let listed = router
        .clone()
        .oneshot(get("/projects/nope/branches"))
        .await
        .unwrap();
    assert_eq!(listed.status(), StatusCode::NOT_FOUND);

    let deleted = router
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/projects/nope/branches/main")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(deleted.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_reset_appends_a_commit_and_keeps_the_old_tip_reachable() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    router
        .clone()
        .oneshot(post("/projects", serde_json::json!({ "name": "coffee" })))
        .await
        .unwrap();
    let first = router
        .clone()
        .oneshot(post(
            "/projects/coffee/commits",
            serde_json::json!({ "branch": "main", "author": "alex", "message": "one", "okf": tiny_okf() }),
        ))
        .await
        .unwrap();
    let first_hash = json_body(first).await["hash"].as_str().unwrap().to_string();

    let mut second_model = tiny_okf();
    second_model["project"] = serde_json::json!("changed");
    let second = router
        .clone()
        .oneshot(post(
            "/projects/coffee/commits",
            serde_json::json!({ "branch": "main", "author": "alex", "message": "two", "okf": second_model }),
        ))
        .await
        .unwrap();
    let second_hash = json_body(second).await["hash"]
        .as_str()
        .unwrap()
        .to_string();

    let reset = router
        .clone()
        .oneshot(post(
            "/projects/coffee/branches/main/reset",
            serde_json::json!({ "to": first_hash, "author": "alex", "message": "revert to one" }),
        ))
        .await
        .unwrap();
    assert_eq!(reset.status(), StatusCode::CREATED);
    let revert = json_body(reset).await;
    assert_eq!(revert["parents"], serde_json::json!([second_hash]));

    // The reverted content matches the target, and the superseded commit is still there.
    let tip = revert["hash"].as_str().unwrap().to_string();
    let fetched = router
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/projects/coffee/commits/{}", tip))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let root: okf::types::OkfRoot = serde_json::from_value(tiny_okf()).unwrap();
    let canonical: serde_json::Value =
        serde_json::from_slice(&okf::hash::canonical_bytes(&root)).unwrap();
    assert_eq!(json_body(fetched).await, canonical);

    let superseded = router
        .oneshot(
            Request::builder()
                .uri(format!("/projects/coffee/commits/{}", second_hash))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(superseded.status(), StatusCode::OK);
}
#[tokio::test]
async fn a_reset_reports_an_unknown_branch_or_target() {
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
            serde_json::json!({ "branch": "main", "author": "alex", "message": "one", "okf": tiny_okf() }),
        ))
        .await
        .unwrap();
    let hash = json_body(committed).await["hash"]
        .as_str()
        .unwrap()
        .to_string();

    let unknown_branch = router
        .clone()
        .oneshot(post(
            "/projects/coffee/branches/nope/reset",
            serde_json::json!({ "to": hash, "author": "alex", "message": "revert" }),
        ))
        .await
        .unwrap();
    assert_eq!(unknown_branch.status(), StatusCode::NOT_FOUND);

    let unknown_target = router
        .oneshot(post(
            "/projects/coffee/branches/main/reset",
            serde_json::json!({ "to": "no-such-commit", "author": "alex", "message": "revert" }),
        ))
        .await
        .unwrap();
    assert_eq!(unknown_target.status(), StatusCode::NOT_FOUND);
}
