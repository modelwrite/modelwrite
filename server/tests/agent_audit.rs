// SPDX-License-Identifier: AGPL-3.0-or-later
//! The agent identity and its audit trail. An agent is a client: it authenticates with an
//! agent token, goes through the same handlers, permissions, locks, audit entries and gate
//! as a human, and every action it takes is recorded with mechanism "agent", the agent's
//! subject, and the human or service that authorised it. A reader can tell an agent action
//! from a human one from the log alone, and an agent is refused whatever its permissions do
//! not allow. The acceptance key also reconciles the agent's entry identity with the
//! acceptance the server stores, so accepting a proposal by ITS identity lands on exactly the
//! loss the human chose.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;

use binding::{Mapping, MappingVerdict};
use server::auth::AuthConfig;
use server::binding_api::acceptance_matches;
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
async fn an_agent_action_is_audited_with_the_agent_mechanism_and_authorizer() {
    // The agent has admin scope only so it can create the project and commit: the point is
    // not the role, it is that the action is recorded AS an agent action.
    let (router, _store, _dir) = app_with_auth(AuthConfig::agent_token(
        "agent-tok",
        "loss-report-resolver",
        "alex",
        &["admin"],
        &["*"],
    ));

    let created = router
        .clone()
        .oneshot(bearer_post(
            "/projects",
            json!({ "name": "coffee" }),
            "agent-tok",
        ))
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::CREATED);

    let committed = router
        .clone()
        .oneshot(bearer_post(
            "/projects/coffee/commits",
            json!({ "branch": "main", "message": "base", "okf": model() }),
            "agent-tok",
        ))
        .await
        .unwrap();
    assert_eq!(committed.status(), StatusCode::CREATED);

    let entries = audit_entries(&router, "agent-tok").await;
    assert!(!entries.is_empty(), "an agent action must be recorded");
    for entry in &entries {
        assert_eq!(
            entry["mechanism"], "agent",
            "every agent action must be recorded with mechanism agent: {:?}",
            entry
        );
        assert_eq!(
            entry["actor"], "loss-report-resolver",
            "the audit must carry the agent's subject, not a claimed name"
        );
        assert_eq!(
            entry["authorizer"], "alex",
            "the audit must carry the human that authorised the agent"
        );
    }

    // Both the create and the commit are present, so the mechanism holds across actions.
    let actions: Vec<&str> = entries
        .iter()
        .map(|e| e["action"].as_str().unwrap())
        .collect();
    assert!(actions.contains(&"project.create"));
    assert!(actions.contains(&"commit.create"));
}

#[tokio::test]
async fn a_reader_can_tell_an_agent_action_from_a_human_one_from_the_log_alone() {
    // A human on a shared (static) token: mechanism "static", no authorizer.
    let (router, _store, _dir) = app_with_auth(AuthConfig::static_token("human-tok"));
    let created = router
        .clone()
        .oneshot(bearer_post(
            "/projects",
            json!({ "name": "coffee" }),
            "human-tok",
        ))
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::CREATED);
    let human = audit_entries(&router, "human-tok").await;
    let human_create = human
        .iter()
        .find(|e| e["action"] == "project.create")
        .expect("a project.create entry must exist");
    assert_eq!(human_create["mechanism"], "static");
    assert_eq!(human_create["actor"], "admin");
    assert_eq!(
        human_create["authorizer"], "",
        "a human action has no authorizer"
    );

    // An agent under a human: mechanism "agent", authorizer named.
    let (router, _store, _dir) = app_with_auth(AuthConfig::agent_token(
        "agent-tok",
        "loss-report-resolver",
        "alex",
        &["admin"],
        &["*"],
    ));
    let created = router
        .clone()
        .oneshot(bearer_post(
            "/projects",
            json!({ "name": "coffee" }),
            "agent-tok",
        ))
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::CREATED);
    let agent = audit_entries(&router, "agent-tok").await;
    let agent_create = agent
        .iter()
        .find(|e| e["action"] == "project.create")
        .expect("a project.create entry must exist");
    assert_eq!(agent_create["mechanism"], "agent");
    assert_eq!(agent_create["actor"], "loss-report-resolver");
    assert_eq!(agent_create["authorizer"], "alex");

    // The two records are distinguishable without any out-of-band knowledge.
    assert_ne!(human_create["mechanism"], agent_create["mechanism"]);
    assert_ne!(human_create["actor"], agent_create["actor"]);
    assert_ne!(human_create["authorizer"], agent_create["authorizer"]);
}

#[tokio::test]
async fn an_agent_is_refused_an_action_its_permissions_do_not_allow() {
    // A viewer agent may read but not administer; creating a project must be refused before
    // anything is written.
    let (router, store, _dir) = app_with_auth(AuthConfig::agent_token(
        "agent-tok",
        "loss-report-resolver",
        "alex",
        &["viewer"],
        &["*"],
    ));

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
    // A single XMI comment can drop its id as a Lossy entry and its body as an Unmappable
    // entry. Two entries share the subject "uml:Comment c1"; only the (subject, verdict)
    // identity separates them, and an acceptance must land on exactly one.
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

    // The legacy raw subject still reconciles to the entry, for the workbench form that
    // sends it; the identity is the unambiguous key an agent uses.
    assert!(acceptance_matches(&lossy, "uml:Comment c1"));
}
