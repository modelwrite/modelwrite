// SPDX-License-Identifier: AGPL-3.0-or-later
//! The registered tier, proven end to end: the cross-trial refusal (the security core) and the
//! no-JS registration flow landing in an empty workspace.

use std::path::Path;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

use server::store::{self, sqlite::SqliteStore, Store};
use server::trial::{self, TrialService};

fn setup(dir: &Path) -> Arc<TrialService> {
    let identity = trial::IdentityStore::open(&dir.join("identity.db")).unwrap();
    let registry = store::TrialRegistry::new(dir.to_path_buf());
    TrialService::new(
        identity,
        registry,
        Arc::new(trial::ConsoleTransport),
        trial::TrialConfig::default(),
        dir.join("archive"),
    )
}

fn register(tier: &TrialService, email: &str, ip: &str) -> trial::Session {
    let now = store::now_seconds();
    let code = tier.request_code(email, ip, false, now).unwrap();
    tier.redeem(email, &code, now).unwrap()
}

fn router(tier: Arc<TrialService>, dir: &Path) -> axum::Router {
    let placeholder = SqliteStore::open(Path::new(":memory:")).unwrap();
    server::registered::app_registered(
        tier,
        Arc::new(placeholder) as Arc<dyn Store>,
        dir.join("evidence"),
        server::max_body_bytes_from_env(),
    )
}

async fn body_of(response: axum::response::Response) -> serde_json::Value {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
}

async fn authorized_get(
    router: &axum::Router,
    uri: &str,
    token: &str,
) -> (StatusCode, serde_json::Value) {
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .uri(uri)
                .header(header::AUTHORIZATION, format!("Bearer {}", token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let body = body_of(response).await;
    (status, body)
}

async fn authorized_post_json(
    router: &axum::Router,
    uri: &str,
    token: &str,
    body: serde_json::Value,
) -> (StatusCode, serde_json::Value) {
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header(header::AUTHORIZATION, format!("Bearer {}", token))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let body = body_of(response).await;
    (status, body)
}

/// THE PROOF: user A's session cannot read user B's project even holding B's project id in
/// hand. The trial identity comes from the SESSION, never from the request; the project id in
/// the URL is looked up in A's own trial database, which does not contain B's project.
#[tokio::test]
async fn user_as_session_cannot_read_user_bs_project_even_holding_the_id() {
    let dir = tempfile::tempdir().unwrap();
    let tier = setup(dir.path());
    let session_a = register(&tier, "a@example.com", "1.2.3.4");
    let session_b = register(&tier, "b@example.com", "5.6.7.8");

    let router = router(tier.clone(), dir.path());

    // B creates a project called "secret". A does not know it exists.
    let (status, _) = authorized_post_json(
        &router,
        "/projects",
        &session_b.token,
        serde_json::json!({ "name": "secret" }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    // B can list its own branches for "secret".
    let (status, _) = authorized_get(&router, "/projects/secret/branches", &session_b.token).await;
    assert_eq!(status, StatusCode::OK);

    // A holds B's project id in hand and asks for it: the store A's session resolves to has no
    // such project, so the answer is NOT FOUND, not the project.
    let (status, _) = authorized_get(&router, "/projects/secret/branches", &session_a.token).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // And A's project list is empty: it never learned B's project name.
    let (status, body) = authorized_get(&router, "/projects", &session_a.token).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, serde_json::json!([]));
}

/// The registration flow, server-rendered and no-JS: request a code, enter it, and land in an
/// empty workspace. The code is transactional (it arrives regardless of the consent choice),
/// and the session is a cookie the workbench reads.
#[tokio::test]
async fn the_registration_flow_lands_in_an_empty_workspace() {
    let dir = tempfile::tempdir().unwrap();
    let tier = setup(dir.path());
    let router = router(tier.clone(), dir.path());
    let now = store::now_seconds();

    // Request a code (unchecked consent: the code still arrives).
    let code = tier
        .request_code("a@example.com", "1.2.3.4", false, now)
        .unwrap();
    assert_eq!(code.len(), 6);

    // Enter the code: the login form responds with the session cookie.
    let form = format!("email={}&code={}", "a@example.com", code);
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/login")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(form))
                .unwrap(),
        )
        .await
        .unwrap();
    // Redirect::to is a 303 See Other; the session cookie rides that redirect.
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let cookie = response
        .headers()
        .get(header::SET_COOKIE)
        .and_then(|v| v.to_str().ok())
        .expect("login sets a session cookie");
    let token = cookie
        .trim_start_matches("mw_session=")
        .split(';')
        .next()
        .unwrap()
        .to_string();

    // The workbench, with the cookie, is an empty workspace.
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/projects")
                .header(header::COOKIE, format!("mw_session={}", token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(body_of(response).await, serde_json::json!([]));
}
