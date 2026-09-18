// SPDX-License-Identifier: AGPL-3.0-or-later
//! A commit knows how it arrived. The provenance written in the SAME transaction as the
//! commit says authored (a normal commit or edit), imported (a migration read from a retained
//! artifact through a binding, carrying the artifact hash, binding id and version and the
//! accepted losses), or unknown (a commit written before provenance existed). A reader can
//! tell the three apart from the commit alone, and absence never reads as a claim: a commit
//! with no recorded provenance reads as unknown, never as authored.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use server::store::{CommitProvenance, Store};
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
        "summary": {},
        "stateMachine": { "name": "tiny sm", "regions": [] },
        "graph": { "nodes": [{ "id": "b1", "kind": "block", "name": "B1" }], "edges": [] }
    })
}

/// Read a hand-written SysML v1 XMI fixture from the binding's corpus.
fn fixture(name: &str) -> Vec<u8> {
    let path = format!(
        "{}/../engine/binding-xmi/fixtures/{}",
        env!("CARGO_MANIFEST_DIR"),
        name
    );
    std::fs::read(path).expect("fixture must exist")
}

fn import_body(name: &str, accept_losses: &[&str]) -> serde_json::Value {
    let bytes = fixture(name);
    let mut body = serde_json::json!({ "artifact": String::from_utf8(bytes).unwrap() });
    body["binding"] = serde_json::json!("sysml-v1-xmi@2.4");
    body["branch"] = serde_json::json!("main");
    body["author"] = serde_json::json!("alex");
    body["message"] = serde_json::json!(format!("import {}", name));
    body["acceptLosses"] = serde_json::json!(accept_losses);
    body
}

/// The six blocking losses of coffee-grinder.xmi, named by their raw subjects.
const COFFEE_LOSSES: [&str; 6] = [
    "uml:Model model-grinder",
    "uml:Comment doc-grinder",
    "uml:Property prop-motor",
    "uml:Property prop-capacity",
    "uml:Dependency dep-satisfy",
    "uml:Package pkg-structure (Structure)",
];

#[tokio::test]
async fn a_normal_commit_records_authored() {
    // A commit made by the editor or the plain commit endpoint flows through the shared
    // commit core with no import, so it is labelled authored - never left for a reader to
    // guess and never defaulted to it from absence.
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
                "message": "typed",
                "okf": tiny_okf()
            }),
        ))
        .await
        .unwrap();
    assert_eq!(committed.status(), StatusCode::CREATED);
    let body = json_body(committed).await;
    assert_eq!(body["provenance"]["kind"], "authored");
    assert!(
        body["provenance"].get("artifactHash").is_none(),
        "an authored commit must not carry an artifact hash"
    );

    // The same provenance is readable from the store: the commit row itself records it.
    let store = server::store::sqlite::SqliteStore::open(&dir.path().join("mw.db")).unwrap();
    let hash = body["hash"].as_str().unwrap();
    let commit = store
        .commit("coffee", hash)
        .unwrap()
        .expect("commit must exist");
    assert_eq!(commit.provenance, CommitProvenance::Authored);
}

#[tokio::test]
async fn an_import_records_imported_with_its_artifact_and_binding() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    router
        .clone()
        .oneshot(post("/projects", serde_json::json!({ "name": "coffee" })))
        .await
        .unwrap();

    let bytes = fixture("coffee-grinder.xmi");
    let expected_hash = server::store::blob_hash(&bytes);

    let response = router
        .clone()
        .oneshot(post(
            "/projects/coffee/import",
            import_body("coffee-grinder.xmi", &COFFEE_LOSSES),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED, "{:?}", response);
    let body = json_body(response).await;

    let provenance = &body["commit"]["provenance"];
    assert_eq!(provenance["kind"], "imported");
    assert_eq!(provenance["artifactHash"], expected_hash.as_str());
    assert_eq!(provenance["bindingId"], "sysml-v1-xmi");
    assert_eq!(provenance["bindingVersion"], "2.4");
    assert!(provenance["acceptedLosses"].is_array());

    // The store's commit row carries the same imported provenance, including the binding.
    let store = server::store::sqlite::SqliteStore::open(&dir.path().join("mw.db")).unwrap();
    let hash = body["commit"]["hash"].as_str().unwrap();
    let commit = store
        .commit("coffee", hash)
        .unwrap()
        .expect("commit must exist");
    match commit.provenance {
        CommitProvenance::Imported {
            artifact_hash,
            binding_id,
            binding_version,
            accepted_losses,
        } => {
            assert_eq!(artifact_hash, expected_hash);
            assert_eq!(binding_id, "sysml-v1-xmi");
            assert_eq!(binding_version, "2.4");
            assert!(!accepted_losses.is_empty());
        }
        other => panic!("expected imported provenance, got {:?}", other),
    }
}

#[test]
fn a_commit_written_before_provenance_reads_as_unknown() {
    // A database created by an earlier build has a commits table with no provenance column.
    // The migration adds the column; the pre-existing rows stay NULL and read as unknown, so
    // absence is never upgraded to a claim that the commit was authored.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("old.db");
    {
        let connection = rusqlite::Connection::open(&path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE commits (hash TEXT PRIMARY KEY, project TEXT NOT NULL, branch TEXT NOT NULL, parents TEXT NOT NULL, okf_hash TEXT NOT NULL, author TEXT NOT NULL, message TEXT NOT NULL, created_at TEXT NOT NULL)",
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO commits (hash, project, branch, parents, okf_hash, author, message, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                rusqlite::params!["old-hash", "coffee", "main", "[]", "okf-hash", "alex", "before provenance", "1"],
            )
            .unwrap();
    }

    let store = server::store::sqlite::SqliteStore::open(&path)
        .expect("an older database must open, not fail");
    let commit = store
        .commit("coffee", "old-hash")
        .unwrap()
        .expect("the old commit must be readable");
    assert_eq!(commit.provenance, CommitProvenance::Unknown);

    // The JSON says unknown, not authored: absence is not a claim.
    let json = server::api::commit_json(&commit);
    assert_eq!(json["provenance"]["kind"], "unknown");
    assert!(json["provenance"].get("artifactHash").is_none());
}

#[test]
fn authored_imported_and_unknown_are_distinguishable_from_the_commit_alone() {
    // The three kinds serialize to distinct values, and only an import carries an artifact
    // hash: a reader never has to guess which of authored, imported or unknown a commit is.
    let authored = serde_json::to_value(CommitProvenance::Authored).unwrap();
    let imported = serde_json::to_value(CommitProvenance::Imported {
        artifact_hash: "a".to_string(),
        binding_id: "b".to_string(),
        binding_version: "1".to_string(),
        accepted_losses: vec![],
    })
    .unwrap();
    let unknown = serde_json::to_value(CommitProvenance::Unknown).unwrap();

    assert_eq!(authored["kind"], "authored");
    assert_eq!(imported["kind"], "imported");
    assert_eq!(unknown["kind"], "unknown");
    assert!(authored.get("artifactHash").is_none());
    assert_eq!(imported["artifactHash"], "a");
}

#[tokio::test]
async fn the_retained_artifact_is_fetchable_by_its_hash() {
    // "Retained byte-for-byte" is only a real promise if a reader can FETCH the bytes. After
    // an import, the retained source artifact must be reachable by its content address from a
    // project-scoped, Read-permission route, and the returned bytes must equal the source.
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    router
        .clone()
        .oneshot(post("/projects", serde_json::json!({ "name": "coffee" })))
        .await
        .unwrap();

    let bytes = fixture("coffee-grinder.xmi");
    let expected_hash = server::store::blob_hash(&bytes);

    let response = router
        .clone()
        .oneshot(post(
            "/projects/coffee/import",
            import_body("coffee-grinder.xmi", &COFFEE_LOSSES),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED, "{:?}", response);

    let fetched = router
        .clone()
        .oneshot(get(&format!(
            "/projects/coffee/import/{}/artifact",
            expected_hash
        )))
        .await
        .unwrap();
    assert_eq!(fetched.status(), StatusCode::OK, "{:?}", fetched);
    let body = fetched.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(
        body.as_ref(),
        bytes,
        "the fetched artifact must be byte-for-byte the source"
    );
}

#[tokio::test]
async fn provenance_is_reachable_by_commit_hash_alone() {
    // Provenance must be reachable from the commit hash, not only by listing a whole branch
    // and finding the hash. The record route returns the commit row (including provenance) for
    // one hash, so a reader can check an imported commit's source from the hash alone.
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    router
        .clone()
        .oneshot(post("/projects", serde_json::json!({ "name": "coffee" })))
        .await
        .unwrap();

    let response = router
        .clone()
        .oneshot(post(
            "/projects/coffee/import",
            import_body("coffee-grinder.xmi", &COFFEE_LOSSES),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED, "{:?}", response);
    let body = json_body(response).await;
    let hash = body["commit"]["hash"].as_str().unwrap();

    let record = router
        .clone()
        .oneshot(get(&format!("/projects/coffee/commits/{}/record", hash)))
        .await
        .unwrap();
    assert_eq!(record.status(), StatusCode::OK, "{:?}", record);
    let record = json_body(record).await;
    assert_eq!(record["hash"], hash);
    assert_eq!(record["provenance"]["kind"], "imported");
    assert_eq!(
        record["provenance"]["artifactHash"],
        body["commit"]["provenance"]["artifactHash"]
    );
    assert_eq!(record["provenance"]["bindingId"], "sysml-v1-xmi");
}
