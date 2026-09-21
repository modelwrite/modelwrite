// SPDX-License-Identifier: AGPL-3.0-or-later
//! End-to-end tests for the read-only analytics endpoint: it answers the portfolio
//! compliance question and the cost question over a REAL model from the store, refuses an
//! under-privileged caller before touching the store, 404s without leaking internal detail,
//! and - most importantly - writes NOTHING, proven by comparing the store's commits, audit
//! log and blobs before and after the request.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;

use server::auth::{AuthConfig, Identity};
use server::store::sqlite::SqliteStore;
use server::store::{AuditEntry, Commit, Store};
use server::{app, AppState};

/// The corpus fixture, parsed once per test where needed.
fn corpus() -> Value {
    serde_json::from_str(&test_support::load_okf_expected()).expect("corpus fixture parses")
}

fn req_ids(corpus: &Value) -> Vec<String> {
    corpus["requirements"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["id"].as_str().unwrap().to_string())
        .collect()
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

/// Create the project and commit the real corpus to `main` through the HTTP write path, so
/// the analytics endpoint is tested against a model that arrived the way every model does.
async fn setup() -> (axum::Router, Arc<SqliteStore>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(SqliteStore::open(&dir.path().join("mw.db")).unwrap());
    let router = app(AppState {
        store: store.clone(),
        evidence_dir: dir.path().to_path_buf(),
        auth: AuthConfig::Open,
    });
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
            json!({ "branch": "main", "author": "alex", "message": "import the corpus", "okf": corpus() }),
        ))
        .await
        .unwrap();
    assert_eq!(committed.status(), StatusCode::CREATED);
    (router, store, dir)
}

/// Seed a project and a commit DIRECTLY through the store, for tests whose router is pinned
/// to an identity that cannot write (and therefore cannot create the fixtures over HTTP).
fn seed(store: &SqliteStore) {
    store.create_project("coffee", None).unwrap();
    let bytes = test_support::load_okf_expected().into_bytes();
    let okf_hash = store.put_blob(&bytes).unwrap();
    store
        .commit_model(
            "coffee", "main", &okf_hash, "alex", "import", None, None, None,
        )
        .unwrap();
}

/// The observable store state a read-only request must leave untouched: every commit, every
/// audit entry, and every blob. Blobs are read from the same SQLite file the store manages,
/// because the trait exposes no blob enumeration - the file IS the record.
#[derive(Debug, PartialEq)]
struct Fingerprint {
    commits: Vec<Commit>,
    audit: Vec<AuditEntry>,
    blobs: Vec<(String, Vec<u8>)>,
}

fn fingerprint(dir: &std::path::Path, store: &SqliteStore, project: &str) -> Fingerprint {
    let mut commits = store.commits_on(project, "main").unwrap();
    commits.sort_by(|a, b| a.hash.cmp(&b.hash));
    let mut audit = store.audit(project, 1000).unwrap();
    audit.sort_by_key(|a| a.id);
    let conn = rusqlite::Connection::open(dir.join("mw.db")).unwrap();
    let mut stmt = conn
        .prepare("SELECT hash, bytes FROM blobs ORDER BY hash")
        .unwrap();
    let blobs = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?))
        })
        .unwrap()
        .collect::<Result<Vec<_>, rusqlite::Error>>()
        .unwrap();
    Fingerprint {
        commits,
        audit,
        blobs,
    }
}

#[tokio::test]
async fn a_real_model_reports_compliance_and_uncosted_and_writes_nothing() {
    let (router, store, dir) = setup().await;
    let corpus = corpus();
    let before = fingerprint(dir.path(), store.as_ref(), "coffee");

    // The specification is the model's own 25 requirements plus one it does not contain.
    let mut spec = req_ids(&corpus);
    spec.push("REQ-ABSENT-FROM-MODEL".to_string());
    let query = format!("requirements={}", spec.join(","));

    let response = router
        .oneshot(get(&format!("/projects/coffee/analytics?{}", query)))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;

    assert_eq!(body["project"], "coffee");
    assert_eq!(body["compliance"]["specificationSize"], 26);
    let model = &body["compliance"]["models"][0];
    assert_eq!(model["model"], corpus["project"]);
    // 15 of the corpus's 25 requirements are covered, 10 are uncovered - the same counts the
    // engine's own coverage reports - and the one absent requirement is UNKNOWN, in the
    // unknown bucket rather than omitted or blanked.
    assert_eq!(model["covered"].as_array().unwrap().len(), 15);
    assert_eq!(model["uncovered"].as_array().unwrap().len(), 10);
    assert_eq!(model["unknown"], json!(["REQ-ABSENT-FROM-MODEL"]));

    // No dataset was supplied, so every requirement is UNCOSTED - the explicit string, never
    // a zero and never omitted.
    let costs = body["costs"].as_array().unwrap();
    assert_eq!(costs.len(), 25);
    assert!(
        costs.iter().all(|c| c["cost"] == "Uncosted"),
        "an uncosted requirement must read UNCOSTED, never zero or blank"
    );

    assert_eq!(
        fingerprint(dir.path(), store.as_ref(), "coffee"),
        before,
        "the analytics endpoint must not write a commit, a blob or an audit entry"
    );
}

#[tokio::test]
async fn a_supplied_dataset_prices_a_requirement_and_labels_its_source() {
    let (router, _store, _dir) = setup().await;
    let corpus = corpus();
    let first = corpus["requirements"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let second = corpus["requirements"][1]["id"]
        .as_str()
        .unwrap()
        .to_string();

    // Price the FIRST requirement at 1200 from a reported ERP source read on a known date.
    let csv = format!(
        "requirement_id,unit_cost
{},1200
",
        first
    );
    let uri = format!(
        "/projects/coffee/analytics?requirements={},{}&costCsv={}&costSource=erp&costTrust=reported&costCapturedAt=2026-09-17T00:00:00Z",
        first,
        second,
        csv.replace('\n', "%0A")
    );

    let response = router.oneshot(get(&uri)).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;

    let costs = body["costs"].as_array().unwrap();
    assert_eq!(costs.len(), 25);
    assert_eq!(costs[0]["requirement"], first);
    let costed = &costs[0]["cost"]["Costed"];
    assert!(
        costs[0]["cost"].get("Costed").is_some(),
        "a requirement with a record must be Costed, not Uncosted"
    );
    assert_eq!(costed["value"], "1200");
    assert_eq!(costed["trust"], "Reported");
    assert_eq!(costed["sources"], json!(["erp"]));
    assert_eq!(costed["captured_at"]["erp"], "2026-09-17T00:00:00Z");
    assert_eq!(costed["trusts"]["erp"], "Reported");

    // Every other requirement stays UNCOSTED - still present, never zero.
    assert_eq!(costs[1]["requirement"], second);
    assert_eq!(costs[1]["cost"], "Uncosted");
    assert!(costs[2..].iter().all(|c| c["cost"] == "Uncosted"));
}

#[tokio::test]
async fn a_caller_without_read_is_refused_before_the_store_is_touched() {
    // An identity that reaches the project but holds no Read role gets a refusal, never an
    // empty report - and the refusal happens before any store access.
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(SqliteStore::open(&dir.path().join("mw.db")).unwrap());
    seed(store.as_ref());
    let before = fingerprint(dir.path(), store.as_ref(), "coffee");

    let nobody = Identity {
        subject: "nobody".to_string(),
        roles: Vec::new(),
        projects: vec!["*".to_string()],
        trial_id: None,
    };
    let router = app(AppState {
        store: store.clone(),
        evidence_dir: dir.path().to_path_buf(),
        auth: AuthConfig::fixed(nobody),
    });

    let response = router
        .oneshot(get("/projects/coffee/analytics?requirements=r1"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let body = json_body(response).await;
    assert!(body["error"].is_string());

    assert_eq!(
        fingerprint(dir.path(), store.as_ref(), "coffee"),
        before,
        "a refusal must happen before the store is touched"
    );
}

#[tokio::test]
async fn a_scoped_caller_is_refused_for_a_project_it_may_not_reach() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(SqliteStore::open(&dir.path().join("mw.db")).unwrap());
    seed(store.as_ref());

    // A viewer who may read, but only the "tea" project - "coffee" is out of scope.
    let scoped = Identity {
        subject: "viewer".to_string(),
        roles: vec!["viewer".to_string()],
        projects: vec!["tea".to_string()],
        trial_id: None,
    };
    let router = app(AppState {
        store: store.clone(),
        evidence_dir: dir.path().to_path_buf(),
        auth: AuthConfig::fixed(scoped),
    });

    let response = router
        .oneshot(get("/projects/coffee/analytics?requirements=r1"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let body = json_body(response).await;
    assert_eq!(body["error"], "project not in scope");
}

#[tokio::test]
async fn analytics_resolves_a_named_commit_and_branch_and_404s_on_missing() {
    let (router, _store, _dir) = setup().await;
    let corpus = corpus();
    let id = corpus["requirements"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();

    // The default branch "main" resolves to the tip.
    let response = router
        .clone()
        .oneshot(get(&format!(
            "/projects/coffee/analytics?requirements={}",
            id
        )))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    assert_eq!(body["branch"], "main");
    let tip = body["commit"].as_str().unwrap().to_string();

    // Naming the commit directly reads the SAME model.
    let response = router
        .clone()
        .oneshot(get(&format!(
            "/projects/coffee/analytics?requirements={}&commit={}",
            id, tip
        )))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    assert_eq!(body["commit"], tip);

    // A missing project, commit or branch is a 404 naming only what the caller asked for.
    for (uri, want) in [
        (
            format!(
                "/projects/coffee/analytics?requirements={}&commit=0000000000000000",
                id
            ),
            StatusCode::NOT_FOUND,
        ),
        (
            format!("/projects/coffee/analytics?requirements={}&branch=nope", id),
            StatusCode::NOT_FOUND,
        ),
        (
            "/projects/nope/analytics?requirements=r1".to_string(),
            StatusCode::NOT_FOUND,
        ),
    ] {
        let response = router.clone().oneshot(get(&uri)).await.unwrap();
        assert_eq!(response.status(), want, "{} must 404", uri);
        let body = json_body(response).await;
        assert!(body["error"].is_string());
        assert!(
            !body["error"].as_str().unwrap().contains("blob"),
            "a 404 must not leak internal storage detail"
        );
    }
}

#[tokio::test]
async fn an_empty_specification_is_a_bad_request() {
    let (router, _store, _dir) = setup().await;
    let response = router
        .oneshot(get("/projects/coffee/analytics"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = json_body(response).await;
    assert!(body["error"].is_string());
}
