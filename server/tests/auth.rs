// SPDX-License-Identifier: AGPL-3.0-or-later
//! End-to-end tests for the opt-in authentication layer: open mode admits every request,
//! static mode demands a matching bearer token, and no token is ever echoed in a response.
//!
//! Task 2 adds the denial half: with a `Fixed` identity the full route table must refuse an
//! under-privileged caller with 403 BEFORE touching the store, so a route added without a
//! permission decision cannot pass unnoticed.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::routing::get as axum_get;
use axum::{Json, Router};
use http_body_util::BodyExt;
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use serde_json::{json, Value};
use tower::ServiceExt;

use server::api::ApiState;
use server::auth::{parse_jwks, AuthConfig, Identity};
use server::store::{sqlite::SqliteStore, Store};
use server::{app, AppState};

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
    let store = SqliteStore::open(&dir.path().join("mw.db")).unwrap();
    Router::new()
        .route("/whoami", axum_get(whoami))
        .with_state(ApiState {
            store: Arc::new(store),
            evidence_dir: dir.path().to_path_buf(),
            auth,
            max_body_bytes: server::DEFAULT_MAX_BODY_BYTES,
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

// ---------------------------------------------------------------------------
// Task 2: the denial tests. A `Fixed` identity pins every request to a specific
// subject/role/scope, so the permission and project-scope checks on each route can be
// exercised before Task 4 wires a real per-claim authority.
// ---------------------------------------------------------------------------

fn identity(subject: &str, roles: &[&str], projects: &[&str]) -> Identity {
    Identity {
        subject: subject.to_string(),
        roles: roles.iter().map(|s| s.to_string()).collect(),
        projects: projects.iter().map(|s| s.to_string()).collect(),
    }
}

fn viewer() -> Identity {
    identity("viewer", &["viewer"], &["*"])
}

fn author() -> Identity {
    identity("author", &["author"], &["*"])
}

fn reviewer() -> Identity {
    identity("reviewer", &["reviewer"], &["*"])
}

/// No roles at all: cannot even Read, so it is the denial identity for a route whose only
/// requirement is Read and which has no project to scope (list projects).
fn nobody() -> Identity {
    identity("nobody", &[], &["*"])
}

/// An author scoped to one project: may write, but not reach any other project.
fn scoped_coffee() -> Identity {
    identity("scoped", &["author"], &["coffee"])
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

fn delete(uri: &str) -> Request<Body> {
    Request::builder()
        .method("DELETE")
        .uri(uri)
        .body(Body::empty())
        .unwrap()
}

fn delete_json(uri: &str, body: Value) -> Request<Body> {
    Request::builder()
        .method("DELETE")
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

/// Build the full application router pinned to one identity, returning a handle to the
/// store so a test can prove the denied request touched nothing. The tempdir is returned so
/// it stays alive for as long as the router (and its open SQLite connection) is used.
fn app_with_identity(identity: Identity) -> (Router, Arc<SqliteStore>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(SqliteStore::open(&dir.path().join("mw.db")).unwrap());
    let state = AppState {
        store: store.clone(),
        evidence_dir: dir.path().to_path_buf(),
        auth: AuthConfig::fixed(identity),
    };
    (app(state), store, dir)
}

/// Build the full application router with an ARBITRARY auth configuration (open, static or
/// jwt), returning a handle to the store and the tempdir. Used to prove the audit log records
/// HOW the caller authenticated, not just who they claimed to be.
fn app_with_auth(auth: AuthConfig) -> (Router, Arc<SqliteStore>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(SqliteStore::open(&dir.path().join("mw.db")).unwrap());
    let state = AppState {
        store: store.clone(),
        evidence_dir: dir.path().to_path_buf(),
        auth,
    };
    (app(state), store, dir)
}

#[tokio::test]
async fn static_mode_requires_a_token_on_a_real_route() {
    // A real route (not /whoami) must refuse a missing token with 401 BEFORE consulting the
    // store: with the token absent and no "coffee" project, reaching the store would 404.
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(SqliteStore::open(&dir.path().join("mw.db")).unwrap());
    let state = AppState {
        store: store.clone(),
        evidence_dir: dir.path().to_path_buf(),
        auth: AuthConfig::static_token("the-token"),
    };
    let router = app(state);
    let response = router
        .oneshot(get("/projects/coffee/commits"))
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "a missing token must be refused before the store is consulted (which would 404)"
    );
    assert!(
        store.list_projects().unwrap().is_empty(),
        "a refused request must touch nothing"
    );
}

#[tokio::test]
async fn viewer_is_refused_a_commit_without_touching_the_store() {
    let (router, store, _dir) = app_with_identity(viewer());
    // Seed the target so a denied commit is observable as "nothing changed".
    store.create_project("coffee", None).unwrap();

    let response = router
        .oneshot(post(
            "/projects/coffee/commits",
            json!({ "branch": "main", "author": "alex", "message": "m", "okf": {} }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    assert!(
        store.commits_on("coffee", "main").unwrap().is_empty(),
        "a refused commit must write no commit"
    );
    assert!(
        store.list_branches("coffee").unwrap().is_empty(),
        "a refused commit must not move a branch"
    );
    assert!(
        store.audit("coffee", 1000).unwrap().is_empty(),
        "a refused commit must write no audit entry"
    );
}

#[tokio::test]
async fn author_is_refused_project_creation() {
    let (router, store, _dir) = app_with_identity(author());
    let response = router
        .oneshot(post("/projects", json!({ "name": "tea" })))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert!(
        store.list_projects().unwrap().is_empty(),
        "a refused project creation must create nothing"
    );
}

#[tokio::test]
async fn a_token_scoped_to_one_project_is_refused_on_another() {
    let (router, store, _dir) = app_with_identity(scoped_coffee());
    // The scoped author may WRITE, so the only thing standing between it and a commit on
    // "tea" is project scope: may_reach must refuse it before any store access.
    let response = router
        .oneshot(post(
            "/projects/tea/commits",
            json!({ "branch": "main", "author": "alex", "message": "m", "okf": {} }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert!(
        store.list_projects().unwrap().is_empty(),
        "an out-of-scope request must touch nothing"
    );
}

/// The complete route table, one entry per (method, path) pair in lib.rs except the public
/// /health and /version liveness endpoints. Each entry names an identity that must be
/// refused with 403: a viewer or author for routes whose permission it lacks, a no-role
/// identity for list-projects (Read with no project to scope), and a coffee-scoped author
/// on the "tea" project for every Read route, which only project scope can deny. A new
/// route that does not appear here, or that is not refused, is a permission bug.
#[tokio::test]
async fn every_route_denies_a_caller_without_permission_or_scope() {
    let commit = || json!({ "branch": "main", "author": "alex", "message": "m", "okf": {} });
    let branch = || json!({ "name": "review", "from": "abc" });
    let reset = || json!({ "to": "abc", "author": "alex", "message": "m" });
    let merge =
        || json!({ "branch": "main", "other": "feature", "author": "alex", "message": "m" });
    let gate = || json!({ "reference": "a", "candidate": "b" });
    let locks =
        || json!({ "branch": "main", "elements": ["b1"], "holder": "alex", "ttlSeconds": 300 });
    let release = || json!({ "holder": "alex", "ids": ["x"] });

    let cases: Vec<(&str, Identity, Request<Body>)> = vec![
        (
            "create_project",
            author(),
            post("/projects", json!({ "name": "tea" })),
        ),
        ("list_projects", nobody(), get("/projects")),
        (
            "create_commit",
            viewer(),
            post("/projects/coffee/commits", commit()),
        ),
        (
            "list_commits",
            scoped_coffee(),
            get("/projects/tea/commits"),
        ),
        (
            "get_commit",
            scoped_coffee(),
            get("/projects/tea/commits/abc"),
        ),
        (
            "get_commit_record",
            scoped_coffee(),
            get("/projects/tea/commits/abc/record"),
        ),
        (
            "get_commit_checks",
            viewer(),
            get("/projects/coffee/commits/abc/checks"),
        ),
        (
            "get_artifact",
            scoped_coffee(),
            get("/projects/tea/import/abc/artifact"),
        ),
        (
            "create_branch",
            viewer(),
            post("/projects/coffee/branches", branch()),
        ),
        (
            "list_branches",
            scoped_coffee(),
            get("/projects/tea/branches"),
        ),
        (
            "delete_branch",
            author(),
            delete("/projects/coffee/branches/main"),
        ),
        (
            "reset_branch",
            viewer(),
            post("/projects/coffee/branches/main/reset", reset()),
        ),
        ("run_gate", viewer(), post("/projects/coffee/gate", gate())),
        (
            "merge_branches",
            viewer(),
            post("/projects/coffee/merge", merge()),
        ),
        (
            "list_gate_runs",
            viewer(),
            get("/projects/coffee/gate-runs"),
        ),
        ("list_audit", scoped_coffee(), get("/projects/tea/audit")),
        (
            "acquire_locks",
            viewer(),
            post("/projects/coffee/locks", locks()),
        ),
        ("list_locks", scoped_coffee(), get("/projects/tea/locks")),
        (
            "release_locks_delete",
            viewer(),
            delete_json("/projects/coffee/locks", release()),
        ),
        (
            "release_locks_post",
            viewer(),
            post("/projects/coffee/locks/release", release()),
        ),
    ];

    for (name, identity, request) in cases {
        let (router, store, _dir) = app_with_identity(identity);
        let response = router.oneshot(request).await.unwrap();
        assert_eq!(
            response.status(),
            StatusCode::FORBIDDEN,
            "{} must be refused with 403",
            name
        );
        assert!(
            store.list_projects().unwrap().is_empty(),
            "{} must be refused before the store is touched",
            name
        );
    }
}
#[tokio::test]
async fn a_scoped_identity_only_sees_the_projects_it_may_reach() {
    // A refusal is not the only way to leak: a listing that returns every project name
    // tells a caller scoped to one project about all the others. The listing is filtered,
    // so the answer to "what may I see" is the truth rather than an error.
    let (router, store, _dir) = app_with_identity(Identity {
        subject: "alex".to_string(),
        roles: vec!["viewer".to_string()],
        projects: vec!["coffee".to_string()],
    });
    store.create_project("coffee", None).unwrap();
    store.create_project("tea", None).unwrap();

    let response = router
        .oneshot(
            Request::builder()
                .uri("/projects")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    let names: Vec<String> = body
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["name"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(names, vec!["coffee"], "tea must not be disclosed");
}
#[tokio::test]
async fn liveness_and_version_are_public_and_reach_nothing() {
    // These two routes are deliberately outside the permission model, so they must be
    // reachable with no token even when authentication is configured - a probe that needs
    // a credential fails exactly when it is needed. They take no state at all, so they
    // cannot reach the store and cannot disclose anything from it.
    let (router, _store, _dir) = app_with_identity(Identity::open());
    for uri in ["/health", "/version"] {
        let response = router
            .clone()
            .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{} must be public", uri);
    }
}

#[tokio::test]
async fn a_identity_with_no_roles_is_denied_a_read() {
    // Scope and permission are separate decisions: an identity that reaches the project
    // must STILL be refused if it holds no role that grants reading.
    let (router, store, _dir) = app_with_identity(Identity {
        subject: "nobody".to_string(),
        roles: Vec::new(),
        projects: vec!["*".to_string()],
    });
    store.create_project("coffee", None).unwrap();

    let response = router
        .oneshot(
            Request::builder()
                .uri("/projects/coffee/commits?branch=main")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::FORBIDDEN,
        "holding no role must be refused even with full scope"
    );
}

#[tokio::test]
async fn a_identity_that_reaches_nothing_sees_an_empty_listing() {
    let (router, store, _dir) = app_with_identity(Identity {
        subject: "scopeless".to_string(),
        roles: vec!["viewer".to_string()],
        projects: Vec::new(),
    });
    store.create_project("coffee", None).unwrap();

    let response = router
        .oneshot(
            Request::builder()
                .uri("/projects")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        json_body(response).await.as_array().unwrap().is_empty(),
        "an identity scoped to nothing sees nothing"
    );
}

#[tokio::test]
async fn an_author_and_a_reviewer_can_list_gate_runs() {
    // An author may RUN a gate (Write), so it must be able to SEE the result too; a
    // reviewer reads runs it did not start. A viewer holds neither role and stays refused.
    for ident in [author(), reviewer()] {
        let (router, _store, _dir) = app_with_identity(ident);
        let response = router
            .oneshot(get("/projects/coffee/gate-runs"))
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "Write OR Review must list gate runs"
        );
    }

    let (router, _store, _dir) = app_with_identity(viewer());
    let response = router
        .oneshot(get("/projects/coffee/gate-runs"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}
// ---------------------------------------------------------------------------
// Task 4: signed tokens (JWT) against a JWKS. A committed RSA-2048 key pair
// (generated once, committed so tests need no network and no identity provider)
// signs tokens; the JWKS carries only the public half. A second private key
// proves that a token signed by the wrong key is refused.
// ---------------------------------------------------------------------------

const JWKS: &str = include_str!("fixtures/jwks.json");
const PRIVATE_KEY: &str = include_str!("fixtures/rs256_private.pem");
const OTHER_PRIVATE_KEY: &str = include_str!("fixtures/rs256_other_private.pem");

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

/// Sign an RS256 token with the given claims, kid and private key PEM.
fn sign(claims: Value, kid: &str, pem: &str) -> String {
    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some(kid.to_string());
    encode(
        &header,
        &claims,
        &EncodingKey::from_rsa_pem(pem.as_bytes()).unwrap(),
    )
    .unwrap()
}

fn jwt_config(issuer: Option<&str>, audience: Option<&str>) -> AuthConfig {
    AuthConfig::jwt(
        parse_jwks(JWKS).unwrap(),
        issuer.map(str::to_string),
        audience.map(str::to_string),
    )
}

fn bearer(uri: &str, token: &str) -> Request<Body> {
    Request::builder()
        .uri(uri)
        .header("authorization", format!("Bearer {}", token))
        .body(Body::empty())
        .unwrap()
}

fn bearer_post(uri: &str, body: Value, token: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {}", token))
        .body(Body::from(body.to_string()))
        .unwrap()
}

/// base64url without padding, for the hand-crafted alg:none token (jsonwebtoken cannot
/// emit an alg:none header, by design).
fn b64url(input: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::new();
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(ALPHABET[((n >> 18) & 63) as usize] as char);
        out.push(ALPHABET[((n >> 12) & 63) as usize] as char);
        if chunk.len() > 1 {
            out.push(ALPHABET[((n >> 6) & 63) as usize] as char);
        }
        if chunk.len() > 2 {
            out.push(ALPHABET[(n & 63) as usize] as char);
        }
    }
    out
}

#[tokio::test]
async fn jwt_mode_accepts_a_valid_token() {
    let router = router(jwt_config(
        Some("https://idp.example.com"),
        Some("modelwrite"),
    ));
    let token = sign(
        json!({
            "sub": "alex",
            "roles": ["author", "reviewer"],
            "projects": ["coffee"],
            "iss": "https://idp.example.com",
            "aud": "modelwrite",
            "nbf": now() - 3600,
            "exp": now() + 3600,
        }),
        "test-key",
        PRIVATE_KEY,
    );
    let response = router.oneshot(bearer("/whoami", &token)).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    assert_eq!(body["subject"], "alex");
    assert_eq!(body["roles"][0], "author");
    assert_eq!(body["roles"][1], "reviewer");
    assert_eq!(body["projects"][0], "coffee");
}

#[tokio::test]
async fn jwt_mode_rejects_a_token_with_no_sub() {
    // A signed token with no subject must be refused, not accepted as an identity whose
    // subject is empty: such an identity reads fine and then breaks every write at the audit
    // table's non-empty-actor constraint.
    let router = router(jwt_config(None, None));
    let token = sign(
        json!({ "roles": ["admin"], "nbf": now() - 3600, "exp": now() + 3600 }),
        "test-key",
        PRIVATE_KEY,
    );
    let response = router.oneshot(bearer("/whoami", &token)).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn jwt_mode_rejects_a_token_with_an_empty_sub() {
    let router = router(jwt_config(None, None));
    let token = sign(
        json!({ "sub": "", "roles": ["admin"], "nbf": now() - 3600, "exp": now() + 3600 }),
        "test-key",
        PRIVATE_KEY,
    );
    let response = router.oneshot(bearer("/whoami", &token)).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn jwt_mode_rejects_a_wrong_typed_sub() {
    // A `sub` of the wrong JSON type must be refused rather than treated as absent.
    let router = router(jwt_config(None, None));
    let token = sign(
        json!({ "sub": 123, "roles": ["admin"], "nbf": now() - 3600, "exp": now() + 3600 }),
        "test-key",
        PRIVATE_KEY,
    );
    let response = router.oneshot(bearer("/whoami", &token)).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn jwt_mode_rejects_an_expired_token_without_echoing_it() {
    let router = router(jwt_config(None, None));
    let token = sign(
        json!({ "sub": "alex", "roles": ["author"], "nbf": now() - 3600, "exp": now() - 120 }),
        "test-key",
        PRIVATE_KEY,
    );
    let response = router.oneshot(bearer("/whoami", &token)).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let text = body_text(response).await;
    assert!(text.contains("error"), "a 401 must be a structured error");
    assert!(
        !text.contains(&token),
        "a token must never appear in a response body"
    );
}

#[tokio::test]
async fn jwt_mode_rejects_a_token_signed_by_the_wrong_key() {
    let router = router(jwt_config(None, None));
    // Signed with the OTHER private key but claiming the configured kid, so the right key
    // is selected and the signature check fails.
    let token = sign(
        json!({ "sub": "alex", "roles": ["author"], "nbf": now() - 3600, "exp": now() + 3600 }),
        "test-key",
        OTHER_PRIVATE_KEY,
    );
    let response = router.oneshot(bearer("/whoami", &token)).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn jwt_mode_rejects_a_wrong_issuer() {
    let router = router(jwt_config(Some("https://idp.example.com"), None));
    let token = sign(
        json!({
            "sub": "alex",
            "roles": ["author"],
            "iss": "https://evil.example.com",
            "nbf": now() - 3600,
            "exp": now() + 3600,
        }),
        "test-key",
        PRIVATE_KEY,
    );
    let response = router.oneshot(bearer("/whoami", &token)).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn jwt_mode_rejects_a_missing_issuer() {
    let router = router(jwt_config(Some("https://idp.example.com"), None));
    let token = sign(
        json!({ "sub": "alex", "roles": ["author"], "nbf": now() - 3600, "exp": now() + 3600 }),
        "test-key",
        PRIVATE_KEY,
    );
    let response = router.oneshot(bearer("/whoami", &token)).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn jwt_mode_rejects_a_wrong_audience() {
    let router = router(jwt_config(None, Some("modelwrite")));
    let token = sign(
        json!({
            "sub": "alex",
            "roles": ["author"],
            "aud": "some-other-service",
            "nbf": now() - 3600,
            "exp": now() + 3600,
        }),
        "test-key",
        PRIVATE_KEY,
    );
    let response = router.oneshot(bearer("/whoami", &token)).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn jwt_mode_rejects_alg_none() {
    let router = router(jwt_config(None, None));
    let header = b64url(br#"{"alg":"none","kid":"test-key"}"#);
    let claims = b64url(br#"{"sub":"alex","roles":["admin"],"exp":9999999999}"#);
    let token = format!("{}.{}.", header, claims);
    let response = router.oneshot(bearer("/whoami", &token)).await.unwrap();
    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "alg:none must be refused outright"
    );
}

#[tokio::test]
async fn jwt_mode_rejects_a_malformed_expiry_type() {
    // A malformed exp (a string rather than a number) must be refused, not silently
    // treated as absent: otherwise an attacker could shed the expiry entirely.
    let router = router(jwt_config(None, None));
    let token = sign(
        json!({ "sub": "alex", "roles": ["author"], "nbf": now() - 3600, "exp": "not-a-number" }),
        "test-key",
        PRIVATE_KEY,
    );
    let response = router.oneshot(bearer("/whoami", &token)).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn jwt_mode_rejects_a_token_whose_nbf_is_in_the_future() {
    // A token whose not-before is still ahead must be refused, not accepted early.
    let router = router(jwt_config(None, None));
    let token = sign(
        json!({
            "sub": "alex",
            "roles": ["author"],
            "exp": now() + 7200,
            "nbf": now() + 3600,
        }),
        "test-key",
        PRIVATE_KEY,
    );
    let response = router.oneshot(bearer("/whoami", &token)).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn jwt_mode_rejects_a_malformed_not_before_type() {
    // A malformed nbf (a string rather than a number) must be refused, not silently
    // treated as absent: otherwise a not-before could be shed entirely.
    let router = router(jwt_config(None, None));
    let token = sign(
        json!({
            "sub": "alex",
            "roles": ["author"],
            "exp": now() + 3600,
            "nbf": "not-a-number",
        }),
        "test-key",
        PRIVATE_KEY,
    );
    let response = router.oneshot(bearer("/whoami", &token)).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn jwt_mode_tolerates_clock_skew_within_the_window_but_not_beyond() {
    // 30 seconds of past expiry is inside the 60-second window, so a user is not locked
    // out by an identity provider whose clock runs slightly ahead.
    let router = router(jwt_config(None, None));
    let within = sign(
        json!({ "sub": "alex", "roles": ["viewer"], "nbf": now() - 3600, "exp": now() - 30 }),
        "test-key",
        PRIVATE_KEY,
    );
    let response = router
        .clone()
        .oneshot(bearer("/whoami", &within))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // 120 seconds of past expiry is beyond the window and must be refused.
    let beyond = sign(
        json!({ "sub": "alex", "roles": ["viewer"], "nbf": now() - 3600, "exp": now() - 120 }),
        "test-key",
        PRIVATE_KEY,
    );
    let response = router.oneshot(bearer("/whoami", &beyond)).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn jwt_mode_without_a_roles_claim_cannot_write() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(SqliteStore::open(&dir.path().join("mw.db")).unwrap());
    store.create_project("coffee", None).unwrap();
    let state = AppState {
        store: store.clone(),
        evidence_dir: dir.path().to_path_buf(),
        auth: jwt_config(None, None),
    };
    let router = app(state);
    // A valid token with no roles claim: it verifies, but holds no role, so a write is
    // refused with 403 rather than defaulting to a permissive identity.
    let token = sign(
        json!({ "sub": "alex", "nbf": now() - 3600, "exp": now() + 3600 }),
        "test-key",
        PRIVATE_KEY,
    );
    let response = router
        .oneshot(bearer_post(
            "/projects/coffee/commits",
            json!({ "branch": "main", "author": "alex", "message": "m", "okf": {} }),
            &token,
        ))
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::FORBIDDEN,
        "a no-role identity must be refused a write, not treated as permissive"
    );
}

#[tokio::test]
async fn from_jwks_file_loads_a_configuration() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("jwks.json");
    std::fs::write(&path, JWKS).unwrap();
    let config = AuthConfig::from_jwks_file(path.to_str().unwrap()).unwrap();
    match &config {
        AuthConfig::Jwt { keys, .. } => {
            assert!(keys.contains_key("test-key"), "the committed kid must load");
        }
        other => panic!("expected a Jwt config, got {:?}", other),
    }
}
async fn audit_entries(router: &axum::Router, token: Option<&str>) -> Vec<Value> {
    let response = match token {
        Some(t) => router.clone().oneshot(bearer("/projects/coffee/audit", t)),
        None => router.clone().oneshot(get("/projects/coffee/audit")),
    }
    .await
    .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    json_body(response).await.as_array().unwrap().clone()
}

#[tokio::test]
async fn audit_entries_record_how_the_caller_authenticated() {
    // OPEN: nobody was authenticated; the honest subject is "anonymous" and the mechanism
    // says so.
    let (router, _store, _dir) = app_with_auth(AuthConfig::Open);
    let created = router
        .clone()
        .oneshot(post("/projects", json!({ "name": "coffee" })))
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::CREATED);
    for entry in audit_entries(&router, None).await {
        assert_eq!(entry["mechanism"], "open");
        assert_eq!(
            entry["actor"], "anonymous",
            "the mechanism must not change the subject"
        );
    }

    // STATIC: a shared token maps to the fixed subject "admin", but the log must say it was a
    // shared token rather than imply a named individual.
    let (router, _store, _dir) = app_with_auth(AuthConfig::static_token("the-token"));
    let created = router
        .clone()
        .oneshot(bearer_post(
            "/projects",
            json!({ "name": "coffee" }),
            "the-token",
        ))
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::CREATED);
    for entry in audit_entries(&router, Some("the-token")).await {
        assert_eq!(entry["mechanism"], "static");
        assert_eq!(entry["actor"], "admin");
    }

    // JWT: the subject arrives from the verified sub claim, and the mechanism records that a
    // signed token was verified.
    let (router, _store, _dir) = app_with_auth(jwt_config(None, None));
    let token = sign(
        json!({
            "sub": "alex",
            "roles": ["admin"],
            "projects": ["*"],
            "nbf": now() - 3600,
            "exp": now() + 3600,
        }),
        "test-key",
        PRIVATE_KEY,
    );
    let created = router
        .clone()
        .oneshot(bearer_post(
            "/projects",
            json!({ "name": "coffee" }),
            &token,
        ))
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::CREATED);
    for entry in audit_entries(&router, Some(&token)).await {
        assert_eq!(entry["mechanism"], "jwt");
        assert_eq!(entry["actor"], "alex");
    }
}
