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
#[test]
fn the_bind_rule_refuses_to_expose_an_unauthenticated_service() {
    // Open mode plus an interface anyone can route to means every request is an anonymous
    // admin. The combination is refused unless the operator says so explicitly, and the
    // safe default stays loopback.
    use server::resolve_bind;

    // Loopback is always allowed, authenticated or not.
    assert!(resolve_bind("127.0.0.1", true, false).is_ok());
    assert!(resolve_bind("127.0.0.1", false, false).is_ok());

    // Beyond loopback WITH authentication is fine: the container case.
    assert!(resolve_bind("0.0.0.0", false, false).is_ok());

    // Beyond loopback WITHOUT authentication is refused...
    let refused = resolve_bind("0.0.0.0", true, false);
    assert!(
        refused.is_err(),
        "exposing an unauthenticated service must be refused"
    );
    let message = refused.unwrap_err().to_string();
    assert!(message.contains("anonymous admin"), "got: {}", message);

    // ...unless it is deliberate.
    assert!(resolve_bind("0.0.0.0", true, true).is_ok());

    // And a value that is not an address is a clear error, not a panic.
    assert!(resolve_bind("not-an-address", false, false).is_err());
}
