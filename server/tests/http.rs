// SPDX-License-Identifier: AGPL-3.0-or-later
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

fn state(dir: &std::path::Path) -> server::AppState {
    server::AppState {
        evidence_dir: dir.to_path_buf(),
    }
}

#[tokio::test]
async fn health_answers_ok() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    let response = router
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(value["status"], "ok");
}

#[tokio::test]
async fn version_names_the_service() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    let response = router
        .oneshot(
            Request::builder()
                .uri("/version")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(value["service"], "mw-server");
    assert_eq!(value["server"], env!("CARGO_PKG_VERSION"));
}
