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

async fn commit(
    router: &axum::Router,
    branch: &str,
    message: &str,
    okf: serde_json::Value,
) -> String {
    let response = router
        .clone()
        .oneshot(post(
            "/projects/coffee/commits",
            serde_json::json!({ "branch": branch, "author": "alex", "message": message, "okf": okf }),
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
async fn the_corpus_can_be_committed_branched_and_gated() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));

    router
        .clone()
        .oneshot(post("/projects", serde_json::json!({ "name": "coffee" })))
        .await
        .unwrap();

    let expected: serde_json::Value =
        serde_json::from_str(&test_support::load_okf_expected()).unwrap();
    let broken: serde_json::Value = serde_json::from_str(&test_support::load_okf_broken()).unwrap();

    let imported = commit(&router, "main", "import the exported model", expected).await;
    let branched = router
        .clone()
        .oneshot(post(
            "/projects/coffee/branches",
            serde_json::json!({ "name": "corrupted", "from": imported }),
        ))
        .await
        .unwrap();
    assert_eq!(
        branched.status(),
        StatusCode::CREATED,
        "the branch must exist before the lossy commit lands on it"
    );
    let corrupted = commit(&router, "corrupted", "drop a requirement", broken).await;
    // The lossy commit must DESCEND from the imported one. Without this, a broken branch
    // or parentage would still pass: the gate detects loss by content, not by topology,
    // and a commit onto a branch that did not exist would simply start a new history.
    assert_ne!(imported, corrupted);

    let clean = router
        .clone()
        .oneshot(post(
            "/projects/coffee/gate",
            serde_json::json!({ "reference": imported, "candidate": imported }),
        ))
        .await
        .unwrap();
    assert_eq!(clean.status(), StatusCode::OK);
    let clean = json_body(clean).await;
    assert_eq!(clean["passed"], true);
    assert_eq!(clean["integration"]["componentCount"], 1);
    assert_eq!(clean["coverage"]["total"], 25);

    let lossy = router
        .clone()
        .oneshot(post(
            "/projects/coffee/gate",
            serde_json::json!({ "reference": imported, "candidate": corrupted }),
        ))
        .await
        .unwrap();
    assert_eq!(
        lossy.status(),
        StatusCode::OK,
        "a failed gate is a successful run"
    );
    let lossy = json_body(lossy).await;
    assert_eq!(lossy["passed"], false);
    let failures: Vec<String> = lossy["failures"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f.as_str().unwrap().to_string())
        .collect();
    assert!(failures.iter().any(|f| f.contains("missing elements")));
    assert!(failures.iter().any(|f| f.contains("isolated")));

    let runs = router
        .oneshot(
            Request::builder()
                .uri("/projects/coffee/gate-runs")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let runs = json_body(runs).await;
    assert_eq!(runs.as_array().unwrap().len(), 2);
}
