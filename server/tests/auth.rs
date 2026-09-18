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
use serde_json::{json, Value};
use tower::ServiceExt;

use server::api::ApiState;
use server::auth::{AuthConfig, Identity};
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
            author(),
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
