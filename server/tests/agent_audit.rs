// SPDX-License-Identifier: AGPL-3.0-or-later
//! The agent identity and what it may do. An agent is a client: it authenticates with an
//! agent token, goes through the same handlers and permission checks as a human, and holds
//! only the read roles (`viewer`, `reviewer`) a deployment grants it. An agent proposes and a
//! human commits, so an agent token can never hold a write or admin role, and every write
//! route refuses an agent with 403. The acceptance key also reconciles the agent's entry
//! identity with the acceptance the server stores, so accepting a proposal by ITS identity
//! lands on exactly the loss the human chose.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;

use binding::{Mapping, MappingVerdict};
use server::auth::AuthConfig;
use server::binding_api::{acceptance_matches, resolve_acceptances};
use server::store::{sqlite::SqliteStore, Store};

fn post(uri: &str, body: Value) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
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

fn bearer_get(uri: &str, token: &str) -> Request<Body> {
    Request::builder()
        .uri(uri)
        .header("authorization", format!("Bearer {}", token))
        .body(Body::empty())
        .unwrap()
}

async fn json_body(response: axum::response::Response) -> Value {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

/// The full application router under the given authentication, with a handle to the store so
/// a test can prove a denied request recorded nothing.
fn app_with_auth(auth: AuthConfig) -> (axum::Router, Arc<SqliteStore>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(SqliteStore::open(&dir.path().join("mw.db")).unwrap());
    let state = server::AppState {
        store: store.clone(),
        evidence_dir: dir.path().to_path_buf(),
        auth,
    };
    (server::app(state), store, dir)
}

/// A minimal but valid OKF document, the same shape the audit tests commit.
fn model() -> Value {
    json!({
        "project": "coffee",
        "exportedAt": "2026-09-17T00:00:00Z",
        "summary": {},
        "stateMachine": {"name": "sm", "regions": []},
        "requirements": [{
            "id": "r1", "name": "r1", "kind": "requirement",
            "stereotypes": ["Requirement"], "attributes": [], "documentation": "",
            "reqId": "1.1", "reqText": "text"
        }],
        "structure": [{
            "id": "b1", "name": "Block", "kind": "block",
            "stereotypes": ["Block"], "attributes": [], "documentation": ""
        }],
        "graph": {
            "nodes": [
                {"id": "b1", "kind": "block", "name": "Block"},
                {"id": "r1", "kind": "requirement", "name": "r1"}
            ],
            "edges": [{"source": "b1", "target": "r1", "kind": "dependency", "label": "Satisfy"}]
        }
    })
}

/// A hand-written SysML v1 XMI fixture from the binding's corpus, the same documents the
/// reader's own tests use.
fn fixture(name: &str) -> Vec<u8> {
    let path = format!(
        "{}/../engine/binding-xmi/fixtures/{}",
        env!("CARGO_MANIFEST_DIR"),
        name
    );
    std::fs::read(path).expect("fixture must exist")
}

async fn audit_entries(router: &axum::Router, token: &str) -> Vec<Value> {
    let response = router
        .clone()
        .oneshot(bearer_get("/projects/coffee/audit", token))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    json_body(response).await.as_array().unwrap().clone()
}

#[tokio::test]
async fn an_agent_authenticates_as_a_named_agent_and_can_read() {
    // An agent token authenticates a NAMED agent holding only a read/review role. Health
    // reports the mechanism ("agent", never the token), and a read through the agent's own
    // token succeeds.
    let (router, store, _dir) = app_with_auth(
        AuthConfig::agent_token(
            "agent-tok",
            "loss-report-resolver",
            "alex",
            &["reviewer"],
            &["*"],
        )
        .unwrap(),
    );

    let health = router
        .clone()
        .oneshot(bearer_get("/health", "agent-tok"))
        .await
        .unwrap();
    assert_eq!(health.status(), StatusCode::OK);
    assert_eq!(json_body(health).await["authMode"], "agent");

    store.create_project("coffee", None).unwrap();
    let read = router
        .clone()
        .oneshot(bearer_get("/projects", "agent-tok"))
        .await
        .unwrap();
    assert_eq!(read.status(), StatusCode::OK);
    let projects = json_body(read).await;
    assert_eq!(
        projects.as_array().unwrap().len(),
        1,
        "an agent may read the project its roles reach"
    );
}

#[tokio::test]
async fn an_agent_is_refused_every_model_change() {
    // A reviewer agent may read and review but holds no write role, so each of the four
    // routes that change a model - commit, merge, reset, import - is refused 403 before the
    // store is touched. The agent mechanism has no write path today: a human performs the
    // change through these ordinary routes.
    let (router, store, _dir) = app_with_auth(
        AuthConfig::agent_token(
            "agent-tok",
            "loss-report-resolver",
            "alex",
            &["reviewer"],
            &["*"],
        )
        .unwrap(),
    );
    store.create_project("coffee", None).unwrap();

    let writes: [(&str, Value); 4] = [
        (
            "/projects/coffee/commits",
            json!({ "branch": "main", "message": "m", "okf": model() }),
        ),
        (
            "/projects/coffee/merge",
            json!({ "branch": "main", "other": "feature", "message": "m" }),
        ),
        (
            "/projects/coffee/branches/main/reset",
            json!({ "to": "abc", "message": "m" }),
        ),
        (
            "/projects/coffee/import",
            json!({ "binding": "b@1", "branch": "main", "message": "m", "artifact": "x" }),
        ),
    ];

    for (uri, body) in writes {
        let response = router
            .clone()
            .oneshot(bearer_post(uri, body, "agent-tok"))
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::FORBIDDEN,
            "{} must be refused for an agent holding no write role",
            uri
        );
    }

    assert!(
        store.commits_on("coffee", "main").unwrap().is_empty(),
        "a refused write must write no commit"
    );
    assert!(
        audit_entries(&router, "agent-tok").await.is_empty(),
        "a refused write must record no audit entry"
    );
}

#[tokio::test]
async fn an_agent_is_refused_an_action_its_permissions_do_not_allow() {
    // A viewer agent may read but not administer; creating a project must be refused before
    // anything is written.
    let (router, store, _dir) = app_with_auth(
        AuthConfig::agent_token(
            "agent-tok",
            "loss-report-resolver",
            "alex",
            &["viewer"],
            &["*"],
        )
        .unwrap(),
    );

    let response = router
        .oneshot(bearer_post(
            "/projects",
            json!({ "name": "tea" }),
            "agent-tok",
        ))
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::FORBIDDEN,
        "a viewer agent must be refused project creation"
    );

    assert!(
        store.list_projects().unwrap().is_empty(),
        "a refused agent action must write nothing"
    );
    assert!(
        store.audit("tea", 1000).unwrap().is_empty(),
        "a refused agent action must record nothing"
    );
}

#[tokio::test]
async fn accepting_by_agent_identity_stores_the_acceptance_the_human_chose() {
    let (router, _store, _dir) = app_with_auth(AuthConfig::Open);
    // Seed the project (open mode accepts unauthenticated requests).
    let created = router
        .clone()
        .oneshot(post("/projects", json!({ "name": "coffee" })))
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::CREATED);

    // The six blocking losses of coffee-grinder.xmi, named by their ENTRY IDENTITIES - the
    // key a proposal carries, not the raw subject. If the server did not reconcile the two,
    // these would be refused as unknown losses and the import would not commit.
    let identities = [
        "uml:Model model-grinder [lossy]",
        "uml:Comment doc-grinder [lossy]",
        "uml:Property prop-motor [lossy]",
        "uml:Property prop-capacity [lossy]",
        "uml:Dependency dep-satisfy [lossy]",
        "uml:Package pkg-structure (Structure) [lossy]",
    ];
    let request_body = json!({
        "artifact": String::from_utf8(fixture("coffee-grinder.xmi")).unwrap(),
        "binding": "sysml-v1-xmi@2.4",
        "branch": "main",
        "message": "import coffee-grinder by identity",
        "acceptLosses": identities,
    });

    let response = router
        .clone()
        .oneshot(post("/projects/coffee/import", request_body))
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::CREATED,
        "accepting by entry identity must commit: {:?}",
        response
    );
    let body = json_body(response).await;

    // The acceptance the server STORED is exactly the identity the human chose, not the raw
    // subject the identity disambiguates.
    let stored: Vec<&str> = body["commit"]["provenance"]["acceptedLosses"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(stored, identities);
}

#[test]
fn an_entry_identity_accepts_exactly_the_entry_it_names() {
    // Two entries sharing the subject "uml:Comment c1" (a Lossy id-drop and an Unmappable
    // body-drop) differ only in verdict. Only the entry identity separates them, and the
    // per-entry predicate `acceptance_matches` accepts only that identity, never a raw
    // subject.
    let lossy = Mapping {
        subject: "uml:Comment c1".to_string(),
        verdict: MappingVerdict::Lossy,
        note: "uml:Comment xmi:id dropped".to_string(),
    };
    let unmappable = Mapping {
        subject: "uml:Comment c1".to_string(),
        verdict: MappingVerdict::Unmappable,
        note: "uml:Comment body dropped".to_string(),
    };

    assert!(
        acceptance_matches(&lossy, "uml:Comment c1 [lossy]"),
        "the Lossy identity must accept the Lossy entry"
    );
    assert!(
        !acceptance_matches(&unmappable, "uml:Comment c1 [lossy]"),
        "the Lossy identity must not accept the Unmappable entry"
    );
    assert!(
        acceptance_matches(&unmappable, "uml:Comment c1 [unmappable]"),
        "the Unmappable identity must accept the Unmappable entry"
    );
    assert!(
        !acceptance_matches(&lossy, "uml:Comment c1 [unmappable]"),
        "the Unmappable identity must not accept the Lossy entry"
    );

    // A raw subject is NOT an entry identity: `acceptance_matches` never reconciles it, so
    // the ambiguity of two entries sharing a subject cannot leak through this predicate.
    assert!(!acceptance_matches(&lossy, "uml:Comment c1"));
    assert!(!acceptance_matches(&unmappable, "uml:Comment c1"));
}

#[test]
fn two_entries_sharing_a_subject_require_the_entry_identity() {
    // Two blocking entries share the subject "uml:Comment c1". A raw subject names both, so
    // the request is refused (400) rather than silently accepting the one the human did not
    // choose. The entry identity is the key that keeps them two decisions.
    let lossy = Mapping {
        subject: "uml:Comment c1".to_string(),
        verdict: MappingVerdict::Lossy,
        note: "id dropped".to_string(),
    };
    let unmappable = Mapping {
        subject: "uml:Comment c1".to_string(),
        verdict: MappingVerdict::Unmappable,
        note: "body dropped".to_string(),
    };
    let blocking = [&lossy, &unmappable];

    let err = resolve_acceptances(&blocking, &["uml:Comment c1".to_string()])
        .expect_err("a raw subject shared by two entries must be refused");
    assert_eq!(err.status, StatusCode::BAD_REQUEST);
    assert!(
        err.message.contains("2 blocking entries"),
        "the refusal must name the ambiguity: {}",
        err.message
    );

    // Accepting ONE entry by its identity leaves the other unaccepted.
    let accepted = resolve_acceptances(&blocking, &["uml:Comment c1 [lossy]".to_string()])
        .expect("an entry identity is unambiguous");
    assert_eq!(accepted, vec!["uml:Comment c1 [lossy]".to_string()]);
}
