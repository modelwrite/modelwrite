// SPDX-License-Identifier: AGPL-3.0-or-later
//! End-to-end tests for the opt-in authentication layer: open mode admits every request,
//! static mode demands a matching bearer token, and no token is ever echoed in a response.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::routing::get;
use axum::{Json, Router};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;

use server::api::ApiState;
use server::auth::{AuthConfig, Identity};

/// A route that exercises the Identity extractor and returns what it resolved. The rest of
/// the service threads Identity through its real routes in Task 2; this is the minimal
/// surface the extraction layer needs to be proven against.
async fn whoami(identity: Identity) -> Json<Value> {
    Json(json!({
        "subject": identity.subject,
        "roles": identity.roles,
        "projects": identity.projects,
    }))
}

fn router(auth: AuthConfig) -> Router {
    let dir = tempfile::tempdir().unwrap();
    let store = server::store::sqlite::SqliteStore::open(&dir.path().join("mw.db")).unwrap();
    Router::new()
        .route("/whoami", get(whoami))
        .with_state(ApiState {
            store: Arc::new(store),
            evidence_dir: dir.path().to_path_buf(),
            auth,
        })
}

async fn json_body(response: axum::response::Response) -> Value {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

async fn body_text(response: axum::response::Response) -> String {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8(bytes.to_vec()).unwrap()
}

#[tokio::test]
async fn open_mode_accepts_an_unauthenticated_request() {
    let router = router(AuthConfig::Open);
    let response = router
        .oneshot(
            Request::builder()
                .uri("/whoami")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    assert_eq!(body["subject"], "anonymous");
    assert_eq!(body["roles"][0], "admin");
    assert_eq!(body["projects"][0], "*");
}

#[tokio::test]
async fn static_mode_rejects_a_missing_token() {
    let router = router(AuthConfig::static_token("the-token"));
    let response = router
        .oneshot(
            Request::builder()
                .uri("/whoami")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let text = body_text(response).await;
    assert!(text.contains("error"), "a 401 must be a structured error");
}

#[tokio::test]
async fn static_mode_rejects_a_wrong_token_without_echoing_it() {
    let router = router(AuthConfig::static_token("the-token"));
    let response = router
        .oneshot(
            Request::builder()
                .uri("/whoami")
                .header("authorization", "Bearer wrong-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let text = body_text(response).await;
    assert!(text.contains("error"), "a 401 must be a structured error");
    assert!(
        !text.contains("wrong-token"),
        "a token must never appear in a response body"
    );
}

#[tokio::test]
async fn static_mode_rejects_a_malformed_authorization_header() {
    let router = router(AuthConfig::static_token("the-token"));
    let response = router
        .oneshot(
            Request::builder()
                .uri("/whoami")
                .header("authorization", "NotABearer the-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn static_mode_accepts_the_right_token() {
    let router = router(AuthConfig::static_token("the-token"));
    let response = router
        .oneshot(
            Request::builder()
                .uri("/whoami")
                .header("authorization", "Bearer the-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    assert_eq!(body["subject"], "admin");
    assert_eq!(body["roles"][0], "admin");
    assert_eq!(body["projects"][0], "*");
}
