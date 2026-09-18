// SPDX-License-Identifier: AGPL-3.0-or-later
//! The agent acceptance flow, proven end to end: a proposal is persisted and fetched by id,
//! an agent token is refused at the acceptance endpoint, a human with write accepts a proposal
//! and the commit's provenance names both the agent and the human, a refusal is recorded, and
//! the acceptance's enforcement lives in the commit path - so any route reaching it inherits
//! the rule.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;

use agent::{Confidence, Proposal, ProposedAction, ScriptedReasoner};
use server::auth::{AuthConfig, Identity};
use server::store::{
    blob_hash, sqlite::SqliteStore, CommitProvenance, ImportProvenance, ProposalAcceptance,
    ProposalDecision, Store, StoreError,
};

fn store() -> (SqliteStore, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::open(&dir.path().join("mw.db")).unwrap();
    (store, dir)
}

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

fn bearer_post(uri: &str, body: Value, token: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {}", token))
        .body(Body::from(body.to_string()))
        .unwrap()
}

async fn json_body(response: axum::response::Response) -> Value {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

fn fixture(name: &str) -> Vec<u8> {
    let path = format!(
        "{}/../engine/binding-xmi/fixtures/{}",
        env!("CARGO_MANIFEST_DIR"),
        name
    );
    std::fs::read(path).expect("fixture must exist")
}

/// The six blocking losses of coffee-grinder.xmi, named by their entry identities.
const COFFEE_IDENTITIES: [&str; 6] = [
    "uml:Model model-grinder [lossy]",
    "uml:Comment doc-grinder [lossy]",
    "uml:Property prop-motor [lossy]",
    "uml:Property prop-capacity [lossy]",
    "uml:Dependency dep-satisfy [lossy]",
    "uml:Package pkg-structure (Structure) [lossy]",
];

/// A loss report naming one blocking (lossy) entry, serialized exactly as the store persists
/// it - the minimal report the store-level acceptance tests substantiate against.
fn lossy_report(artifact_hash: &str) -> String {
    let report = binding::LossReport {
        binding: binding::BindingInfo {
            id: "sysml-v1-xmi".to_string(),
            version: "2.4".to_string(),
            direction: binding::Direction::ImportAndExport,
            description: "test".to_string(),
        },
        mappings: vec![binding::Mapping {
            subject: "uml:Model model".to_string(),
            verdict: binding::MappingVerdict::Lossy,
            note: "dropped body".to_string(),
        }],
        artifact_hash: artifact_hash.to_string(),
    };
    serde_json::to_string(&report).unwrap()
}

/// A real review artifact JSON for the retained artifact's recorded loss report, produced by
/// the loss resolver with a scripted reasoner that proposes accepting every blocking loss.
fn review_artifact_json(store: &dyn Store, project: &str, artifact_hash: &str) -> String {
    let record = store
        .import_report(project, artifact_hash)
        .unwrap()
        .expect("import must be recorded");
    let report: binding::LossReport = serde_json::from_str(&record.loss_report).unwrap();
    let proposals: Vec<Proposal> = report
        .blocking()
        .iter()
        .map(|m| Proposal {
            subject: agent::losses::entry_identity(m),
            action: ProposedAction::AcceptLoss,
            rationale: m.note.clone(),
            confidence: Confidence::High,
        })
        .collect();
    let reasoner = ScriptedReasoner::new("loss-report-resolver", proposals);
    let artifact = agent::losses::propose_loss_resolutions(&report, &reasoner).unwrap();
    serde_json::to_string(&artifact).unwrap()
}

/// The accepted item identities that commit_model must substantiate against the lossy report.
fn accepted_identity() -> Vec<String> {
    vec!["uml:Model model [lossy]".to_string()]
}

#[test]
fn a_proposal_is_recorded_and_fetched_by_id() {
    let (store, _dir) = store();
    store.create_project("coffee", None).unwrap();
    let id = store
        .record_proposal(
            "coffee",
            "loss-report-resolver",
            "resolve the blocking losses",
            Some("abc123"),
            Some("sysml-v1-xmi@2.4"),
            "{\"agent\":\"loss-report-resolver\"}",
            None,
        )
        .unwrap();

    let record = store
        .proposal("coffee", &id)
        .unwrap()
        .expect("the proposal must be fetchable by id");
    assert_eq!(record.id, id);
    assert_eq!(record.project, "coffee");
    assert_eq!(record.agent, "loss-report-resolver");
    assert_eq!(record.task_goal, "resolve the blocking losses");
    assert_eq!(record.artifact_hash.as_deref(), Some("abc123"));
    assert_eq!(record.binding.as_deref(), Some("sysml-v1-xmi@2.4"));
    assert_eq!(record.decision, None);
    assert_eq!(record.decided_by, "");
    assert!(record.accepted_items.is_empty());

    // Re-recording the same proposal is a no-op that returns the same id.
    let again = store
        .record_proposal(
            "coffee",
            "loss-report-resolver",
            "resolve the blocking losses",
            Some("abc123"),
            Some("sysml-v1-xmi@2.4"),
            "{\"agent\":\"loss-report-resolver\"}",
            None,
        )
        .unwrap();
    assert_eq!(again, id);
}

#[test]
fn an_acceptance_commit_names_both_parties() {
    let (store, _dir) = store();
    store.create_project("coffee", None).unwrap();
    let artifact_hash = store.put_blob(b"source xmi bytes").unwrap();
    store
        .record_import(
            "coffee",
            &artifact_hash,
            "sysml-v1-xmi",
            "2.4",
            &lossy_report(&artifact_hash),
            "{}",
        )
        .unwrap();
    let id = store
        .record_proposal(
            "coffee",
            "loss-report-resolver",
            "resolve the blocking losses",
            Some(&artifact_hash),
            Some("sysml-v1-xmi@2.4"),
            "{\"agent\":\"loss-report-resolver\"}",
            None,
        )
        .unwrap();

    let provenance = ImportProvenance {
        artifact_hash: artifact_hash.clone(),
        binding_id: "sysml-v1-xmi".to_string(),
        binding_version: "2.4".to_string(),
        accepted_losses: accepted_identity(),
        acceptance: Some(ProposalAcceptance {
            proposal_id: id.clone(),
            agent: "loss-report-resolver".to_string(),
            accepted_by: "alex".to_string(),
        }),
    };
    let commit = store
        .commit_model(
            "coffee",
            "main",
            "okf-hash",
            "alex",
            "accept proposal",
            None,
            None,
            Some(&provenance),
        )
        .unwrap();

    match commit.provenance {
        CommitProvenance::Accepted {
            proposal_id,
            agent,
            accepted_by,
            accepted_items,
        } => {
            assert_eq!(proposal_id, id);
            assert_eq!(agent, "loss-report-resolver");
            assert_eq!(accepted_by, "alex");
            assert_eq!(accepted_items, accepted_identity());
        }
        other => panic!("expected accepted provenance, got {:?}", other),
    }

    // The proposal record itself is marked accepted, with the human and the commit, inside
    // the same transaction as the commit row.
    let record = store.proposal("coffee", &id).unwrap().unwrap();
    assert_eq!(record.decision, Some(ProposalDecision::Accepted));
    assert_eq!(record.decided_by, "alex");
    assert_eq!(record.commit_hash.as_deref(), Some(commit.hash.as_str()));
    assert_eq!(record.accepted_items, accepted_identity());
}

#[test]
fn an_acceptance_whose_proposal_is_missing_is_refused() {
    let (store, _dir) = store();
    store.create_project("coffee", None).unwrap();
    let artifact_hash = store.put_blob(b"source xmi bytes").unwrap();
    store
        .record_import(
            "coffee",
            &artifact_hash,
            "sysml-v1-xmi",
            "2.4",
            &lossy_report(&artifact_hash),
            "{}",
        )
        .unwrap();

    let provenance = ImportProvenance {
        artifact_hash: artifact_hash.clone(),
        binding_id: "sysml-v1-xmi".to_string(),
        binding_version: "2.4".to_string(),
        accepted_losses: accepted_identity(),
        acceptance: Some(ProposalAcceptance {
            proposal_id: "missing-proposal".to_string(),
            agent: "loss-report-resolver".to_string(),
            accepted_by: "alex".to_string(),
        }),
    };
    let refused = store.commit_model(
        "coffee",
        "main",
        "okf-hash",
        "alex",
        "accept proposal",
        None,
        None,
        Some(&provenance),
    );
    match refused {
        Err(StoreError::NotFound(message)) => {
            assert!(message.contains("proposal"), "message: {}", message);
        }
        other => panic!("expected NotFound, got {:?}", other.map(|c| c.hash)),
    }
    assert!(
        store.commits_on("coffee", "main").unwrap().is_empty(),
        "a refused acceptance must write no commit"
    );
}

#[test]
fn an_acceptance_whose_proposal_is_decided_is_refused() {
    let (store, _dir) = store();
    store.create_project("coffee", None).unwrap();
    let artifact_hash = store.put_blob(b"source xmi bytes").unwrap();
    store
        .record_import(
            "coffee",
            &artifact_hash,
            "sysml-v1-xmi",
            "2.4",
            &lossy_report(&artifact_hash),
            "{}",
        )
        .unwrap();
    let id = store
        .record_proposal(
            "coffee",
            "loss-report-resolver",
            "resolve the blocking losses",
            Some(&artifact_hash),
            Some("sysml-v1-xmi@2.4"),
            "{\"agent\":\"loss-report-resolver\"}",
            None,
        )
        .unwrap();
    store.refuse_proposal("coffee", &id, "alex", None).unwrap();

    let provenance = ImportProvenance {
        artifact_hash: artifact_hash.clone(),
        binding_id: "sysml-v1-xmi".to_string(),
        binding_version: "2.4".to_string(),
        accepted_losses: accepted_identity(),
        acceptance: Some(ProposalAcceptance {
            proposal_id: id.clone(),
            agent: "loss-report-resolver".to_string(),
            accepted_by: "alex".to_string(),
        }),
    };
    let refused = store.commit_model(
        "coffee",
        "main",
        "okf-hash",
        "alex",
        "accept proposal",
        None,
        None,
        Some(&provenance),
    );
    match refused {
        Err(StoreError::Conflict(message)) => {
            assert!(message.contains("decided"), "message: {}", message);
        }
        other => panic!("expected Conflict, got {:?}", other.map(|c| c.hash)),
    }
    assert!(
        store.commits_on("coffee", "main").unwrap().is_empty(),
        "an acceptance of a decided proposal must write no commit"
    );
}

#[tokio::test]
async fn a_proposal_is_recorded_over_http_and_fetched_by_id() {
    let (router, store, _dir) = app_with_auth(AuthConfig::fixed(Identity {
        subject: "alex".to_string(),
        roles: vec!["admin".to_string()],
        projects: vec!["*".to_string()],
    }));

    // Retain the artifact and record its report by attempting an import without acceptances.
    let created = router
        .clone()
        .oneshot(post("/projects", json!({ "name": "coffee" })))
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::CREATED);
    let bytes = fixture("coffee-grinder.xmi");
    let artifact_hash = blob_hash(&bytes);
    let import = router
        .clone()
        .oneshot(post(
            "/projects/coffee/import",
            json!({
                "artifact": String::from_utf8(bytes).unwrap(),
                "binding": "sysml-v1-xmi@2.4",
                "branch": "main",
                "message": "import",
                "acceptLosses": []
            }),
        ))
        .await
        .unwrap();
    assert_eq!(import.status(), StatusCode::UNPROCESSABLE_ENTITY);

    // The agent records a real review artifact over HTTP, and its identity is the verified
    // subject, never the artifact's own agent text.
    let artifact = review_artifact_json(store.as_ref(), "coffee", &artifact_hash);
    let recorded = router
        .clone()
        .oneshot(post(
            "/projects/coffee/proposals",
            serde_json::from_str::<Value>(&artifact).unwrap(),
        ))
        .await
        .unwrap();
    assert_eq!(recorded.status(), StatusCode::CREATED, "{:?}", recorded);
    let body = json_body(recorded).await;
    let id = body["id"].as_str().unwrap().to_string();
    assert_eq!(body["agent"], "alex");
    assert_eq!(body["artifactHash"], artifact_hash.as_str());
    assert_eq!(body["binding"], "sysml-v1-xmi@2.4");

    let fetched = router
        .clone()
        .oneshot(get(&format!("/projects/coffee/proposals/{}", id)))
        .await
        .unwrap();
    assert_eq!(fetched.status(), StatusCode::OK);
    let body = json_body(fetched).await;
    assert_eq!(body["id"], id);
    assert_eq!(body["agent"], "alex");
    assert_eq!(body["decision"], Value::Null);
}

#[tokio::test]
async fn an_agent_is_refused_at_the_acceptance_endpoint() {
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
    let id = store
        .record_proposal(
            "coffee",
            "loss-report-resolver",
            "resolve the blocking losses",
            None,
            None,
            "{\"agent\":\"loss-report-resolver\"}",
            None,
        )
        .unwrap();

    let response = router
        .oneshot(bearer_post(
            &format!("/projects/coffee/proposals/{}/accept", id),
            json!({ "acceptedItems": ["x"], "branch": "main", "message": "m" }),
            "agent-tok",
        ))
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::FORBIDDEN,
        "an agent token must be refused at the acceptance endpoint"
    );

    // Nothing was decided: the proposal stays undecided.
    let record = store.proposal("coffee", &id).unwrap().unwrap();
    assert_eq!(record.decision, None);
}

#[tokio::test]
async fn a_human_with_write_accepts_a_proposal_and_the_commit_names_both() {
    let (router, store, _dir) = app_with_auth(AuthConfig::fixed(Identity {
        subject: "alex".to_string(),
        roles: vec!["admin".to_string()],
        projects: vec!["*".to_string()],
    }));

    let created = router
        .clone()
        .oneshot(post("/projects", json!({ "name": "coffee" })))
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::CREATED);
    let bytes = fixture("coffee-grinder.xmi");
    let artifact_hash = blob_hash(&bytes);
    let import = router
        .clone()
        .oneshot(post(
            "/projects/coffee/import",
            json!({
                "artifact": String::from_utf8(bytes).unwrap(),
                "binding": "sysml-v1-xmi@2.4",
                "branch": "main",
                "message": "import",
                "acceptLosses": []
            }),
        ))
        .await
        .unwrap();
    assert_eq!(import.status(), StatusCode::UNPROCESSABLE_ENTITY);

    let id = store
        .record_proposal(
            "coffee",
            "loss-report-resolver",
            "resolve the blocking losses",
            Some(&artifact_hash),
            Some("sysml-v1-xmi@2.4"),
            "{\"agent\":\"loss-report-resolver\"}",
            None,
        )
        .unwrap();

    let accepted = router
        .clone()
        .oneshot(post(
            &format!("/projects/coffee/proposals/{}/accept", id),
            json!({
                "acceptedItems": COFFEE_IDENTITIES,
                "branch": "main",
                "message": "accept the proposal"
            }),
        ))
        .await
        .unwrap();
    assert_eq!(accepted.status(), StatusCode::CREATED, "{:?}", accepted);
    let body = json_body(accepted).await;

    let provenance = &body["commit"]["provenance"];
    assert_eq!(provenance["kind"], "accepted");
    assert_eq!(provenance["proposalId"], id);
    assert_eq!(provenance["agent"], "loss-report-resolver");
    assert_eq!(provenance["acceptedBy"], "alex");
    let accepted_items: Vec<&str> = provenance["acceptedItems"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(accepted_items, COFFEE_IDENTITIES);

    // The proposal record is marked accepted with the human and the commit.
    assert_eq!(body["proposal"]["decision"], "accepted");
    assert_eq!(body["proposal"]["decidedBy"], "alex");
    assert_eq!(
        body["proposal"]["commitHash"], body["commit"]["hash"],
        "the proposal must name the commit it produced"
    );
}

#[tokio::test]
async fn a_refusal_is_recorded() {
    let (router, store, _dir) = app_with_auth(AuthConfig::fixed(Identity {
        subject: "alex".to_string(),
        roles: vec!["admin".to_string()],
        projects: vec!["*".to_string()],
    }));
    store.create_project("coffee", None).unwrap();
    let id = store
        .record_proposal(
            "coffee",
            "loss-report-resolver",
            "resolve the blocking losses",
            None,
            None,
            "{\"agent\":\"loss-report-resolver\"}",
            None,
        )
        .unwrap();

    let refused = router
        .clone()
        .oneshot(post(
            &format!("/projects/coffee/proposals/{}/refuse", id),
            json!({ "reason": "not enough confidence" }),
        ))
        .await
        .unwrap();
    assert_eq!(refused.status(), StatusCode::OK, "{:?}", refused);
    let body = json_body(refused).await;
    assert_eq!(body["decision"], "refused");
    assert_eq!(body["decidedBy"], "alex");

    // The refusal is on the audit trail, with the verified actor and the reason.
    let audit = router
        .clone()
        .oneshot(get("/projects/coffee/audit"))
        .await
        .unwrap();
    assert_eq!(audit.status(), StatusCode::OK);
    let entries = json_body(audit).await;
    let refusal = entries
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["action"] == "proposal.refused")
        .expect("a proposal.refused entry must be recorded");
    assert_eq!(refusal["actor"], "alex");
    assert_eq!(refusal["subject"], id);
    assert!(refusal["detail"]
        .as_str()
        .unwrap()
        .contains("not enough confidence"));

    // The proposal is decided: accepting or refusing again is a conflict.
    let record = store.proposal("coffee", &id).unwrap().unwrap();
    assert_eq!(record.decision, Some(ProposalDecision::Refused));
    assert_eq!(record.decided_by, "alex");
}
