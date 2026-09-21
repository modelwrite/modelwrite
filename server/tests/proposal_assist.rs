// SPDX-License-Identifier: AGPL-3.0-or-later
//! The assist loop, proven end to end: a scripted reasoner turns a request into a proposal
//! (a review artifact, never a commit), the assist endpoint writes nothing, a human with write
//! accepts the proposal and the commit's provenance names both the agent and the human, and a
//! proposal that would produce an invalid document is refused with the validator's own errors.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;

use agent::{Confidence, ProposedAction};
use okf::types::Requirement;
use server::assist::{build_review_artifact, ModelChange};
use server::auth::{AuthConfig, Identity};
use server::store::{sqlite::SqliteStore, Store};

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

/// The fixed admin identity every assist test runs as: a human with write.
fn admin() -> AuthConfig {
    AuthConfig::fixed(Identity {
        subject: "alex".to_string(),
        roles: vec!["admin".to_string()],
        projects: vec!["*".to_string()],
        trial_id: None,
    })
}

/// Force the deterministic scripted reasoner, so no test ever reaches the network.
fn use_scripted_reasoner() {
    std::env::set_var("MW_ASSIST_REASONER", "scripted");
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

async fn json_body(response: axum::response::Response) -> Value {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

/// Create the project and seed a minimal, valid model on main. Returns the seed commit hash.
async fn seed_model(router: &axum::Router) -> String {
    let created = router
        .clone()
        .oneshot(post("/projects", json!({ "name": "coffee" })))
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::CREATED);
    let seeded = router
        .clone()
        .oneshot(post(
            "/projects/coffee/commits",
            json!({
                "branch": "main",
                "message": "seed",
                "okf": {
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
                }
            }),
        ))
        .await
        .unwrap();
    assert_eq!(seeded.status(), StatusCode::CREATED, "{:?}", seeded);
    json_body(seeded).await["hash"]
        .as_str()
        .unwrap()
        .to_string()
}

#[tokio::test]
async fn a_scripted_assist_produces_a_review_artifact_and_writes_no_commit() {
    use_scripted_reasoner();
    let (router, store, _dir) = app_with_auth(admin());
    let seed = seed_model(&router).await;

    let assisted = router
        .clone()
        .oneshot(post(
            "/projects/coffee/assist",
            json!({
                "request": "add a heater block with a water inlet port and a requirement that it heats to 95C",
                "branch": "main"
            }),
        ))
        .await
        .unwrap();
    assert_eq!(assisted.status(), StatusCode::CREATED, "{:?}", assisted);
    let body = json_body(assisted).await;
    let id = body["id"].as_str().unwrap().to_string();

    // The response IS the review artifact, in the existing shape a human reads everywhere.
    let artifact = &body["reviewArtifact"];
    assert_eq!(artifact["agent"], "scripted-assist");
    assert_eq!(
        artifact["task"]["goal"],
        "add a heater block with a water inlet port and a requirement that it heats to 95C"
    );
    assert_eq!(artifact["proposals"].as_array().unwrap().len(), 2);
    assert_eq!(artifact["proposals"][0]["action"], "EditElement");
    assert_eq!(artifact["proposals"][1]["action"], "DraftText");
    // The candidate document rides the material, so the human reads exactly what acceptance
    // would apply.
    assert!(artifact["task"]["material"]["Document"].is_object());

    // The assist endpoint performed NO write: the branch tip is still the seed commit.
    let tip = store.branch_tip("coffee", "main").unwrap().unwrap();
    assert_eq!(tip, seed, "assist must not move the branch tip");
    let commits = store.commits_on("coffee", "main").unwrap();
    assert_eq!(commits.len(), 1, "assist must not create a commit");

    // But the proposal IS recorded, addressable by id, undecided.
    let record = store
        .proposal("coffee", &id)
        .unwrap()
        .expect("proposal recorded");
    assert_eq!(record.agent, "scripted-assist");
    assert_eq!(record.decision, None);
}

#[tokio::test]
async fn a_proposal_reports_the_graph_orphans_and_uncovered_requirements_it_would_create() {
    use_scripted_reasoner();
    let (router, _store, _dir) = app_with_auth(admin());
    seed_model(&router).await;

    let assisted = router
        .clone()
        .oneshot(post(
            "/projects/coffee/assist",
            json!({ "request": "add a heater block and a 95C requirement", "branch": "main" }),
        ))
        .await
        .unwrap();
    assert_eq!(assisted.status(), StatusCode::CREATED);
    let body = json_body(assisted).await;

    // The review artifact carries the validator-and-gate result BEFORE any acceptance, so a
    // human sees that the candidate would leave isolated nodes and an uncovered requirement.
    let check = &body["reviewArtifact"]["check"];
    assert_eq!(
        check["passed"], false,
        "the candidate leaves isolated nodes and more than one component"
    );
    let isolated: Vec<&str> = check["isolatedNodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert!(
        isolated.contains(&"heater-block"),
        "the new element must be reported isolated: {:?}",
        isolated
    );
    assert!(
        isolated.contains(&"req-heat"),
        "the new requirement must be reported isolated: {:?}",
        isolated
    );
    assert_eq!(check["componentCount"], 3, "one node per isolated element");
    let uncovered: Vec<&str> = check["uncoveredRequirements"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert!(
        uncovered.contains(&"req-heat"),
        "the uncovered requirement must be reported: {:?}",
        uncovered
    );
}

#[tokio::test]
async fn an_accepted_change_writes_a_model_whose_summary_agrees_with_its_content() {
    use_scripted_reasoner();
    let (router, store, _dir) = app_with_auth(admin());
    seed_model(&router).await;

    let assisted = router
        .clone()
        .oneshot(post(
            "/projects/coffee/assist",
            json!({ "request": "add a heater block and a 95C requirement", "branch": "main" }),
        ))
        .await
        .unwrap();
    assert_eq!(assisted.status(), StatusCode::CREATED);
    let id = json_body(assisted).await["id"]
        .as_str()
        .unwrap()
        .to_string();

    let accepted = router
        .clone()
        .oneshot(post(
            &format!("/projects/coffee/proposals/{}/accept", id),
            json!({ "branch": "main", "message": "accept the proposal" }),
        ))
        .await
        .unwrap();
    assert_eq!(accepted.status(), StatusCode::CREATED, "{:?}", accepted);

    let tip = store.branch_tip("coffee", "main").unwrap().unwrap();
    let model = server::api::load_model(store.as_ref(), "coffee", &tip).unwrap();
    // The accepted model's summary is DERIVED, never trusted: every count agrees with the
    // document it describes.
    assert_eq!(model.summary.blocks as usize, model.structure.len());
    assert_eq!(
        model.summary.requirements as usize,
        model.requirements.len()
    );
    assert_eq!(
        model.summary.graph_nodes as usize,
        model.graph.as_ref().unwrap().nodes.len()
    );
    assert_eq!(
        model.summary.graph_edges as usize,
        model.graph.as_ref().unwrap().edges.len()
    );
}

#[tokio::test]
async fn a_human_accepts_a_model_change_and_the_commit_names_both() {
    use_scripted_reasoner();
    let (router, store, _dir) = app_with_auth(admin());
    seed_model(&router).await;

    let assisted = router
        .clone()
        .oneshot(post(
            "/projects/coffee/assist",
            json!({ "request": "add a heater block and a 95C requirement", "branch": "main" }),
        ))
        .await
        .unwrap();
    assert_eq!(assisted.status(), StatusCode::CREATED);
    let id = json_body(assisted).await["id"]
        .as_str()
        .unwrap()
        .to_string();

    let accepted = router
        .clone()
        .oneshot(post(
            &format!("/projects/coffee/proposals/{}/accept", id),
            json!({ "branch": "main", "message": "accept the proposal" }),
        ))
        .await
        .unwrap();
    assert_eq!(accepted.status(), StatusCode::CREATED, "{:?}", accepted);
    let body = json_body(accepted).await;

    let provenance = &body["commit"]["provenance"];
    assert_eq!(provenance["kind"], "accepted");
    assert_eq!(provenance["proposalId"], id);
    assert_eq!(provenance["agent"], "scripted-assist");
    assert_eq!(provenance["acceptedBy"], "alex");
    let accepted_items: Vec<&str> = provenance["acceptedItems"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(accepted_items, vec!["heater-block", "req-heat"]);

    // The proposal record is marked accepted with the human and the commit.
    assert_eq!(body["proposal"]["decision"], "accepted");
    assert_eq!(body["proposal"]["decidedBy"], "alex");
    assert_eq!(body["proposal"]["commitHash"], body["commit"]["hash"]);

    // The accepted model really contains the proposed elements.
    let tip = store.branch_tip("coffee", "main").unwrap().unwrap();
    let model = server::api::load_model(store.as_ref(), "coffee", &tip).unwrap();
    assert!(model.structure.iter().any(|e| e.id == "heater-block"));
    assert!(model.requirements.iter().any(|r| r.id == "req-heat"));
    assert_eq!(
        model.requirements[0].req_text,
        "the heater block heats water to 95C"
    );
}

#[tokio::test]
async fn an_invalid_model_change_is_refused_with_the_validators_errors() {
    use_scripted_reasoner();
    let (router, store, _dir) = app_with_auth(admin());
    seed_model(&router).await;

    // Craft a proposal whose candidate document is invalid: a requirement with an empty reqId.
    let tip = store.branch_tip("coffee", "main").unwrap().unwrap();
    let current = server::api::load_model(store.as_ref(), "coffee", &tip).unwrap();
    let change = ModelChange {
        action: ProposedAction::DraftText,
        element: None,
        requirement: Some(Requirement {
            id: "req-bad".to_string(),
            name: "Bad requirement".to_string(),
            kind: "requirement".to_string(),
            stereotypes: Vec::new(),
            attributes: Vec::new(),
            documentation: String::new(),
            req_id: String::new(),
            req_text: "no reqId".to_string(),
        }),
        rationale: "deliberately invalid".to_string(),
        confidence: Confidence::High,
    };
    let artifact = build_review_artifact(
        "scripted-assist",
        "add an invalid requirement",
        &current,
        &[change],
    );
    let artifact_json = serde_json::to_string(&artifact).unwrap();
    let id = store
        .record_proposal(
            "coffee",
            "scripted-assist",
            "add an invalid requirement",
            None,
            None,
            &artifact_json,
            None,
        )
        .unwrap();

    let accepted = router
        .clone()
        .oneshot(post(
            &format!("/projects/coffee/proposals/{}/accept", id),
            json!({ "branch": "main", "message": "accept" }),
        ))
        .await
        .unwrap();
    assert_eq!(
        accepted.status(),
        StatusCode::UNPROCESSABLE_ENTITY,
        "{:?}",
        accepted
    );
    let body = json_body(accepted).await;
    let error = body["error"].as_str().unwrap();
    assert!(
        error.contains("empty reqId"),
        "the refusal must carry the validator's own error, got: {}",
        error
    );

    // Nothing was committed and the proposal stays undecided.
    assert_eq!(store.commits_on("coffee", "main").unwrap().len(), 1);
    assert_eq!(
        store.proposal("coffee", &id).unwrap().unwrap().decision,
        None
    );
}

#[tokio::test]
async fn proposals_list_is_newest_first_with_decisions() {
    use_scripted_reasoner();
    let (router, store, _dir) = app_with_auth(admin());
    seed_model(&router).await;

    // Record two proposals directly, then check the list endpoint returns both, newest first.
    let first = store
        .record_proposal(
            "coffee",
            "scripted-assist",
            "first",
            None,
            None,
            r#"{"agent":"first"}"#,
            None,
        )
        .unwrap();
    let second = store
        .record_proposal(
            "coffee",
            "scripted-assist",
            "second",
            None,
            None,
            r#"{"agent":"second"}"#,
            None,
        )
        .unwrap();
    store
        .refuse_proposal("coffee", &first, "alex", None)
        .unwrap();

    let listed = router
        .clone()
        .oneshot(get("/projects/coffee/proposals"))
        .await
        .unwrap();
    assert_eq!(listed.status(), StatusCode::OK);
    let body = json_body(listed).await;
    let items = body.as_array().unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["id"], second, "newest first");
    assert_eq!(items[1]["id"], first);
    assert_eq!(items[1]["decision"], "refused");
    assert_eq!(items[1]["decidedBy"], "alex");
}
