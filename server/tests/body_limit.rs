// SPDX-License-Identifier: AGPL-3.0-or-later
//! The request-body limit: configurable through `MW_MAX_BODY_BYTES`, applied to every
//! body-consuming route, and refused with a USEFUL 413 that names the configured limit and
//! the size received. These tests drive the refusal with a SMALL limit through
//! `app_with_limit` (no 512 MB fixture) and pin the documented default to its value.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use server::store::Store;
use tower::ServiceExt;

fn state(dir: &std::path::Path) -> server::AppState {
    let store = server::store::sqlite::SqliteStore::open(&dir.join("mw.db")).unwrap();
    server::AppState {
        store: std::sync::Arc::new(store),
        evidence_dir: dir.to_path_buf(),
        auth: server::auth::AuthConfig::Open,
    }
}

fn post_json(uri: &str, body: serde_json::Value, content_length: usize) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .header("content-length", content_length.to_string())
        .body(Body::from(body.to_string()))
        .unwrap()
}

async fn json_body(response: axum::response::Response) -> serde_json::Value {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

async fn create_project(router: &axum::Router, name: &str) {
    let response = router
        .clone()
        .oneshot(post_json(
            "/projects",
            serde_json::json!({ "name": name }),
            64,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
}

/// A `multipart/form-data` body carrying one file field, built by hand so the test never
/// needs a multipart crate and can drive the streaming path with an exact artifact size.
fn multipart_import_body(artifact: &[u8]) -> (String, Vec<u8>) {
    let boundary = "mw-test-boundary";
    let mut body = Vec::new();
    for (name, value) in [
        ("binding", "sysml-v1-xmi@2.4"),
        ("branch", "main"),
        ("message", "streamed import"),
    ] {
        body.extend_from_slice(format!("--{}\r\n", boundary).as_bytes());
        body.extend_from_slice(
            format!(
                "Content-Disposition: form-data; name=\"{}\"\r\n\r\n{}\r\n",
                name, value
            )
            .as_bytes(),
        );
    }
    body.extend_from_slice(format!("--{}\r\n", boundary).as_bytes());
    body.extend_from_slice(
        b"Content-Disposition: form-data; name=\"artifact\"; filename=\"model.xmi\"\r\n",
    );
    body.extend_from_slice(b"Content-Type: application/octet-stream\r\n\r\n");
    body.extend_from_slice(artifact);
    body.extend_from_slice(format!("\r\n--{}--\r\n", boundary).as_bytes());
    let content_type = format!("multipart/form-data; boundary={}", boundary);
    (content_type, body)
}

fn post_multipart(uri: &str, content_type: &str, body: Vec<u8>) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", content_type)
        .body(Body::from(body))
        .unwrap()
}

#[test]
fn the_documented_default_body_limit_is_512_mib() {
    assert_eq!(
        server::DEFAULT_MAX_BODY_BYTES,
        512 * 1024 * 1024,
        "the documented default must be 512 MiB"
    );
}

#[tokio::test]
async fn the_json_import_refuses_an_oversize_body_naming_the_limit_and_size() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app_with_limit(state(dir.path()), 1024);
    create_project(&router, "coffee").await;

    // The artifact is larger than the 1 KiB limit this test configures; the request also
    // declares its exact size, so the refusal can name it.
    let big_artifact = "x".repeat(4096);
    let body = serde_json::json!({
        "binding": "sysml-v1-xmi@2.4",
        "branch": "main",
        "author": "alex",
        "message": "too big",
        "artifact": big_artifact
    });
    let body_len = body.to_string().len();

    let response = router
        .oneshot(post_json("/projects/coffee/import", body, body_len))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    let value = json_body(response).await;
    assert_eq!(
        value["limit"], 1024,
        "the 413 must name the configured limit"
    );
    assert_eq!(
        value["received"], body_len as u64,
        "the 413 must name the size received"
    );
    assert!(
        value["error"].as_str().unwrap().contains("1024"),
        "the message must name the limit"
    );
}

#[tokio::test]
async fn the_streaming_import_refuses_an_oversize_artifact_naming_the_limit() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app_with_limit(state(dir.path()), 1024);
    create_project(&router, "coffee").await;

    // No Content-Length is declared, so the refusal comes from the streaming count itself,
    // not from a header precheck.
    let (content_type, body) = multipart_import_body(&vec![b'x'; 8192]);
    let response = router
        .oneshot(post_multipart(
            "/projects/coffee/import/stream",
            &content_type,
            body,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    let value = json_body(response).await;
    assert_eq!(
        value["limit"], 1024,
        "the 413 must name the configured limit"
    );
    assert!(
        value["received"].is_number(),
        "the 413 must carry the received size"
    );
}

#[tokio::test]
async fn the_streaming_import_retains_and_reads_a_real_artifact() {
    // The streaming endpoint, exercised end-to-end with the small hand-written fixture that
    // the JSON endpoint's own tests use, so the two paths are pinned to the same result.
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    create_project(&router, "coffee").await;

    let fixture = std::fs::read(format!(
        "{}/../engine/binding-xmi/fixtures/coffee-grinder.xmi",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    let expected_hash = server::store::blob_hash(&fixture);

    // No acceptances: the import is refused with its blocking losses, but the artifact must
    // already be retained and its hash returned.
    let (content_type, body) = multipart_import_body(&fixture);
    let response = router
        .oneshot(post_multipart(
            "/projects/coffee/import/stream",
            &content_type,
            body,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let value = json_body(response).await;
    assert_eq!(
        value["artifactHash"],
        expected_hash.as_str(),
        "the streaming endpoint must name the retained artifact hash"
    );

    let store = server::store::sqlite::SqliteStore::open(&dir.path().join("mw.db")).unwrap();
    assert_eq!(
        store.blob(&expected_hash).unwrap().as_deref(),
        Some(&fixture[..]),
        "the streamed artifact must be retained byte-for-byte"
    );
}

#[test]
fn the_streaming_store_retains_the_exact_bytes_under_the_same_hash() {
    let dir = tempfile::tempdir().unwrap();
    let store = server::store::sqlite::SqliteStore::open(&dir.path().join("mw.db")).unwrap();

    // A deliberately awkward payload: not UTF-8, with interior nuls, so a streaming write
    // that treated it as text or truncated at a terminator would be caught.
    let bytes: Vec<u8> = (0..=255u8).cycle().take(300_000).collect();
    let expected_hash = server::store::blob_hash(&bytes);

    let path = dir.path().join("artifact.bin");
    std::fs::write(&path, &bytes).unwrap();
    let stored_hash = store.put_blob_file(&path).unwrap();
    assert_eq!(stored_hash, expected_hash, "the hash must match the bytes");
    assert_eq!(
        store.blob(&stored_hash).unwrap().as_deref(),
        Some(&bytes[..])
    );
    assert!(
        !path.exists(),
        "the staged file must be removed after it is stored"
    );
}
