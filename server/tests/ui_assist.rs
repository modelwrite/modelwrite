// SPDX-License-Identifier: AGPL-3.0-or-later
//! The assist panel and proposals page, proven end to end through the server-rendered
//! workbench: the panel renders the review artifact WITHOUT committing, a human with write
//! accepts a proposal and the page names both parties, a caller without write never sees an
//! Accept button, a refused request shows its reason, and the proposals page lists decisions.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;

use okf::types::OkfRoot;
use server::auth::{AuthConfig, Identity};
use server::store::{sqlite::SqliteStore, ProposalDecision, Store};
use server::AppState;

fn app_with_auth(auth: AuthConfig) -> (axum::Router, Arc<SqliteStore>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(SqliteStore::open(&dir.path().join("mw.db")).unwrap());
    let state = AppState {
        store: store.clone(),
        evidence_dir: dir.path().to_path_buf(),
        auth,
    };
    (server::app(state), store, dir)
}

/// A human admin: read and write.
fn admin() -> AuthConfig {
    AuthConfig::fixed(Identity {
        subject: "alex".to_string(),
        roles: vec!["admin".to_string()],
        projects: vec!["*".to_string()],
        trial_id: None,
    })
}

/// A read-only viewer: read, but no write.
fn viewer() -> AuthConfig {
    AuthConfig::fixed(Identity {
        subject: "viewer".to_string(),
        roles: vec!["viewer".to_string()],
        projects: vec!["*".to_string()],
        trial_id: None,
    })
}

/// Force the deterministic scripted reasoner, so no test reaches the network.
fn use_scripted_reasoner() {
    std::env::set_var("MW_ASSIST_REASONER", "scripted");
}

fn get(uri: &str) -> Request<Body> {
    Request::builder().uri(uri).body(Body::empty()).unwrap()
}

fn post_form(uri: &str, params: &[(&str, &str)]) -> Request<Body> {
    let body = params
        .iter()
        .map(|(key, value)| format!("{}={}", key, percent_encode(value)))
        .collect::<Vec<_>>()
        .join("&");
    Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from(body))
        .unwrap()
}

fn percent_encode(value: &str) -> String {
    let mut out = String::new();
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{:02X}", byte)),
        }
    }
    out
}

async fn body_text(response: axum::response::Response) -> String {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8(bytes.to_vec()).unwrap()
}

/// A minimal, valid OKF model the scripted reasoner can build on.
fn minimal_model() -> Value {
    json!({
        "okf": "1.0",
        "project": "coffee",
        "summary": {
            "blocks": 0, "requirements": 0, "interfaces": 0, "signals": 0,
            "activities": 0, "graphNodes": 1, "graphEdges": 0
        },
        "stateMachine": { "name": "sm", "regions": [] },
        "graph": {
            "nodes": [{ "id": "root", "kind": "block", "name": "root", "stereotypes": [] }],
            "edges": []
        }
    })
}

/// Seed a project and a model directly through the store, bypassing the JSON commit handler's
/// author resolution so a test can fix a specific identity afterwards.
fn seed_model_directly(store: &dyn Store, okf: Value) {
    store.create_project("coffee", None).unwrap();
    let root: OkfRoot = serde_json::from_value(okf).unwrap();
    let bytes = serde_json::to_vec(&root).unwrap();
    let okf_hash = store.put_blob(&bytes).unwrap();
    store
        .commit_model(
            "coffee", "main", &okf_hash, "seeder", "seed", None, None, None,
        )
        .unwrap();
}

#[tokio::test]
async fn the_assist_panel_renders_the_artifact_and_writes_no_commit() {
    use_scripted_reasoner();
    let (router, store, _dir) = app_with_auth(admin());
    seed_model_directly(store.as_ref(), minimal_model());

    // The model page shows the panel with a request box and a submit, in scripted mode.
    let model = router
        .clone()
        .oneshot(get("/ui/projects/coffee/model"))
        .await
        .unwrap();
    assert_eq!(model.status(), StatusCode::OK);
    let html = body_text(model).await;
    assert!(
        html.contains(r#"name="request""#),
        "the panel must render the request box, got:
{}",
        html
    );
    assert!(
        html.contains("scripted"),
        "the panel must name the scripted mode, got:
{}",
        html
    );
    assert!(
        !html.contains("No live reasoner configured"),
        "scripted mode must not claim no reasoner is configured"
    );

    // Submit a request; the result is the review artifact, never a commit.
    let assisted = router
        .clone()
        .oneshot(post_form(
            "/ui/projects/coffee/assist",
            &[
                ("branch", "main"),
                (
                    "request",
                    "add a heater block with a water inlet port and a requirement that it heats to 95C",
                ),
            ],
        ))
        .await
        .unwrap();
    assert_eq!(assisted.status(), StatusCode::OK, "{:?}", assisted);
    let html = body_text(assisted).await;
    assert!(
        html.contains("scripted-assist"),
        "the proposing agent must be named, got:
{}",
        html
    );
    assert!(
        html.contains("EditElement"),
        "the element change must render"
    );
    assert!(
        html.contains("DraftText"),
        "the requirement change must render"
    );
    assert!(
        html.contains("add a heater block with a water inlet port"),
        "the rationale must render"
    );
    assert!(
        html.contains(r#"name="message""#),
        "the accept form must offer a commit message, got:
{}",
        html
    );

    // Nothing was committed: the branch tip is unchanged and there is still one commit.
    let tip = store.branch_tip("coffee", "main").unwrap().unwrap();
    assert_eq!(tip, store.commit("coffee", &tip).unwrap().unwrap().hash);
    assert_eq!(
        store.commits_on("coffee", "main").unwrap().len(),
        1,
        "the panel must not create a commit"
    );
}

#[tokio::test]
async fn accepting_with_write_commits_and_names_both_parties() {
    use_scripted_reasoner();
    let (router, store, _dir) = app_with_auth(admin());
    seed_model_directly(store.as_ref(), minimal_model());

    let assisted = router
        .clone()
        .oneshot(post_form(
            "/ui/projects/coffee/assist",
            &[
                ("branch", "main"),
                ("request", "add a heater block and a 95C requirement"),
            ],
        ))
        .await
        .unwrap();
    assert_eq!(assisted.status(), StatusCode::OK);
    let id = store.list_proposals("coffee").unwrap()[0].id.clone();

    let accepted = router
        .clone()
        .oneshot(post_form(
            &format!("/ui/projects/coffee/proposals/{}/accept", id),
            &[("branch", "main"), ("message", "accept the proposal")],
        ))
        .await
        .unwrap();
    assert_eq!(accepted.status(), StatusCode::CREATED, "{:?}", accepted);
    let html = body_text(accepted).await;
    assert!(
        html.contains("Proposed by"),
        "the page must name the proposer, got:
{}",
        html
    );
    assert!(html.contains("scripted-assist"), "the agent must be named");
    assert!(
        html.contains("Accepted by"),
        "the page must name the acceptor"
    );
    assert!(html.contains("alex"), "the accepting human must be named");

    // The proposal is decided, accepted by the human, and the commit landed.
    let record = store.proposal("coffee", &id).unwrap().unwrap();
    assert_eq!(record.decision, Some(ProposalDecision::Accepted));
    assert_eq!(record.decided_by, "alex");
    assert!(record.commit_hash.is_some());
    let tip = store.branch_tip("coffee", "main").unwrap().unwrap();
    let model = server::api::load_model(store.as_ref(), "coffee", &tip).unwrap();
    assert!(model.structure.iter().any(|e| e.id == "heater-block"));
    assert!(model.requirements.iter().any(|r| r.id == "req-heat"));
}

#[tokio::test]
async fn a_caller_without_write_sees_no_assist_or_accept() {
    use_scripted_reasoner();
    let (router, store, _dir) = app_with_auth(viewer());
    seed_model_directly(store.as_ref(), minimal_model());

    // The model page hides the panel entirely: no request box, no accept button.
    let model = router
        .clone()
        .oneshot(get("/ui/projects/coffee/model"))
        .await
        .unwrap();
    assert_eq!(model.status(), StatusCode::OK);
    let html = body_text(model).await;
    assert!(
        !html.contains(r#"name="request""#),
        "a viewer must not see the request box, got:
{}",
        html
    );
    assert!(
        !html.contains(">Accept<"),
        "a viewer must not see an Accept button, got:
{}",
        html
    );

    // A direct assist submit is refused before anything is recorded.
    let assisted = router
        .clone()
        .oneshot(post_form(
            "/ui/projects/coffee/assist",
            &[("branch", "main"), ("request", "steal")],
        ))
        .await
        .unwrap();
    assert_eq!(assisted.status(), StatusCode::FORBIDDEN);
    let html = body_text(assisted).await;
    assert!(
        html.contains("write permission required"),
        "got:
{}",
        html
    );
    assert_eq!(
        store.list_proposals("coffee").unwrap().len(),
        0,
        "a refused assist must record nothing"
    );
}

#[tokio::test]
async fn a_refused_assist_shows_its_reason() {
    use_scripted_reasoner();
    let (router, store, _dir) = app_with_auth(admin());
    seed_model_directly(store.as_ref(), minimal_model());

    let refused = router
        .clone()
        .oneshot(post_form(
            "/ui/projects/coffee/assist",
            &[("branch", "main"), ("request", "   ")],
        ))
        .await
        .unwrap();
    assert_eq!(refused.status(), StatusCode::BAD_REQUEST, "{:?}", refused);
    let html = body_text(refused).await;
    assert!(
        html.contains("the assist request must not be empty"),
        "the refusal must render its reason, got:
{}",
        html
    );
    assert_eq!(
        store.list_proposals("coffee").unwrap().len(),
        0,
        "a refused request records nothing"
    );
}

#[tokio::test]
async fn the_proposals_page_lists_decisions_and_accepted_items() {
    use_scripted_reasoner();
    let (router, store, _dir) = app_with_auth(admin());
    seed_model_directly(store.as_ref(), minimal_model());

    // One proposal accepted through the panel flow, carrying accepted items.
    let assisted = router
        .clone()
        .oneshot(post_form(
            "/ui/projects/coffee/assist",
            &[("branch", "main"), ("request", "add a heater block")],
        ))
        .await
        .unwrap();
    assert_eq!(assisted.status(), StatusCode::OK);
    let id = store.list_proposals("coffee").unwrap()[0].id.clone();
    let accepted = router
        .clone()
        .oneshot(post_form(
            &format!("/ui/projects/coffee/proposals/{}/accept", id),
            &[("branch", "main"), ("message", "accept")],
        ))
        .await
        .unwrap();
    assert_eq!(accepted.status(), StatusCode::CREATED);

    // A second proposal refused directly, and a third left open.
    let refused = store
        .record_proposal(
            "coffee",
            "scripted-assist",
            "a refused request",
            None,
            None,
            r#"{"agent":"refused"}"#,
            None,
        )
        .unwrap();
    store
        .refuse_proposal("coffee", &refused, "alex", None)
        .unwrap();
    store
        .record_proposal(
            "coffee",
            "scripted-assist",
            "an open request",
            None,
            None,
            r#"{"agent":"open"}"#,
            None,
        )
        .unwrap();

    let page = router
        .clone()
        .oneshot(get("/ui/projects/coffee/proposals"))
        .await
        .unwrap();
    assert_eq!(page.status(), StatusCode::OK);
    let html = body_text(page).await;

    assert!(
        html.contains("scripted-assist"),
        "the agent must be named, got:
{}",
        html
    );
    assert!(
        html.contains("accepted by alex"),
        "the acceptance must name who, got:
{}",
        html
    );
    assert!(
        html.contains("refused by alex"),
        "the refusal must name who"
    );
    assert!(
        html.contains("open"),
        "the undecided proposal must read open"
    );
    assert!(
        html.contains("heater-block"),
        "the accepted items must render, got:
{}",
        html
    );
    assert!(html.contains("req-heat"), "the accepted items must render");

    // Newest first: open (recorded last), then refused, then accepted (recorded first).
    let open_pos = html.find("an open request").unwrap();
    let refused_pos = html.find("a refused request").unwrap();
    let accepted_pos = html.find("add a heater block").unwrap();
    assert!(open_pos < refused_pos, "newest first: open before refused");
    assert!(
        refused_pos < accepted_pos,
        "newest first: refused before accepted"
    );
}
