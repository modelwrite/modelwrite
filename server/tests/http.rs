// SPDX-License-Identifier: AGPL-3.0-or-later
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

use server::auth::AuthConfig;

fn state_with_auth(dir: &std::path::Path, auth: AuthConfig) -> server::AppState {
    let store = server::store::sqlite::SqliteStore::open(&dir.join("mw.db")).unwrap();
    server::AppState {
        store: std::sync::Arc::new(store),
        evidence_dir: dir.to_path_buf(),
        auth,
    }
}

fn state(dir: &std::path::Path) -> server::AppState {
    state_with_auth(dir, AuthConfig::Open)
}

async fn health_body(router: axum::Router) -> serde_json::Value {
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
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn health_answers_ok_and_reports_the_open_mode() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    let value = health_body(router).await;
    assert_eq!(value["status"], "ok");
    assert_eq!(value["authMode"], "open");
}

#[tokio::test]
async fn health_reports_the_auth_mode_and_never_a_secret() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state_with_auth(
        dir.path(),
        AuthConfig::static_token("the-token"),
    ));
    let value = health_body(router).await;
    assert_eq!(value["authMode"], "static");
    assert!(
        !value.to_string().contains("the-token"),
        "a token must never appear in the health response"
    );

    // The mode is reported, not the keys: an empty key map is enough to prove the value.
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state_with_auth(
        dir.path(),
        AuthConfig::jwt(std::collections::HashMap::new(), None, None),
    ));
    let value = health_body(router).await;
    assert_eq!(value["authMode"], "jwt");
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
