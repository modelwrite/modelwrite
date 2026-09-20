// SPDX-License-Identifier: AGPL-3.0-or-later
//! Acceptance resumes from the retained artifact: a refused import is completed by naming its
//! artifact hash and the accepted losses - the retained bytes are re-read by hash, never
//! re-uploaded. The acceptance names the hash it applies to, a missing retained artifact is a
//! NAMED failure (not a fall back to asking for a file), and the workbench offers the same
//! hash-based acceptance as the JSON API.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use server::store::Store;
use tower::ServiceExt;

fn state(dir: &std::path::Path) -> server::AppState {
    let store = server::store::sqlite::SqliteStore::open(&dir.join("mw.db")).unwrap();
    server::AppState {
        store: std::sync::Arc::new(store),
        evidence_dir: dir.to_path_buf(),
        auth: server::auth::AuthConfig::Open,
    }
}

fn post_json(uri: &str, body: serde_json::Value) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
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

fn get(uri: &str) -> Request<Body> {
    Request::builder().uri(uri).body(Body::empty()).unwrap()
}

async fn json_body(response: axum::response::Response) -> serde_json::Value {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

async fn body_text(response: axum::response::Response) -> String {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8(bytes.to_vec()).unwrap()
}

async fn create_project(router: &axum::Router, name: &str) {
    let response = router
        .clone()
        .oneshot(post_json("/projects", serde_json::json!({ "name": name })))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
}

fn fixture(name: &str) -> Vec<u8> {
    let path = format!(
        "{}/../engine/binding-xmi/fixtures/{}",
        env!("CARGO_MANIFEST_DIR"),
        name
    );
    std::fs::read(path).expect("fixture must exist")
}

/// The entry identities of the six blocking losses of coffee-grinder.xmi.
const COFFEE_IDENTITIES: [&str; 6] = [
    "uml:Model model-grinder [lossy]",
    "uml:Comment doc-grinder [lossy]",
    "uml:Property prop-motor [lossy]",
    "uml:Property prop-capacity [lossy]",
    "uml:Dependency dep-satisfy [lossy]",
    "uml:Package pkg-structure (Structure) [lossy]",
];

/// Reconstruct entry identities from a JSON `blocking` array, exactly as `entry_identity`
/// formats them ("subject [verdict]"), so a test can accept every blocking loss of any fixture
/// without hardcoding the subjects.
fn identities_from_blocking(blocking: &serde_json::Value) -> Vec<String> {
    blocking
        .as_array()
        .expect("blocking must be an array")
        .iter()
        .map(|m| {
            let subject = m["subject"].as_str().unwrap();
            let verdict = m["verdict"].as_str().unwrap().to_lowercase();
            format!("{} [{}]", subject, verdict)
        })
        .collect()
}

#[tokio::test]
async fn a_refused_import_is_accepted_by_hash_and_the_commit_names_the_hash() {
    // THE ROUND TRIP: import a model refused as Blocking, then accept its blocking losses with
    // ONLY the artifact hash in hand. The acceptance re-reads the retained bytes by hash and
    // the commit's provenance names the hash and the accepted losses.
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    create_project(&router, "coffee").await;

    let bytes = fixture("coffee-grinder.xmi");
    let expected_hash = server::store::blob_hash(&bytes);

    let refused = router
        .clone()
        .oneshot(post_json(
            "/projects/coffee/import",
            serde_json::json!({
                "artifact": String::from_utf8(bytes.clone()).unwrap(),
                "binding": "sysml-v1-xmi@2.4",
                "branch": "main",
                "author": "alex",
                "message": "import coffee-grinder",
                "acceptLosses": []
            }),
        ))
        .await
        .unwrap();
    assert_eq!(refused.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let refused_body = json_body(refused).await;
    let artifact_hash = refused_body["artifactHash"]
        .as_str()
        .expect("the JSON refusal must name the retained artifact hash")
        .to_string();
    assert_eq!(artifact_hash, expected_hash);

    // The acceptance request carries the hash and the accepted loss names - no artifact, no
    // binding, nothing else the caller had to keep hold of.
    let accepted = router
        .clone()
        .oneshot(post_json(
            &format!("/projects/coffee/import/{}/accept", artifact_hash),
            serde_json::json!({
                "acceptLosses": COFFEE_IDENTITIES,
                "branch": "main",
                "author": "alex",
                "message": "accept the coffee-grinder losses"
            }),
        ))
        .await
        .unwrap();
    assert_eq!(accepted.status(), StatusCode::CREATED, "{:?}", accepted);
    let accepted_body = json_body(accepted).await;

    let provenance = &accepted_body["commit"]["provenance"];
    assert_eq!(provenance["kind"], "imported");
    assert_eq!(provenance["artifactHash"], artifact_hash.as_str());
    assert_eq!(provenance["bindingId"], "sysml-v1-xmi");
    assert_eq!(provenance["bindingVersion"], "2.4");
    let accepted_losses: Vec<&str> = provenance["acceptedLosses"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(accepted_losses, COFFEE_IDENTITIES);

    // BYTE IDENTITY: the bytes the acceptance migrated are the bytes that were retained - the
    // artifact fetched by hash is unchanged, and equals the source.
    let store = server::store::sqlite::SqliteStore::open(&dir.path().join("mw.db")).unwrap();
    assert_eq!(
        store.blob(&artifact_hash).unwrap().as_deref(),
        Some(&bytes[..]),
        "the retained artifact must be byte-identical to the source"
    );
    let fetched = router
        .clone()
        .oneshot(get(&format!(
            "/projects/coffee/import/{}/artifact",
            artifact_hash
        )))
        .await
        .unwrap();
    assert_eq!(fetched.status(), StatusCode::OK);
    let fetched_bytes = fetched.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(
        fetched_bytes.as_ref(),
        bytes,
        "the artifact fetched by hash must be byte-for-byte the retained bytes"
    );
}

#[tokio::test]
async fn an_incomplete_acceptance_names_the_remaining_losses() {
    // Accepting only SOME blocking losses refuses again, naming the losses still unaccepted -
    // and it does so from the hash, never by asking for the file.
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    create_project(&router, "coffee").await;

    let bytes = fixture("coffee-grinder.xmi");
    let refused = router
        .clone()
        .oneshot(post_json(
            "/projects/coffee/import",
            serde_json::json!({
                "artifact": String::from_utf8(bytes).unwrap(),
                "binding": "sysml-v1-xmi@2.4",
                "branch": "main",
                "message": "import",
                "acceptLosses": []
            }),
        ))
        .await
        .unwrap();
    assert_eq!(refused.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let artifact_hash = json_body(refused).await["artifactHash"]
        .as_str()
        .unwrap()
        .to_string();

    let incomplete = router
        .clone()
        .oneshot(post_json(
            &format!("/projects/coffee/import/{}/accept", artifact_hash),
            serde_json::json!({
                "acceptLosses": ["uml:Model model-grinder [lossy]"],
                "branch": "main",
                "message": "accept one loss"
            }),
        ))
        .await
        .unwrap();
    assert_eq!(incomplete.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body = json_body(incomplete).await;
    assert_eq!(body["artifactHash"], artifact_hash.as_str());
    let blocking = body["blocking"].as_array().unwrap();
    assert_eq!(blocking.len(), 5, "five losses remain unaccepted");

    // Nothing committed.
    let store = server::store::sqlite::SqliteStore::open(&dir.path().join("mw.db")).unwrap();
    assert!(store.commits_on("coffee", "main").unwrap().is_empty());
}

#[tokio::test]
async fn a_missing_retained_artifact_is_named_not_a_upload_prompt() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    create_project(&router, "coffee").await;

    let bytes = fixture("coffee-grinder.xmi");
    let refused = router
        .clone()
        .oneshot(post_json(
            "/projects/coffee/import",
            serde_json::json!({
                "artifact": String::from_utf8(bytes.clone()).unwrap(),
                "binding": "sysml-v1-xmi@2.4",
                "branch": "main",
                "message": "import",
                "acceptLosses": []
            }),
        ))
        .await
        .unwrap();
    assert_eq!(refused.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let artifact_hash = json_body(refused).await["artifactHash"]
        .as_str()
        .unwrap()
        .to_string();

    // Simulate the platform losing the retained bytes: the import record remains, the blob is
    // gone. The acceptance must name the missing hash, not ask for the file again.
    {
        let connection = rusqlite::Connection::open(dir.path().join("mw.db")).unwrap();
        connection
            .execute(
                "DELETE FROM blobs WHERE hash = ?1",
                rusqlite::params![artifact_hash],
            )
            .unwrap();
    }

    let accepted = router
        .clone()
        .oneshot(post_json(
            &format!("/projects/coffee/import/{}/accept", artifact_hash),
            serde_json::json!({
                "acceptLosses": COFFEE_IDENTITIES,
                "branch": "main",
                "message": "accept"
            }),
        ))
        .await
        .unwrap();
    assert_eq!(
        accepted.status(),
        StatusCode::INTERNAL_SERVER_ERROR,
        "{:?}",
        accepted
    );
    let body = json_body(accepted).await;
    let error = body["error"].as_str().unwrap();
    assert!(
        error.contains(&artifact_hash),
        "the failure must NAME the missing hash, got: {}",
        error
    );
    assert!(
        error.contains("missing"),
        "the failure must say the artifact is missing, got: {}",
        error
    );
    assert!(
        !error.contains("upload") && !error.contains("re-submit"),
        "the failure must not fall back to asking for a file, got: {}",
        error
    );
}

#[tokio::test]
async fn accepting_a_hash_with_no_import_record_is_a_named_404() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    create_project(&router, "coffee").await;

    let accepted = router
        .clone()
        .oneshot(post_json(
            "/projects/coffee/import/deadbeef/accept",
            serde_json::json!({
                "acceptLosses": ["uml:Model x [lossy]"],
                "branch": "main",
                "message": "accept"
            }),
        ))
        .await
        .unwrap();
    assert_eq!(accepted.status(), StatusCode::NOT_FOUND);
    let body = json_body(accepted).await;
    assert!(
        body["error"].as_str().unwrap().contains("deadbeef"),
        "the 404 must name the hash, got: {}",
        body["error"]
    );
}

#[tokio::test]
async fn the_coffee_machine_xmi_import_accepts_from_hash() {
    // The REAL MagicDraw coffee-machine model (real-world/mdzip/model.xmi, 193 KB XMI): a
    // full loss report with 252 blocking losses, refused as Blocking, then accepted with ONLY
    // the hash in hand. The retained bytes are re-read by hash and the migration completes.
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    create_project(&router, "coffee").await;

    let path = format!(
        "{}/../real-world/mdzip/model.xmi",
        env!("CARGO_MANIFEST_DIR")
    );
    let bytes = std::fs::read(path).expect("the coffee-machine XMI must be committed");
    let expected_hash = server::store::blob_hash(&bytes);

    let refused = router
        .clone()
        .oneshot(post_json(
            "/projects/coffee/import",
            serde_json::json!({
                "artifact": String::from_utf8(bytes.clone()).unwrap(),
                "binding": "sysml-v1-xmi@2.4",
                "branch": "main",
                "author": "alex",
                "message": "import coffee-machine",
                "acceptLosses": []
            }),
        ))
        .await
        .unwrap();
    assert_eq!(refused.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let refused_body = json_body(refused).await;
    let artifact_hash = refused_body["artifactHash"].as_str().unwrap().to_string();
    assert_eq!(artifact_hash, expected_hash);
    let blocking_count = refused_body["blocking"].as_array().unwrap().len();
    let accept_losses = identities_from_blocking(&refused_body["blocking"]);
    // Some blocking entries share a (subject, verdict) pair, so their entry identities
    // collapse; acceptance covers every entry with the unique identities.
    let mut unique = accept_losses.clone();
    unique.sort();
    unique.dedup();
    let unique_count = unique.len();
    assert_eq!(
        unique_count, blocking_count,
        "every blocking entry must carry a unique entry identity"
    );

    let accepted = router
        .clone()
        .oneshot(post_json(
            &format!("/projects/coffee/import/{}/accept", artifact_hash),
            serde_json::json!({
                "acceptLosses": accept_losses,
                "branch": "main",
                "author": "alex",
                "message": "accept coffee-machine losses"
            }),
        ))
        .await
        .unwrap();
    assert_eq!(accepted.status(), StatusCode::CREATED, "{:?}", accepted);
    let accepted_body = json_body(accepted).await;
    let provenance = &accepted_body["commit"]["provenance"];
    assert_eq!(provenance["artifactHash"], artifact_hash.as_str());
    assert_eq!(provenance["kind"], "imported");
    assert_eq!(
        provenance["acceptedLosses"].as_array().unwrap().len(),
        unique_count,
        "every blocking entry must be accepted by its unique entry identity"
    );

    println!("=== COFFEE-MACHINE ACCEPTANCE ROUND TRIP ===");
    println!("blocking entries     : {}", blocking_count);
    println!("unique identities    : {}", unique_count);
    println!("artifact hash        : {}", artifact_hash);
    println!("commit               : {}", accepted_body["commit"]["hash"]);

    // BYTE IDENTITY: the retained bytes are unchanged and match the source.
    let store = server::store::sqlite::SqliteStore::open(&dir.path().join("mw.db")).unwrap();
    assert_eq!(
        store.blob(&artifact_hash).unwrap().as_deref(),
        Some(&bytes[..]),
        "the retained coffee-machine artifact must be byte-identical"
    );
}

#[tokio::test]
async fn accepting_one_use_case_reference_does_not_accept_its_twins() {
    // The coffee-machine model owns five use cases as five <useCase xmi:idref=.../>
    // references. Before the subject fix those five collapsed to one identity, so a
    // human could not accept one without accepting the other four. Now each names its
    // idref, and accepting ONE of them must leave the other four blocking - the
    // acceptance key names exactly one loss.
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    create_project(&router, "coffee").await;

    let path = format!(
        "{}/../real-world/mdzip/model.xmi",
        env!("CARGO_MANIFEST_DIR")
    );
    let bytes = std::fs::read(path).expect("the coffee-machine XMI must be committed");

    let refused = router
        .clone()
        .oneshot(post_json(
            "/projects/coffee/import",
            serde_json::json!({
                "artifact": String::from_utf8(bytes).unwrap(),
                "binding": "sysml-v1-xmi@2.4",
                "branch": "main",
                "message": "import coffee-machine",
                "acceptLosses": []
            }),
        ))
        .await
        .unwrap();
    assert_eq!(refused.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let refused_body = json_body(refused).await;
    let artifact_hash = refused_body["artifactHash"].as_str().unwrap().to_string();
    let blocking = refused_body["blocking"].as_array().unwrap().clone();

    let use_case_identities: Vec<String> = blocking
        .iter()
        .filter(|m| m["subject"].as_str().unwrap().starts_with("useCase "))
        .map(|m| {
            let subject = m["subject"].as_str().unwrap();
            let verdict = m["verdict"].as_str().unwrap().to_lowercase();
            format!("{} [{}]", subject, verdict)
        })
        .collect();
    assert_eq!(
        use_case_identities.len(),
        5,
        "the five useCase references must be five distinct identities, got: {:?}",
        use_case_identities
    );
    let mut distinct = use_case_identities.clone();
    distinct.sort();
    distinct.dedup();
    assert_eq!(
        distinct.len(),
        5,
        "no two useCase references may share an identity"
    );

    // Accept exactly ONE useCase reference; the other four must stay blocking.
    let accepted = router
        .clone()
        .oneshot(post_json(
            &format!("/projects/coffee/import/{}/accept", artifact_hash),
            serde_json::json!({
                "acceptLosses": [use_case_identities[0]],
                "branch": "main",
                "message": "accept one useCase reference"
            }),
        ))
        .await
        .unwrap();
    assert_eq!(accepted.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let accepted_body = json_body(accepted).await;
    let remaining: Vec<String> = accepted_body["blocking"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| {
            let subject = m["subject"].as_str().unwrap();
            let verdict = m["verdict"].as_str().unwrap().to_lowercase();
            format!("{} [{}]", subject, verdict)
        })
        .collect();
    let remaining_use_cases: Vec<&String> = remaining
        .iter()
        .filter(|id| id.starts_with("useCase "))
        .collect();
    assert_eq!(
        remaining_use_cases.len(),
        4,
        "accepting one useCase reference must leave the other four blocking, got: {:?}",
        remaining_use_cases
    );
    assert!(
        !remaining.contains(&use_case_identities[0]),
        "the accepted useCase reference must no longer be blocking"
    );

    // Nothing committed: an incomplete acceptance refuses, it never silently
    // accepts the four un-named twins.
    let store = server::store::sqlite::SqliteStore::open(&dir.path().join("mw.db")).unwrap();
    assert!(store.commits_on("coffee", "main").unwrap().is_empty());
}

#[tokio::test]
async fn the_workbench_offers_acceptance_from_the_hash_after_a_refusal() {
    // The workbench refusal page offers the acceptance directly - the acceptance form carries
    // the artifact hash, not the artifact - and accepting completes the migration from the
    // retained bytes. The re-upload form stays the way to start a NEW import.
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    create_project(&router, "coffee").await;

    let fixture_text = String::from_utf8(fixture("coffee-grinder.xmi")).unwrap();
    let expected_hash = server::store::blob_hash(fixture_text.as_bytes());

    let refused = router
        .clone()
        .oneshot(post_form(
            "/ui/projects/coffee/import",
            &[
                ("binding", "sysml-v1-xmi@2.4"),
                ("branch", "main"),
                ("message", "import coffee-grinder"),
                ("artifact", &fixture_text),
            ],
        ))
        .await
        .unwrap();
    assert_eq!(refused.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let html = body_text(refused).await;
    assert!(
        html.contains("Import refused: blocking losses"),
        "the refusal page must render, got:
{}",
        html
    );
    assert!(
        html.contains(&expected_hash),
        "the refusal page must name the retained hash"
    );
    assert!(
        html.contains("/import/accept"),
        "the refusal page must offer the hash-based acceptance form"
    );

    // Accept every blocking loss through the workbench form, carrying ONLY the hash and the
    // checkboxes - no artifact field.
    let mut params: Vec<(String, String)> = vec![
        ("artifactHash".to_string(), expected_hash.clone()),
        ("branch".to_string(), "main".to_string()),
        (
            "message".to_string(),
            "accept coffee-grinder losses".to_string(),
        ),
    ];
    for (index, identity) in COFFEE_IDENTITIES.iter().enumerate() {
        params.push((format!("accept_loss_{}", index), identity.to_string()));
    }
    let body = params
        .iter()
        .map(|(k, v)| format!("{}={}", k, percent_encode(v)))
        .collect::<Vec<_>>()
        .join("&");
    let accept_req = Request::builder()
        .method("POST")
        .uri("/ui/projects/coffee/import/accept")
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from(body))
        .unwrap();
    let accepted = router.clone().oneshot(accept_req).await.unwrap();
    assert_eq!(accepted.status(), StatusCode::CREATED, "{:?}", accepted);
    let html = body_text(accepted).await;
    assert!(
        html.contains("Import committed"),
        "the acceptance must complete the migration, got:
{}",
        html
    );
    assert!(
        html.contains(&expected_hash),
        "the committed page names the hash"
    );
}
