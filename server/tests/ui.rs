// SPDX-License-Identifier: AGPL-3.0-or-later
//! In-process tests for the workbench pages. They drive the router with tower oneshot and
//! assert on the HTML body text, so a view that stops rendering a section fails the suite,
//! and they prove the two guarantees the pages cannot ship without: authentication (an
//! unauthenticated request is a 401 sign-in prompt) and escaping (a hostile project name
//! cannot execute as markup).

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

use okf::types::OkfRoot;
use server::auth::{AuthConfig, Identity};
use server::store::{sqlite::SqliteStore, GateRun, Store};
use server::AppState;

fn state_with_auth(dir: &std::path::Path, auth: AuthConfig) -> AppState {
    let store = SqliteStore::open(&dir.join("mw.db")).unwrap();
    AppState {
        store: Arc::new(store),
        evidence_dir: dir.to_path_buf(),
        auth,
    }
}

fn state(dir: &std::path::Path) -> AppState {
    state_with_auth(dir, AuthConfig::Open)
}

fn get(uri: &str) -> Request<Body> {
    Request::builder().uri(uri).body(Body::empty()).unwrap()
}

fn post(uri: &str, body: serde_json::Value) -> Request<Body> {
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

/// Percent-encode one form value, so a branch name or message with a space or a
/// non-ASCII character survives the form round-trip.
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

async fn json_body(response: axum::response::Response) -> serde_json::Value {
    serde_json::from_str(&body_text(response).await).unwrap()
}

fn count(haystack: &str, needle: &str) -> usize {
    haystack.matches(needle).count()
}

fn viewer() -> Identity {
    Identity {
        subject: "viewer".to_string(),
        roles: vec!["viewer".to_string()],
        projects: vec!["*".to_string()],
        trial_id: None,
    }
}

#[tokio::test]
async fn the_project_list_and_page_render_the_coffee_corpus() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));

    let created = router
        .clone()
        .oneshot(post("/projects", serde_json::json!({ "name": "coffee" })))
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::CREATED);

    let expected: serde_json::Value =
        serde_json::from_str(&test_support::load_okf_expected()).unwrap();
    let committed = router
        .clone()
        .oneshot(post(
            "/projects/coffee/commits",
            serde_json::json!({
                "branch": "main",
                "author": "alex",
                "message": "import the exported model",
                "okf": expected
            }),
        ))
        .await
        .unwrap();
    assert_eq!(committed.status(), StatusCode::CREATED);

    let list = router.clone().oneshot(get("/ui")).await.unwrap();
    assert_eq!(list.status(), StatusCode::OK);
    let list_html = body_text(list).await;
    assert!(list_html.contains("coffee"), "the project must be listed");
    assert!(
        list_html.contains("1 branch"),
        "the branch count must render, got:\n{}",
        list_html
    );
    assert!(
        list_html.contains("import the exported model"),
        "the latest commit must render, got:\n{}",
        list_html
    );

    let page = router.oneshot(get("/ui/projects/coffee")).await.unwrap();
    assert_eq!(page.status(), StatusCode::OK);
    let page_html = body_text(page).await;
    assert!(page_html.contains("main"), "the branch must be listed");
    assert!(
        page_html.contains("import the exported model"),
        "the tip message must render"
    );
    assert!(page_html.contains("alex"), "the tip author must render");
}

#[tokio::test]
async fn a_viewer_sees_the_pages() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(SqliteStore::open(&dir.path().join("mw.db")).unwrap());
    store.create_project("coffee", None).unwrap();
    let router = server::app(AppState {
        store: store.clone(),
        evidence_dir: dir.path().to_path_buf(),
        auth: AuthConfig::fixed(viewer()),
    });

    let list = router.clone().oneshot(get("/ui")).await.unwrap();
    assert_eq!(list.status(), StatusCode::OK);
    let html = body_text(list).await;
    assert!(html.contains("coffee"), "the project must be listed");
    assert!(
        html.contains("viewer via fixed"),
        "the header must name the identity subject and the auth mechanism"
    );

    let page = router.oneshot(get("/ui/projects/coffee")).await.unwrap();
    assert_eq!(page.status(), StatusCode::OK);
    let html = body_text(page).await;
    assert!(html.contains("coffee"), "the project must be named");
    assert!(!html.contains("not signed in"), "a viewer is authenticated");
}

#[tokio::test]
async fn the_import_pages_state_the_fidelity_boundary_and_label_losses() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    router
        .clone()
        .oneshot(post("/projects", serde_json::json!({ "name": "coffee" })))
        .await
        .unwrap();

    // The form states which half of fidelity is measured and which half is the binding's word.
    let page = router
        .clone()
        .oneshot(get("/ui/projects/coffee/import"))
        .await
        .unwrap();
    assert_eq!(page.status(), StatusCode::OK);
    let html = body_text(page).await;
    assert!(
        html.contains("NOT independently measured"),
        "the import page must state the measurement boundary, got:\n{}",
        html
    );

    // A refused import renders each blocking loss with its verdict and note, so a person can
    // tell a dropped id from a dropped body that happen to name the same subject.
    let fixture = std::fs::read_to_string(format!(
        "{}/../engine/binding-xmi/fixtures/unknown-element.xmi",
        env!("CARGO_MANIFEST_DIR")
    ))
    .expect("fixture must exist");
    let response = router
        .oneshot(post_form(
            "/ui/projects/coffee/import",
            &[
                ("binding", "sysml-v1-xmi@2.4"),
                ("branch", "main"),
                ("message", "import"),
                ("artifact", &fixture),
            ],
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let html = body_text(response).await;
    assert!(
        html.contains("(unmappable)"),
        "each loss must be labelled with its verdict, got:\n{}",
        html
    );
    assert!(
        html.contains("loss-note"),
        "each loss must show its note, got:\n{}",
        html
    );
    assert!(
        html.contains("NOT independently measured"),
        "the refusal page must state the measurement boundary"
    );
}

#[tokio::test]
async fn an_unauthenticated_request_renders_a_sign_in_prompt() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state_with_auth(
        dir.path(),
        AuthConfig::static_token("the-token"),
    ));

    for uri in [
        "/ui",
        "/ui/projects/coffee",
        "/ui/projects/coffee/model",
        "/ui/projects/coffee/diagram",
        "/ui/projects/coffee/compare?from=a&to=b",
        "/ui/projects/coffee/import",
        "/ui/projects/coffee/gate",
        "/ui/projects/coffee/gate/aaaa/bbbb",
    ] {
        let response = router.clone().oneshot(get(uri)).await.unwrap();
        assert_eq!(
            response.status(),
            StatusCode::UNAUTHORIZED,
            "{} must be refused with 401",
            uri
        );
        let html = body_text(response).await;
        assert!(html.contains("<html"), "a 401 must be a page, not JSON");
        assert!(
            html.contains("Sign in required"),
            "a 401 must render a sign-in prompt"
        );
        assert!(
            !html.contains("the-token"),
            "the token must never be echoed"
        );
    }
}

#[tokio::test]
async fn a_hostile_project_name_is_escaped() {
    // Project names are validated at the API boundary, but a store can hold a name that
    // never came through that boundary (an import, an old database, a hostile colleague).
    // The page must escape it regardless, so it can never execute as markup.
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(SqliteStore::open(&dir.path().join("mw.db")).unwrap());
    store
        .create_project("<script>alert(1)</script>", None)
        .unwrap();
    let router = server::app(AppState {
        store,
        evidence_dir: dir.path().to_path_buf(),
        auth: AuthConfig::Open,
    });

    let response = router.oneshot(get("/ui")).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;
    assert!(
        html.contains("&lt;script&gt;alert(1)&lt;/script&gt;"),
        "the hostile name must appear escaped, got:\n{}",
        html
    );
    assert!(
        !html.contains("<script"),
        "a raw script tag must never appear, got:\n{}",
        html
    );
}

#[tokio::test]
async fn a_scoped_identity_only_sees_its_projects() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(SqliteStore::open(&dir.path().join("mw.db")).unwrap());
    store.create_project("coffee", None).unwrap();
    store.create_project("tea", None).unwrap();
    let router = server::app(AppState {
        store,
        evidence_dir: dir.path().to_path_buf(),
        auth: AuthConfig::fixed(Identity {
            subject: "scoped".to_string(),
            roles: vec!["viewer".to_string()],
            projects: vec!["coffee".to_string()],
            trial_id: None,
        }),
    });

    let response = router.oneshot(get("/ui")).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;
    assert!(
        html.contains("coffee"),
        "the in-scope project must be listed"
    );
    assert!(
        !html.contains("tea"),
        "the out-of-scope project must not be disclosed"
    );
}

#[tokio::test]
async fn an_unknown_project_renders_not_found() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));

    let response = router.oneshot(get("/ui/projects/nope")).await.unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let html = body_text(response).await;
    assert!(html.contains("<html"), "a 404 must be a page, not JSON");
    assert!(html.contains("Not Found"), "the page must name the status");
    assert!(
        html.contains("project nope"),
        "the page must name the missing project"
    );
}

#[tokio::test]
async fn the_model_page_renders_all_four_sections_of_the_corpus() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));

    router
        .clone()
        .oneshot(post("/projects", serde_json::json!({ "name": "coffee" })))
        .await
        .unwrap();
    let expected: serde_json::Value =
        serde_json::from_str(&test_support::load_okf_expected()).unwrap();
    let committed = router
        .clone()
        .oneshot(post(
            "/projects/coffee/commits",
            serde_json::json!({ "branch": "main", "author": "alex", "message": "import the exported model", "okf": expected }),
        ))
        .await
        .unwrap();
    assert_eq!(committed.status(), StatusCode::CREATED);

    let response = router
        .oneshot(get("/ui/projects/coffee/model"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;

    // All four sections render, in order.
    assert!(html.contains("<h2>Structure</h2>"), "structure section");
    assert!(
        html.contains("<h2>Requirements</h2>"),
        "requirements section"
    );
    assert!(
        html.contains("<h2>Traceability</h2>"),
        "traceability section"
    );
    assert!(
        html.contains("<h2>State and activity</h2>"),
        "state and activity section"
    );

    // The corpus counts: 49 structure elements, 9 signals, 25 requirements, 8 activities.
    assert_eq!(
        count(&html, "<li class=\"element\""),
        49,
        "49 structure elements"
    );
    assert_eq!(count(&html, "<li class=\"signal\""), 9, "9 signals");
    assert_eq!(
        count(&html, "<tr class=\"requirement\""),
        25,
        "25 requirements"
    );
    assert_eq!(
        count(&html, "<tr class=\"trace-row\""),
        25,
        "25 matrix rows"
    );
    assert_eq!(count(&html, "class=\"activity\""), 8, "8 activities");
    assert_eq!(count(&html, "class=\"state\""), 7, "7 states");
    assert_eq!(count(&html, "class=\"transition\""), 8, "8 transitions");

    // The engine's coverage numbers, rendered verbatim rather than recomputed. The
    // two totals are now shown as text chips (never colour alone), so the assertion
    // checks the numbers with their chips rather than one unbroken string.
    assert!(
        html.contains("<span class=\"covered\">15 covered</span>"),
        "the covered count must come from the engine"
    );
    assert!(
        html.contains("<span class=\"uncovered\">10 uncovered</span>"),
        "the uncovered count must come from the engine"
    );
    assert!(
        html.contains("20 satisfy, 3 refine, 1 verify, 0 allocate"),
        "the per-kind coverage counts must come from the engine"
    );

    // Every requirement is visibly marked with its coverage verdict in BOTH the
    // requirements table and the traceability matrix (15 covered + 10 uncovered each),
    // and the summary and overview cards repeat the two totals as chips: 15 + 15 + 1 + 1
    // covered, 10 + 10 + 1 + 1 uncovered.
    assert_eq!(
        count(&html, "class=\"uncovered\""),
        22,
        "10 uncovered marked in each table plus two summary chips"
    );
    assert_eq!(
        count(&html, "class=\"covered\""),
        32,
        "15 covered marked in each table plus two summary chips"
    );
    assert!(
        html.contains("System Level Requirements"),
        "an uncovered requirement must render"
    );
    assert!(
        html.contains("Regulatory Compliance Mark"),
        "the leaf uncovered requirement must render"
    );
    assert!(
        html.contains("Heater Initialization"),
        "a covered requirement must render"
    );

    // The two dangling edges render as explicit broken links, not silently dropped.
    assert_eq!(
        count(&html, "class=\"unresolved-edge\""),
        2,
        "2 unresolved edges"
    );
    assert!(
        html.contains("_2026x_1_12a70364_1789524431087_459748_5711"),
        "the first dangling endpoint must be shown"
    );
    assert!(
        html.contains("_2026x_1_12a70364_1789524431091_435540_5713"),
        "the second dangling endpoint must be shown"
    );

    // The page names what it is rendering.
    assert!(
        html.contains("Coffee Machine"),
        "the root block must render"
    );
    assert!(
        html.contains("import the exported model"),
        "the commit message must render"
    );
    assert!(html.contains("alex"), "the commit author must render");
}

#[tokio::test]
async fn the_model_page_resolves_branch_and_commit_and_404s_unknown() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    router
        .clone()
        .oneshot(post("/projects", serde_json::json!({ "name": "coffee" })))
        .await
        .unwrap();
    let expected: serde_json::Value =
        serde_json::from_str(&test_support::load_okf_expected()).unwrap();
    let committed = router
        .clone()
        .oneshot(post(
            "/projects/coffee/commits",
            serde_json::json!({ "branch": "main", "author": "alex", "message": "import the exported model", "okf": expected }),
        ))
        .await
        .unwrap();
    let hash = json_body(committed).await["hash"]
        .as_str()
        .unwrap()
        .to_string();

    for uri in [
        "/ui/projects/coffee/model?branch=main".to_string(),
        format!("/ui/projects/coffee/model?commit={}", hash),
    ] {
        let response = router.clone().oneshot(get(&uri)).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{} must render", uri);
        let html = body_text(response).await;
        assert!(
            html.contains("<h2>Structure</h2>"),
            "{} must render structure",
            uri
        );
    }

    let bad_branch = router
        .clone()
        .oneshot(get("/ui/projects/coffee/model?branch=nope"))
        .await
        .unwrap();
    assert_eq!(bad_branch.status(), StatusCode::NOT_FOUND);

    let bad_commit = router
        .clone()
        .oneshot(get("/ui/projects/coffee/model?commit=deadbeef"))
        .await
        .unwrap();
    assert_eq!(bad_commit.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn add_element_and_edit_links_carry_the_viewed_branch_not_main() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    router
        .clone()
        .oneshot(post("/projects", serde_json::json!({ "name": "coffee" })))
        .await
        .unwrap();
    let expected: serde_json::Value =
        serde_json::from_str(&test_support::load_okf_expected()).unwrap();
    let tip = commit_okf(&router, "main", "seed", expected).await;

    // Create a version from main's tip: the new branch points at the SAME commit, whose
    // stored branch is still "main". This is exactly the audit's branch-drop scenario.
    let branched = router
        .clone()
        .oneshot(post(
            "/projects/coffee/branches",
            serde_json::json!({ "name": "feature", "from": tip }),
        ))
        .await
        .unwrap();
    assert_eq!(branched.status(), StatusCode::CREATED);

    let page = router
        .oneshot(get("/ui/projects/coffee/overview?branch=feature"))
        .await
        .unwrap();
    assert_eq!(page.status(), StatusCode::OK);
    let html = body_text(page).await;

    // The add-element link must carry the VIEWED branch, not the commit's own branch.
    assert!(
        html.contains("/element/new?branch=feature"),
        "the add-element link must carry the viewed branch feature"
    );
    assert!(
        !html.contains("/element/new?branch=main"),
        "the add-element link must NOT carry main"
    );
    // The edit links carry the viewed branch too (and never main).
    assert!(
        html.contains("/edit/") && html.contains("?branch=feature"),
        "the edit links must carry the viewed branch feature"
    );
}

#[tokio::test]
async fn a_scoped_identity_cannot_see_another_projects_model() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(SqliteStore::open(&dir.path().join("mw.db")).unwrap());
    store.create_project("coffee", None).unwrap();
    store.create_project("tea", None).unwrap();
    let router = server::app(AppState {
        store,
        evidence_dir: dir.path().to_path_buf(),
        auth: AuthConfig::fixed(Identity {
            subject: "scoped".to_string(),
            roles: vec!["viewer".to_string()],
            projects: vec!["coffee".to_string()],
            trial_id: None,
        }),
    });

    let response = router.oneshot(get("/ui/projects/tea/model")).await.unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let html = body_text(response).await;
    assert!(html.contains("<html"), "a 403 must be a page, not JSON");
    assert!(
        html.contains("project not in scope"),
        "the page must name the scope refusal, got:\n{}",
        html
    );
}

/// A minimal two-element model, in the same shape the merge API tests use: one block, one
/// requirement, one Satisfy edge. The block's name is the only thing a branch changes.
fn merge_model(block_name: &str) -> serde_json::Value {
    serde_json::json!({
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
            "id": "b1", "name": block_name, "kind": "block",
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

/// Commit a document onto a branch and return its commit hash.
async fn commit_okf(
    router: &axum::Router,
    branch: &str,
    message: &str,
    okf: serde_json::Value,
) -> String {
    let response = router
        .clone()
        .oneshot(post(
            "/projects/coffee/commits",
            serde_json::json!({ "branch": branch, "author": "alex", "message": message, "okf": okf }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED, "commit {}", message);
    json_body(response).await["hash"]
        .as_str()
        .unwrap()
        .to_string()
}

#[tokio::test]
async fn the_compare_page_shows_the_engine_diff_and_gate_verdict() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));

    router
        .clone()
        .oneshot(post("/projects", serde_json::json!({ "name": "coffee" })))
        .await
        .unwrap();

    let expected: serde_json::Value =
        serde_json::from_str(&test_support::load_okf_expected()).unwrap();
    let broken: serde_json::Value = serde_json::from_str(&test_support::load_okf_broken()).unwrap();

    let imported = commit_okf(&router, "main", "import the exported model", expected).await;
    let branched = router
        .clone()
        .oneshot(post(
            "/projects/coffee/branches",
            serde_json::json!({ "name": "corrupted", "from": imported }),
        ))
        .await
        .unwrap();
    assert_eq!(branched.status(), StatusCode::CREATED);
    let corrupted = commit_okf(&router, "corrupted", "drop a requirement", broken).await;
    assert_ne!(imported, corrupted);

    let uri = format!(
        "/ui/projects/coffee/compare?from={}&to={}",
        imported, corrupted
    );
    let response = router.clone().oneshot(get(&uri)).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;

    // The diff is the engine's: the missing requirement appears with the exact key the
    // gate's roundtrip reports, and the isolated node appears with the exact id the gate's
    // integration reports.
    assert!(
        html.contains("requirements:_2026x_1_12a70364_1789522470210_613186_5619"),
        "the missing requirement must render exactly as the gate reports it, got:\n{}",
        html
    );
    assert!(
        html.contains("_2026x_1_12a70364_1789602694801_483599_6108"),
        "the isolated node must render exactly as the gate reports it, got:\n{}",
        html
    );

    // The gate verdict and its evidence path.
    assert!(
        html.contains("<h2>Diff</h2>"),
        "the diff section must render"
    );
    assert!(
        html.contains("<h2>Gate</h2>"),
        "the gate section must render"
    );
    assert!(
        html.contains(">failed<"),
        "the failed verdict must render, got:\n{}",
        html
    );
    // The page must NOT name an evidence file, because a comparison writes one.
    //
    // This assertion previously required the evidence PATH here, on the assumption that the
    // compare page produced a record. It does not: gate::run computes a verdict and writes
    // nothing, and only the gate endpoint persists a run and its evidence. A page that named
    // a file it never wrote would tell a reviewer the record lives somewhere it does not -
    // and this platform exists to avoid exactly that kind of false assurance.
    assert!(
        !html.contains(&format!("server-{}-{}.json", imported, corrupted)),
        "a comparison must not name an evidence file it never writes, got:\n{}",
        html
    );
    assert!(
        html.contains("not recorded"),
        "the page must say plainly that a comparison leaves no record, got:\n{}",
        html
    );

    // The branch-name form of the same pair resolves to the same commit pair.
    let by_branch = router
        .oneshot(get("/ui/projects/coffee/compare?from=main&to=corrupted"))
        .await
        .unwrap();
    assert_eq!(by_branch.status(), StatusCode::OK);
    let branch_html = body_text(by_branch).await;
    assert!(
        branch_html.contains(">failed<"),
        "comparing by branch must resolve to the same failing pair"
    );
}

#[tokio::test]
async fn a_conflicting_merge_renders_base_ours_and_theirs() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));

    router
        .clone()
        .oneshot(post("/projects", serde_json::json!({ "name": "coffee" })))
        .await
        .unwrap();

    let base = commit_okf(&router, "main", "base", merge_model("Block")).await;
    let branched = router
        .clone()
        .oneshot(post(
            "/projects/coffee/branches",
            serde_json::json!({ "name": "feature", "from": base }),
        ))
        .await
        .unwrap();
    assert_eq!(branched.status(), StatusCode::CREATED);

    commit_okf(&router, "feature", "theirs", merge_model("Theirs")).await;
    commit_okf(&router, "main", "ours", merge_model("Ours")).await;

    let response = router
        .oneshot(post_form(
            "/ui/projects/coffee/merge",
            &[
                ("branch", "main"),
                ("other", "feature"),
                ("author", "alex"),
                ("message", "merge"),
            ],
        ))
        .await
        .unwrap();

    // A 409 is information, not a failure: the conflict is a PAGE, not an error page.
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let html = body_text(response).await;
    assert!(
        html.contains("Merge conflict"),
        "the conflict must render as a page, got:\n{}",
        html
    );
    assert!(
        html.contains("structure:&quot;b1&quot;"),
        "the conflict subject must render, got:\n{}",
        html
    );
    assert!(
        html.contains("bothModified"),
        "the conflict kind must render, got:\n{}",
        html
    );

    // Base, ours and theirs render side by side.
    assert_eq!(
        count(&html, "class=\"conflict-side\""),
        3,
        "three columns must render side by side, got:\n{}",
        html
    );
    assert!(
        html.contains("&quot;name&quot;:&quot;Block&quot;"),
        "the base value must render, got:\n{}",
        html
    );
    assert!(
        html.contains("&quot;name&quot;:&quot;Ours&quot;"),
        "the ours value must render, got:\n{}",
        html
    );
    assert!(
        html.contains("&quot;name&quot;:&quot;Theirs&quot;"),
        "the theirs value must render, got:\n{}",
        html
    );

    // A resolution form is present, prefilled with the two branches.
    assert!(
        html.contains("name=\"branch\""),
        "the resolution form must render"
    );
    assert!(
        html.contains("value=\"main\""),
        "the branch must be prefilled"
    );
}

#[tokio::test]
async fn a_clean_merge_through_the_form_succeeds() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));

    router
        .clone()
        .oneshot(post("/projects", serde_json::json!({ "name": "coffee" })))
        .await
        .unwrap();

    let base = commit_okf(&router, "main", "base", merge_model("Block")).await;
    let branched = router
        .clone()
        .oneshot(post(
            "/projects/coffee/branches",
            serde_json::json!({ "name": "feature", "from": base }),
        ))
        .await
        .unwrap();
    assert_eq!(branched.status(), StatusCode::CREATED);
    commit_okf(&router, "feature", "rename", merge_model("Renamed")).await;

    let response = router
        .clone()
        .oneshot(post_form(
            "/ui/projects/coffee/merge",
            &[
                ("branch", "main"),
                ("other", "feature"),
                ("author", "alex"),
                ("message", "merge feature"),
            ],
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);
    let html = body_text(response).await;
    assert!(
        html.contains("Merge complete"),
        "the success page must render"
    );
    assert!(
        html.contains("feature"),
        "the merged-from branch must render"
    );
    assert!(html.contains("main"), "the merged-into branch must render");

    // And the merge really happened: main's history now contains a two-parent commit.
    let history = router
        .oneshot(get("/projects/coffee/commits?branch=main"))
        .await
        .unwrap();
    let history = json_body(history).await;
    let has_merge = history.as_array().unwrap().iter().any(|commit| {
        commit["parents"]
            .as_array()
            .map(|parents| parents.len())
            .unwrap_or(0)
            == 2
    });
    assert!(has_merge, "the merge must write a two-parent commit");
}

#[tokio::test]
async fn an_unauthenticated_merge_is_a_sign_in_prompt() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state_with_auth(
        dir.path(),
        AuthConfig::static_token("the-token"),
    ));

    let response = router
        .oneshot(post_form(
            "/ui/projects/coffee/merge",
            &[
                ("branch", "main"),
                ("other", "feature"),
                ("message", "merge"),
            ],
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let html = body_text(response).await;
    assert!(html.contains("<html"), "a 401 must be a page, not JSON");
    assert!(
        html.contains("Sign in required"),
        "a 401 must be a sign-in prompt"
    );
    assert!(
        !html.contains("the-token"),
        "the token must never be echoed"
    );
}

#[tokio::test]
async fn a_viewer_cannot_merge() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(SqliteStore::open(&dir.path().join("mw.db")).unwrap());
    store.create_project("coffee", None).unwrap();
    let router = server::app(AppState {
        store,
        evidence_dir: dir.path().to_path_buf(),
        auth: AuthConfig::fixed(viewer()),
    });

    let response = router
        .oneshot(post_form(
            "/ui/projects/coffee/merge",
            &[
                ("branch", "main"),
                ("other", "feature"),
                ("message", "merge"),
            ],
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let html = body_text(response).await;
    assert!(html.contains("<html"), "a 403 must be a page, not JSON");
    assert!(
        html.contains("write permission required"),
        "the page must name the permission refusal, got:\n{}",
        html
    );
}

#[tokio::test]
async fn hostile_content_in_a_model_is_escaped_on_the_model_page() {
    // The project list is not where a supplier's words arrive: requirement text and element
    // documentation are. The escaping is uniform because maud escapes every splice, but a
    // test here is what keeps that true if somebody reaches for raw markup later.
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));

    let mut okf = test_support::load_okf_expected();
    let mut value: serde_json::Value = serde_json::from_str(&okf).unwrap();
    value["structure"][0]["documentation"] = serde_json::json!("<script>alert('docs')</script>");
    value["requirements"][0]["reqText"] = serde_json::json!("<script>alert('req')</script>");
    okf = value.to_string();

    let committed = router
        .clone()
        .oneshot(post("/projects", serde_json::json!({ "name": "coffee" })))
        .await
        .unwrap();
    assert_eq!(committed.status(), StatusCode::CREATED);
    let committed = router
        .clone()
        .oneshot(post(
            "/projects/coffee/commits",
            serde_json::json!({ "branch": "main", "author": "alex", "message": "hostile", "okf": serde_json::from_str::<serde_json::Value>(&okf).unwrap() }),
        ))
        .await
        .unwrap();
    assert_eq!(committed.status(), StatusCode::CREATED);

    let page = router
        .oneshot(get("/ui/projects/coffee/model"))
        .await
        .unwrap();
    assert_eq!(page.status(), StatusCode::OK);
    let html = body_text(page).await;
    assert!(
        html.contains("&lt;script&gt;"),
        "the hostile content must appear escaped"
    );
    // The page's own enhancement asset is the one legitimate script tag; hostile model
    // content must never add a raw one of its own.
    assert_eq!(
        count(&html, "<script"),
        1,
        "exactly the enhancement asset may be a script tag, got:\n{}",
        html
    );
}
/// The author role: can read and write, so it can both view and submit the edit form.
fn author() -> Identity {
    Identity {
        subject: "alice".to_string(),
        roles: vec!["author".to_string()],
        projects: vec!["*".to_string()],
        trial_id: None,
    }
}

/// Seed a project and a model directly through the store, bypassing the JSON commit
/// handler's author resolution so a test can fix a specific identity afterwards.
fn seed_model_directly(store: &dyn Store, okf: serde_json::Value) {
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

/// The merge model with two attributes on the block, so an edit proves the form round-trips
/// attributes rather than dropping them.
fn block_with_attributes() -> serde_json::Value {
    let mut model = merge_model("Block");
    model["structure"][0]["attributes"] = serde_json::json!([
        { "name": "inlet", "type": "Water Inlet", "aggregation": "none", "default": "" },
        { "name": "outlet", "type": "Water Outlet", "aggregation": "none", "default": "" },
    ]);
    model
}

#[tokio::test]
async fn editing_a_block_documentation_commits_with_the_identity_author() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(SqliteStore::open(&dir.path().join("mw.db")).unwrap());
    seed_model_directly(store.as_ref(), block_with_attributes());
    let router = server::app(AppState {
        store: store.clone(),
        evidence_dir: dir.path().to_path_buf(),
        auth: AuthConfig::fixed(author()),
    });

    let form = router
        .clone()
        .oneshot(get("/ui/projects/coffee/edit/b1"))
        .await
        .unwrap();
    assert_eq!(form.status(), StatusCode::OK);
    let form_html = body_text(form).await;
    assert!(
        form_html.contains("name=\"documentation\""),
        "the form must render the documentation field"
    );
    assert!(
        form_html.contains("name=\"attr_name_0\""),
        "the form must render the block's attributes"
    );

    // The author field the form never asks for is ignored; the commit records the identity.
    let response = router
        .clone()
        .oneshot(post_form(
            "/ui/projects/coffee/edit/b1",
            &[
                ("branch", "main"),
                ("message", "document the block"),
                ("id", "b1"),
                ("name", "Block"),
                ("documentation", "Brews coffee on demand."),
                ("stereotypes", "Block"),
                ("attr_count", "2"),
                ("attr_name_0", "inlet"),
                ("attr_type_0", "Water Inlet"),
                ("attr_aggregation_0", "none"),
                ("attr_default_0", ""),
                ("attr_name_1", "outlet"),
                ("attr_type_1", "Water Outlet"),
                ("attr_aggregation_1", "none"),
                ("attr_default_1", ""),
                // A SPARSE, absurdly high index. An earlier attempt to bound the attribute
                // loop took the highest index present and iterated from zero to there, so
                // this single field would have made the server do a billion iterations of
                // formatting and lookups before answering. With the bound taken from the
                // indexes that ACTUALLY EXIST, this costs exactly one iteration. The test
                // has teeth by construction: under the old code it does not fail, it hangs.
                ("attr_name_1000000000", "Legacy"),
                ("attr_type_1000000000", "String"),
                ("attr_aggregation_1000000000", "none"),
                ("attr_default_1000000000", ""),
                ("author", "evil"),
            ],
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let html = body_text(response).await;
    assert!(
        html.contains("Edit committed"),
        "the success page must render, got:\n{}",
        html
    );

    let commits = router
        .clone()
        .oneshot(get("/projects/coffee/commits?branch=main"))
        .await
        .unwrap();
    let commits = json_body(commits).await;
    let edited = commits
        .as_array()
        .unwrap()
        .iter()
        .find(|commit| commit["message"] == "document the block")
        .expect("the edit commit must exist");
    assert_eq!(
        edited["author"].as_str().unwrap(),
        "alice",
        "the author must be the identity, never a browser field"
    );

    let hash = edited["hash"].as_str().unwrap();
    let doc = router
        .clone()
        .oneshot(get(&format!("/projects/coffee/commits/{}", hash)))
        .await
        .unwrap();
    let doc = json_body(doc).await;
    assert_eq!(
        doc["structure"][0]["documentation"].as_str().unwrap(),
        "Brews coffee on demand.",
        "the documentation must change"
    );
    // Three, not two: the two real attributes plus the one submitted at the sparse index
    // 1000000000. The point of that third field is the WORK it must not cause - see the
    // comment where it is submitted - and this count is what proves it was parsed rather
    // than skipped.
    assert_eq!(
        doc["structure"][0]["attributes"].as_array().unwrap().len(),
        3,
        "the attributes must round-trip through the form"
    );

    // The request-duration lease is released when the edit finishes.
    assert!(
        store
            .locks("coffee", server::store::now_seconds())
            .unwrap()
            .is_empty(),
        "the element must not be left locked"
    );
}

#[tokio::test]
async fn an_element_held_by_another_holder_is_refused_and_names_the_holder() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(SqliteStore::open(&dir.path().join("mw.db")).unwrap());
    seed_model_directly(store.as_ref(), merge_model("Block"));
    let router = server::app(AppState {
        store: store.clone(),
        evidence_dir: dir.path().to_path_buf(),
        auth: AuthConfig::Open,
    });

    // Another holder takes a lease on the element through the JSON locks endpoint.
    let acquired = router
        .clone()
        .oneshot(post(
            "/projects/coffee/locks",
            serde_json::json!({
                "branch": "main",
                "elements": ["b1"],
                "holder": "bob",
                "ttlSeconds": 300
            }),
        ))
        .await
        .unwrap();
    assert_eq!(acquired.status(), StatusCode::CREATED);

    // The form shows the holder and refuses to submit.
    let form = router
        .clone()
        .oneshot(get("/ui/projects/coffee/edit/b1"))
        .await
        .unwrap();
    assert_eq!(form.status(), StatusCode::OK);
    let form_html = body_text(form).await;
    assert!(
        form_html.contains("Held by another holder"),
        "the form must say who holds the element, got:\n{}",
        form_html
    );
    assert!(
        form_html.contains("bob"),
        "the holder must be named, got:\n{}",
        form_html
    );
    assert!(
        form_html.contains("<button type=\"submit\" disabled"),
        "the submit button must be disabled while another holder has it, got:\n{}",
        form_html
    );

    // Submitting is refused, the holder is named, and nothing is stored.
    let response = router
        .clone()
        .oneshot(post_form(
            "/ui/projects/coffee/edit/b1",
            &[
                ("branch", "main"),
                ("message", "steal the block"),
                ("id", "b1"),
                ("name", "Block"),
                ("documentation", "overwritten"),
                ("stereotypes", "Block"),
            ],
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let html = body_text(response).await;
    assert!(
        html.contains("bob"),
        "the refusal must name the holder, got:\n{}",
        html
    );

    let commits = router
        .clone()
        .oneshot(get("/projects/coffee/commits?branch=main"))
        .await
        .unwrap();
    let commits = json_body(commits).await;
    assert_eq!(
        commits.as_array().unwrap().len(),
        1,
        "the refused edit must not have committed"
    );
}

#[tokio::test]
async fn an_invalid_edit_renders_the_validator_errors_and_stores_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(SqliteStore::open(&dir.path().join("mw.db")).unwrap());
    seed_model_directly(store.as_ref(), merge_model("Block"));
    let router = server::app(AppState {
        store: store.clone(),
        evidence_dir: dir.path().to_path_buf(),
        auth: AuthConfig::Open,
    });

    // Emptying the id is an explicit edit the validator refuses, so the errors render next
    // to the form rather than crashing with a 500.
    let response = router
        .clone()
        .oneshot(post_form(
            "/ui/projects/coffee/edit/b1",
            &[
                ("branch", "main"),
                ("message", "break it"),
                ("id", ""),
                ("name", "Block"),
                ("documentation", ""),
                ("stereotypes", "Block"),
            ],
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let html = body_text(response).await;
    assert!(
        html.contains("empty element id"),
        "the validator's error must render, got:\n{}",
        html
    );
    assert!(
        !html.contains("Edit committed"),
        "an invalid edit must not render success"
    );

    let commits = router
        .clone()
        .oneshot(get("/projects/coffee/commits?branch=main"))
        .await
        .unwrap();
    let commits = json_body(commits).await;
    assert_eq!(
        commits.as_array().unwrap().len(),
        1,
        "the invalid edit must not have committed"
    );
}

#[tokio::test]
async fn an_unauthenticated_edit_is_a_sign_in_prompt() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state_with_auth(
        dir.path(),
        AuthConfig::static_token("the-token"),
    ));

    for uri in ["/ui/projects/coffee/edit/b1"] {
        let response = router.clone().oneshot(get(uri)).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let html = body_text(response).await;
        assert!(html.contains("Sign in required"));
        assert!(
            !html.contains("the-token"),
            "the token must never be echoed"
        );
    }

    let response = router
        .oneshot(post_form(
            "/ui/projects/coffee/edit/b1",
            &[
                ("branch", "main"),
                ("message", "steal"),
                ("id", "b1"),
                ("name", "Block"),
            ],
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let html = body_text(response).await;
    assert!(html.contains("Sign in required"));
}

#[tokio::test]
async fn a_viewer_cannot_edit() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(SqliteStore::open(&dir.path().join("mw.db")).unwrap());
    seed_model_directly(store.as_ref(), merge_model("Block"));
    let router = server::app(AppState {
        store,
        evidence_dir: dir.path().to_path_buf(),
        auth: AuthConfig::fixed(viewer()),
    });

    let response = router
        .oneshot(post_form(
            "/ui/projects/coffee/edit/b1",
            &[
                ("branch", "main"),
                ("message", "steal"),
                ("id", "b1"),
                ("name", "Block"),
            ],
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let html = body_text(response).await;
    assert!(html.contains("<html"), "a 403 must be a page, not JSON");
    assert!(
        html.contains("write permission required"),
        "the page must name the permission refusal, got:\n{}",
        html
    );
}

#[tokio::test]
async fn the_edit_form_renders_the_corpus_block_with_its_attributes() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    router
        .clone()
        .oneshot(post("/projects", serde_json::json!({ "name": "coffee" })))
        .await
        .unwrap();
    let expected: serde_json::Value =
        serde_json::from_str(&test_support::load_okf_expected()).unwrap();
    let block_id = expected["structure"][0]["id"].as_str().unwrap().to_string();
    commit_okf(&router, "main", "import the exported model", expected).await;

    let form = router
        .oneshot(get(&format!("/ui/projects/coffee/edit/{}", block_id)))
        .await
        .unwrap();
    assert_eq!(form.status(), StatusCode::OK);
    let html = body_text(form).await;
    assert!(
        html.contains("Coffee Machine"),
        "the root block's name must render, got:\n{}",
        html
    );
    assert!(
        html.contains("water System"),
        "the block's first attribute must render, got:\n{}",
        html
    );
    assert!(
        html.contains("name=\"attr_name_0\""),
        "attribute rows must be indexed"
    );
}

/// A tiny document with one block, optionally including a second block whose name the
/// caller chooses, so two branches can add the SAME element differently.
fn small_model(second: Option<&str>) -> serde_json::Value {
    let mut structure = vec![serde_json::json!({
        "id": "b1", "name": "Block", "kind": "block",
        "stereotypes": ["Block"], "attributes": [], "documentation": ""
    })];
    let mut nodes = vec![serde_json::json!({ "id": "b1", "kind": "block", "name": "Block" })];
    if let Some(name) = second {
        structure.push(serde_json::json!({
            "id": "b9", "name": name, "kind": "block",
            "stereotypes": ["Block"], "attributes": [], "documentation": ""
        }));
        nodes.push(serde_json::json!({ "id": "b9", "kind": "block", "name": name }));
    }
    serde_json::json!({
        "project": "coffee",
        "exportedAt": "2026-09-17T00:00:00Z",
        "summary": {},
        "stateMachine": { "name": "sm", "regions": [] },
        "structure": structure,
        "requirements": [],
        "graph": { "nodes": nodes, "edges": [] }
    })
}

#[tokio::test]
async fn a_conflict_with_no_base_side_says_absent_rather_than_showing_nothing() {
    // An element added on BOTH branches has no base version at all. Rendering an empty
    // cell there would read as "no change" when the truth is that two people invented the
    // same element differently - which is exactly the reading a reviewer must not make.
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));

    let created = router
        .clone()
        .oneshot(post("/projects", serde_json::json!({ "name": "coffee" })))
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::CREATED);

    let base = router
        .clone()
        .oneshot(post(
            "/projects/coffee/commits",
            serde_json::json!({ "branch": "main", "author": "alex", "message": "base", "okf": small_model(None) }),
        ))
        .await
        .unwrap();
    let base_hash = json_body(base).await["hash"].as_str().unwrap().to_string();

    let branched = router
        .clone()
        .oneshot(post(
            "/projects/coffee/branches",
            serde_json::json!({ "name": "feature", "from": base_hash }),
        ))
        .await
        .unwrap();
    assert_eq!(branched.status(), StatusCode::CREATED);

    // The same NEW element, added on both branches with different content.
    for (branch, name) in [("main", "Ours"), ("feature", "Theirs")] {
        let committed = router
            .clone()
            .oneshot(post(
                "/projects/coffee/commits",
                serde_json::json!({ "branch": branch, "author": "alex", "message": "add", "okf": small_model(Some(name)) }),
            ))
            .await
            .unwrap();
        assert_eq!(committed.status(), StatusCode::CREATED, "{}", branch);
    }

    let response = router
        .oneshot(post_form(
            "/ui/projects/coffee/merge",
            &[
                ("branch", "main"),
                ("other", "feature"),
                ("message", "merge"),
                ("author", "alex"),
            ],
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let html = body_text(response).await;
    assert!(
        html.contains("Merge conflict"),
        "a conflict must be a page, not an error"
    );
    assert!(
        html.contains("(absent)"),
        "a missing base side must be named, not left blank"
    );
}

// ---------------------------------------------------------------------------
// Task 5: the gate, rendered where a reviewer looks.

/// The reviewer role: reads gate runs and their evidence, so it may open the detail.
fn reviewer() -> Identity {
    Identity {
        subject: "reviewer".to_string(),
        roles: vec!["reviewer".to_string()],
        projects: vec!["*".to_string()],
        trial_id: None,
    }
}

#[tokio::test]
async fn the_gate_pages_render_the_self_pass_and_the_corrupted_pair() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));

    router
        .clone()
        .oneshot(post("/projects", serde_json::json!({ "name": "coffee" })))
        .await
        .unwrap();

    let expected: serde_json::Value =
        serde_json::from_str(&test_support::load_okf_expected()).unwrap();
    let broken: serde_json::Value = serde_json::from_str(&test_support::load_okf_broken()).unwrap();

    let imported = commit_okf(&router, "main", "import the exported model", expected).await;
    let branched = router
        .clone()
        .oneshot(post(
            "/projects/coffee/branches",
            serde_json::json!({ "name": "corrupted", "from": imported }),
        ))
        .await
        .unwrap();
    assert_eq!(branched.status(), StatusCode::CREATED);
    let corrupted = commit_okf(&router, "corrupted", "drop a requirement", broken).await;

    // Two recorded runs, in order: the self-pass first, then the corrupted pair.
    for (reference, candidate) in [
        (imported.clone(), imported.clone()),
        (imported.clone(), corrupted.clone()),
    ] {
        let response = router
            .clone()
            .oneshot(post(
                "/projects/coffee/gate",
                serde_json::json!({ "reference": reference, "candidate": candidate }),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    // The list: both runs, newest first, each with its verdict, hashes and evidence file name.
    let list = router
        .clone()
        .oneshot(get("/ui/projects/coffee/gate"))
        .await
        .unwrap();
    assert_eq!(list.status(), StatusCode::OK);
    let list_html = body_text(list).await;
    assert_eq!(
        count(&list_html, "class=\"gate-run\""),
        2,
        "two runs must be listed, got:\n{}",
        list_html
    );
    assert!(
        list_html.contains(">failed<"),
        "the failed verdict must render in the list"
    );
    assert!(
        list_html.contains(">passed<"),
        "the passed verdict must render in the list"
    );
    let failed_at = list_html.find(">failed<").unwrap();
    let passed_at = list_html.find(">passed<").unwrap();
    assert!(
        failed_at < passed_at,
        "the most recent run (the corrupted pair) must be listed first"
    );
    assert!(
        list_html.contains(&format!("server-{}-{}.json", imported, corrupted)),
        "the evidence file name must carry both full hashes"
    );

    // The self-pass detail renders green with its coverage counts.
    let self_pass = router
        .clone()
        .oneshot(get(&format!(
            "/ui/projects/coffee/gate/{}/{}",
            imported, imported
        )))
        .await
        .unwrap();
    assert_eq!(self_pass.status(), StatusCode::OK);
    let self_html = body_text(self_pass).await;
    assert!(
        self_html.contains(">passed<"),
        "the self-pass must render passed, got:\n{}",
        self_html
    );
    assert!(
        self_html.contains("25 requirements: 15 covered, 10 uncovered"),
        "the coverage counts must come from the evidence, got:\n{}",
        self_html
    );
    assert!(
        self_html.contains("20 satisfy, 3 refine, 1 verify, 0 allocate"),
        "the per-kind coverage counts must render"
    );
    assert!(
        self_html.contains("1 connected component"),
        "the integration section must render"
    );

    // The corrupted pair renders red, naming the missing element and the isolated node.
    let corrupted_detail = router
        .clone()
        .oneshot(get(&format!(
            "/ui/projects/coffee/gate/{}/{}",
            imported, corrupted
        )))
        .await
        .unwrap();
    assert_eq!(corrupted_detail.status(), StatusCode::OK);
    let corrupted_html = body_text(corrupted_detail).await;
    assert!(
        corrupted_html.contains(">failed<"),
        "the corrupted pair must render failed, got:\n{}",
        corrupted_html
    );
    assert!(
        corrupted_html.contains("requirements:_2026x_1_12a70364_1789522470210_613186_5619"),
        "the missing element must be named exactly as the gate reports it, got:\n{}",
        corrupted_html
    );
    assert!(
        corrupted_html.contains("_2026x_1_12a70364_1789602694801_483599_6108"),
        "the isolated node must be named exactly as the gate reports it"
    );
    // The evidence record itself renders, not just the summary above it.
    assert!(
        corrupted_html.contains("gateVersion"),
        "the evidence record must render in full"
    );
}

#[tokio::test]
async fn the_gate_pages_enforce_the_api_permissions() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(SqliteStore::open(&dir.path().join("mw.db")).unwrap());
    store.create_project("coffee", None).unwrap();
    store
        .record_gate_run(
            &GateRun {
                project: "coffee".to_string(),
                branch: "main".to_string(),
                reference_hash: "aaaa".to_string(),
                candidate_hash: "bbbb".to_string(),
                passed: true,
                evidence: "{}".to_string(),
                created_at: "1".to_string(),
            },
            None,
        )
        .unwrap();

    let app = |identity: Identity| {
        server::app(AppState {
            store: store.clone(),
            evidence_dir: dir.path().to_path_buf(),
            auth: AuthConfig::fixed(identity),
        })
    };

    // A viewer holds neither Write nor Review: the list is a refusal, never an empty page.
    let viewer_router = app(viewer());
    let list = viewer_router
        .clone()
        .oneshot(get("/ui/projects/coffee/gate"))
        .await
        .unwrap();
    assert_eq!(list.status(), StatusCode::FORBIDDEN);
    let html = body_text(list).await;
    assert!(
        html.contains("write or review permission required"),
        "the refusal must name the permission, got:\n{}",
        html
    );

    // An author may RUN a gate (Write), so it may list AND open a run.
    //
    // This assertion used to require Review for the detail, on the premise that the evidence
    // record is a reviewer's artifact. That premise was wrong: the JSON list endpoint already
    // returns the FULL evidence - failure names, isolated nodes, coverage - to any
    // Write-or-Review caller, so refusing an author the same record here would hide nothing
    // and would only teach people that the workbench is the less reliable way to read a run.
    let author_router = app(author());
    let list = author_router
        .clone()
        .oneshot(get("/ui/projects/coffee/gate"))
        .await
        .unwrap();
    assert_eq!(list.status(), StatusCode::OK);
    let detail = author_router
        .oneshot(get("/ui/projects/coffee/gate/aaaa/bbbb"))
        .await
        .unwrap();
    assert_ne!(
        detail.status(),
        StatusCode::FORBIDDEN,
        "an author may read the same evidence the JSON endpoint returns to it"
    );

    // A reviewer sees both.
    let reviewer_router = app(reviewer());
    let list = reviewer_router
        .clone()
        .oneshot(get("/ui/projects/coffee/gate"))
        .await
        .unwrap();
    assert_eq!(list.status(), StatusCode::OK);
    let detail = reviewer_router
        .oneshot(get("/ui/projects/coffee/gate/aaaa/bbbb"))
        .await
        .unwrap();
    assert_eq!(detail.status(), StatusCode::OK);
}

#[tokio::test]
async fn an_unknown_gate_run_or_project_renders_not_found() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    router
        .clone()
        .oneshot(post("/projects", serde_json::json!({ "name": "coffee" })))
        .await
        .unwrap();

    let unknown_run = router
        .clone()
        .oneshot(get("/ui/projects/coffee/gate/deadbeef/cafebabe"))
        .await
        .unwrap();
    assert_eq!(unknown_run.status(), StatusCode::NOT_FOUND);
    let html = body_text(unknown_run).await;
    assert!(html.contains("Not Found"));

    let unknown_project = router.oneshot(get("/ui/projects/nope/gate")).await.unwrap();
    assert_eq!(unknown_project.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn hostile_content_in_the_evidence_record_is_escaped() {
    // The evidence record carries strings the gate read out of an untrusted model - a
    // requirement id, an element key. Seeding a hostile one directly through the store
    // proves the page escapes it rather than trusting it.
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(SqliteStore::open(&dir.path().join("mw.db")).unwrap());
    store.create_project("coffee", None).unwrap();
    store
        .record_gate_run(
            &GateRun {
                project: "coffee".to_string(),
                branch: "main".to_string(),
                reference_hash: "aaaa".to_string(),
                candidate_hash: "bbbb".to_string(),
                passed: false,
                evidence: r#"{"roundtrip":{"equal":false,"missingElements":["requirements:<script>alert(1)</script>"]}}"#.to_string(),
                created_at: "1".to_string(),
            },
            None,
        )
        .unwrap();
    let router = server::app(AppState {
        store,
        evidence_dir: dir.path().to_path_buf(),
        auth: AuthConfig::Open,
    });

    let response = router
        .oneshot(get("/ui/projects/coffee/gate/aaaa/bbbb"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;
    assert!(
        html.contains("&lt;script&gt;"),
        "the hostile evidence content must appear escaped, got:\n{}",
        html
    );
    assert!(
        !html.contains("<script"),
        "a raw script tag must never reach the gate page"
    );
}

// ---------------------------------------------------------------------------
// Task 6: the diagram view.

/// Extract the inline SVG from a rendered page, so assertions target the diagram itself
/// rather than the surrounding shell (which legitimately contains its own navigation links).
fn extract_svg(html: &str) -> &str {
    let start = html.find("<svg").expect("an <svg> must be present");
    let close = html[start..]
        .find("</svg>")
        .expect("</svg> must be present");
    &html[start..start + close + "</svg>".len()]
}

#[tokio::test]
async fn the_diagram_renders_the_corpus_deterministically_and_completely() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));

    router
        .clone()
        .oneshot(post("/projects", serde_json::json!({ "name": "coffee" })))
        .await
        .unwrap();
    let expected: serde_json::Value =
        serde_json::from_str(&test_support::load_okf_expected()).unwrap();
    let committed = router
        .clone()
        .oneshot(post(
            "/projects/coffee/commits",
            serde_json::json!({ "branch": "main", "author": "alex", "message": "import the exported model", "okf": expected }),
        ))
        .await
        .unwrap();
    assert_eq!(committed.status(), StatusCode::CREATED);

    let first = router
        .clone()
        .oneshot(get("/ui/projects/coffee/diagram"))
        .await
        .unwrap();
    assert_eq!(first.status(), StatusCode::OK);
    let first_html = body_text(first).await;

    let second = router
        .clone()
        .oneshot(get("/ui/projects/coffee/diagram"))
        .await
        .unwrap();
    assert_eq!(second.status(), StatusCode::OK);
    let second_html = body_text(second).await;

    // The SAME model produces BYTE-IDENTICAL output, so a diagram can be diffed and cached.
    assert_eq!(
        first_html, second_html,
        "the same model must render byte-identical output"
    );

    let svg = extract_svg(&first_html);

    // Every node and edge appears: the corpus has 99 graph nodes and 165 graph edges.
    assert_eq!(
        count(svg, "class='node'"),
        99,
        "every graph node must be drawn"
    );
    assert_eq!(
        count(svg, "class='edge'"),
        165,
        "every graph edge must be drawn"
    );

    // Both dangling endpoints are drawn as explicit markers, never dropped.
    assert_eq!(
        count(svg, "class='dangling'"),
        2,
        "both dangling endpoints must be drawn as markers"
    );
    assert!(
        svg.contains("_2026x_1_12a70364_1789524431087_459748_5711"),
        "the first dangling endpoint id must be shown"
    );
    assert!(
        svg.contains("_2026x_1_12a70364_1789524431091_435540_5713"),
        "the second dangling endpoint id must be shown"
    );

    // Requirement nodes are visually distinct: 25 of them carry the requirement fill.
    assert_eq!(
        count(svg, "fill='#fff3cd'"),
        25,
        "25 requirement nodes must be visually distinct"
    );

    // The SVG contains no script and no external reference.
    assert!(
        !svg.contains("<script"),
        "the SVG must contain no script tag"
    );
    assert!(
        !svg.contains("href"),
        "the SVG must contain no external reference"
    );
    assert!(
        !svg.contains("xlink:href"),
        "the SVG must contain no xlink reference"
    );
    assert!(
        !svg.contains("url("),
        "the SVG must contain no url() reference"
    );
    assert!(
        !svg.contains("<image"),
        "the SVG must contain no image reference"
    );
    assert!(
        !svg.contains("<use "),
        "the SVG must contain no use reference"
    );
    assert!(
        !svg.contains("<foreignObject"),
        "the SVG must contain no foreignObject"
    );

    // The model page links to the diagram, keeping the two views consistent.
    let model = router
        .clone()
        .oneshot(get("/ui/projects/coffee/model"))
        .await
        .unwrap();
    assert_eq!(model.status(), StatusCode::OK);
    let model_html = body_text(model).await;
    // The link names the COMMIT being viewed, not the branch. Linked by branch, opening a
    // historical model and then following the diagram link would show the branch's current
    // tip - a different model than the one on screen, which is the kind of quiet mismatch a
    // reader has no way to notice.
    assert!(
        model_html.contains("/ui/projects/coffee/diagram?commit="),
        "the model page must link to the diagram of the commit it is showing, got:\n{}",
        model_html
    );
}

#[tokio::test]
async fn the_diagram_resolves_branch_and_commit_and_404s_unknown() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    router
        .clone()
        .oneshot(post("/projects", serde_json::json!({ "name": "coffee" })))
        .await
        .unwrap();
    let expected: serde_json::Value =
        serde_json::from_str(&test_support::load_okf_expected()).unwrap();
    let committed = router
        .clone()
        .oneshot(post(
            "/projects/coffee/commits",
            serde_json::json!({ "branch": "main", "author": "alex", "message": "import the exported model", "okf": expected }),
        ))
        .await
        .unwrap();
    let hash = json_body(committed).await["hash"]
        .as_str()
        .unwrap()
        .to_string();

    for uri in [
        "/ui/projects/coffee/diagram?branch=main".to_string(),
        format!("/ui/projects/coffee/diagram?commit={}", hash),
    ] {
        let response = router.clone().oneshot(get(&uri)).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{} must render", uri);
        let html = body_text(response).await;
        assert!(
            extract_svg(&html).contains("class='node'"),
            "{} must render the graph",
            uri
        );
    }

    let bad_branch = router
        .clone()
        .oneshot(get("/ui/projects/coffee/diagram?branch=nope"))
        .await
        .unwrap();
    assert_eq!(bad_branch.status(), StatusCode::NOT_FOUND);

    let bad_commit = router
        .clone()
        .oneshot(get("/ui/projects/coffee/diagram?commit=deadbeef"))
        .await
        .unwrap();
    assert_eq!(bad_commit.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_hostile_label_in_the_diagram_is_escaped() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(SqliteStore::open(&dir.path().join("mw.db")).unwrap());
    // A hostile node name, a hostile dangling endpoint id and a hostile edge label, seeded
    // straight through the store so the page can never trust any of them.
    let model = serde_json::json!({
        "project": "coffee",
        "exportedAt": "2026-09-17T00:00:00Z",
        "summary": {},
        "stateMachine": { "name": "sm", "regions": [] },
        "requirements": [],
        "structure": [],
        "graph": {
            "nodes": [
                { "id": "n1", "kind": "block", "name": "<script>alert(1)</script>" }
            ],
            "edges": [
                { "source": "n1", "target": "<script>alert(2)</script>", "kind": "dependency", "label": "<script>alert(3)</script>" }
            ]
        }
    });
    seed_model_directly(store.as_ref(), model);
    let router = server::app(AppState {
        store,
        evidence_dir: dir.path().to_path_buf(),
        auth: AuthConfig::Open,
    });

    let response = router
        .oneshot(get("/ui/projects/coffee/diagram"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;
    let svg = extract_svg(&html);

    assert!(
        svg.contains("&lt;script&gt;alert(1)&lt;/script&gt;"),
        "the hostile node name must be escaped"
    );
    assert!(
        svg.contains("&lt;script&gt;alert(2)&lt;/script&gt;"),
        "the hostile dangling endpoint id must be escaped"
    );
    assert!(
        svg.contains("&lt;script&gt;alert(3)&lt;/script&gt;"),
        "the hostile edge label must be escaped"
    );
    assert!(
        !svg.contains("<script"),
        "a raw script tag must never reach the SVG, got:\n{}",
        svg
    );
}

/// A model exercising every symbol path: known kinds, an unknown kind, a declared 2525 SIDC and
/// a declared-but-unmappable SIDC.
fn symbol_demo_model() -> serde_json::Value {
    serde_json::json!({
        "project": "coffee",
        "exportedAt": "2026-09-17T00:00:00Z",
        "summary": {},
        "stateMachine": { "name": "sm", "regions": [] },
        "requirements": [],
        "graph": {
            "nodes": [
                { "id": "b1", "kind": "block", "name": "Pump" },
                { "id": "a1", "kind": "activity", "name": "Brew" },
                { "id": "s1", "kind": "signal", "name": "Ready" },
                { "id": "i1", "kind": "interface", "name": "Port" },
                { "id": "u1", "kind": "mystery", "name": "Unknown kind" },
                { "id": "t1", "kind": "block", "name": "Friendly vehicle", "stereotypes": ["sidc:10310000012000000000"] },
                { "id": "x1", "kind": "block", "name": "Unmapped", "stereotypes": ["sidc:10399000012000000000"] }
            ],
            "edges": []
        }
    })
}

async fn symbol_demo_page(dir: &tempfile::TempDir) -> String {
    let store = Arc::new(SqliteStore::open(&dir.path().join("mw.db")).unwrap());
    seed_model_directly(store.as_ref(), symbol_demo_model());
    let router = server::app(AppState {
        store,
        evidence_dir: dir.path().to_path_buf(),
        auth: AuthConfig::Open,
    });
    let response = router
        .oneshot(get("/ui/projects/coffee/diagram"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    body_text(response).await
}

#[tokio::test]
async fn the_diagram_emits_a_glyph_per_kind_and_a_neutral_default() {
    let dir = tempfile::tempdir().unwrap();
    let html = symbol_demo_page(&dir).await;
    let svg = extract_svg(&html);
    // Every node carries a glyph group (kind mark or declared 2525 symbol).
    assert_eq!(count(svg, "node-glyph"), 7, "a glyph per node");
    // An unknown kind degrades to the neutral default (a plain square), never nothing.
    assert!(
        svg.contains("M5 5 H19 V19 H5 Z"),
        "an unknown kind must draw the neutral default glyph"
    );
    // Known kinds draw their own glyphs, distinct from the neutral default.
    assert!(
        svg.contains("M12 3 L20 7 L20 17 L12 21 L4 17 L4 7 Z"),
        "the block glyph must be drawn"
    );
    assert!(
        svg.contains("M13 3 L6 13 H10 L9 21 L18 9 H13 Z"),
        "the signal glyph must be drawn"
    );
}

#[tokio::test]
async fn a_declared_sidc_renders_a_2525_frame() {
    let dir = tempfile::tempdir().unwrap();
    let html = symbol_demo_page(&dir).await;
    let svg = extract_svg(&html);
    // A declared friend-land SIDC draws the standard frame, filled with the friend colour, and
    // a generic platform glyph.
    assert!(
        svg.contains("class='mw-2525-frame'"),
        "the 2525 frame must be drawn"
    );
    assert!(
        svg.contains("fill='#3d8bfd'"),
        "the friend affiliation must fill the frame with its standard colour"
    );
    assert!(
        svg.contains("class='mw-2525-glyph'"),
        "the platform glyph must be drawn"
    );
}

#[tokio::test]
async fn an_unknown_declared_symbol_is_reported_not_silently_boxed() {
    let dir = tempfile::tempdir().unwrap();
    let html = symbol_demo_page(&dir).await;
    let svg = extract_svg(&html);
    // The node is flagged with its declaration, and the page reports it rather than hiding it.
    assert!(
        svg.contains("data-mw-unmappable='10399000012000000000'"),
        "the unmappable node must carry its declared symbol"
    );
    assert!(
        html.contains("symbol-report"),
        "the report must be rendered"
    );
    assert!(
        html.contains("10399000012000000000"),
        "the declared symbol must be named in the report"
    );
}

#[tokio::test]
async fn the_diagram_routes_node_to_node_edges_orthogonally() {
    // Two children of one parent land in different rows of the child layer, so the parent->child
    // edges leave and arrive at different heights. Each such edge must be an orthogonal polyline,
    // never a straight diagonal that could wander through a box.
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    router
        .clone()
        .oneshot(post("/projects", serde_json::json!({ "name": "coffee" })))
        .await
        .unwrap();
    let model = serde_json::json!({
        "project": "coffee",
        "exportedAt": "2026-09-17T00:00:00Z",
        "summary": {},
        "stateMachine": { "name": "sm", "regions": [] },
        "requirements": [],
        "graph": {
            "nodes": [
                { "id": "p", "kind": "block", "name": "Parent" },
                { "id": "c1", "kind": "block", "name": "Child One" },
                { "id": "c2", "kind": "block", "name": "Child Two" }
            ],
            "edges": [
                { "source": "p", "target": "c1", "kind": "part", "label": "" },
                { "source": "p", "target": "c2", "kind": "part", "label": "" }
            ]
        }
    });
    router
        .clone()
        .oneshot(post(
            "/projects/coffee/commits",
            serde_json::json!({ "branch": "main", "author": "alex", "message": "seed", "okf": model }),
        ))
        .await
        .unwrap();
    let response = router
        .clone()
        .oneshot(get("/ui/projects/coffee/diagram"))
        .await
        .unwrap();
    let html = body_text(response).await;
    let svg = extract_svg(&html);
    assert!(
        svg.contains("<polyline class='mw-edge"),
        "node-to-node edges must be orthogonal polylines, got:\n{}",
        svg
    );
    assert!(
        !svg.contains("<line class='mw-edge"),
        "a node-to-node edge must never be a straight line, got:\n{}",
        svg
    );
}

#[tokio::test]
async fn renaming_an_element_carries_its_references() {
    // A rename that does not move the graph is silent corruption: the element stays in the
    // document, vanishes from coverage and traceability, and the author has no way to put it
    // right from the workbench. The rename must carry the node and every edge endpoint.
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));

    let created = router
        .clone()
        .oneshot(post("/projects", serde_json::json!({ "name": "coffee" })))
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::CREATED);

    let model = serde_json::json!({
        "project": "coffee",
        "exportedAt": "2026-09-17T00:00:00Z",
        "summary": {},
        "stateMachine": { "name": "sm", "regions": [] },
        "requirements": [],
        "structure": [{ "id": "b1", "name": "Block", "kind": "block", "stereotypes": ["Block"], "attributes": [], "documentation": "" }],
        "graph": { "nodes": [{ "id": "b1", "kind": "block", "name": "Block" }], "edges": [] }
    });
    let base = router
        .clone()
        .oneshot(post(
            "/projects/coffee/commits",
            serde_json::json!({ "branch": "main", "author": "alex", "message": "base", "okf": model }),
        ))
        .await
        .unwrap();
    assert_eq!(base.status(), StatusCode::CREATED);

    let response = router
        .clone()
        .oneshot(post_form(
            "/ui/projects/coffee/edit/b1",
            &[
                ("branch", "main"),
                ("message", "give the block a real name"),
                ("id", "brewing_unit"),
                ("name", "Brewing Unit"),
                ("documentation", ""),
                ("stereotypes", "Block"),
            ],
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    // The editor answers with a PAGE, not JSON, so the new tip is read back through the
    // JSON API rather than parsed out of the HTML.
    let branches = router
        .clone()
        .oneshot(get("/projects/coffee/branches"))
        .await
        .unwrap();
    let branches = json_body(branches).await;
    let hash = branches
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["name"] == "main")
        .unwrap()["tip"]
        .as_str()
        .unwrap()
        .to_string();

    let doc = router
        .oneshot(get(&format!("/projects/coffee/commits/{}", hash)))
        .await
        .unwrap();
    let doc = json_body(doc).await;
    assert_eq!(doc["structure"][0]["id"], "brewing_unit");
    assert_eq!(
        doc["graph"]["nodes"][0]["id"], "brewing_unit",
        "the graph node must follow the rename"
    );

    // And the validator must see nothing orphaned: the cross-check that used to be the only
    // defence is now satisfied by construction.
    let root: OkfRoot = serde_json::from_value(doc).unwrap();
    let report = okf::validate::validate(&root);
    assert!(
        !report
            .warnings
            .iter()
            .any(|w| w.contains("no node in the graph")),
        "a rename must not orphan the element: {:?}",
        report.warnings
    );
}

// ---------------------------------------------------------------------------
// Task 4: the import page, where a person decides about a migration.

/// Read a hand-written SysML v1 XMI fixture from the binding's corpus, the same synthetic
/// documents the migration tests use so the page is pinned to the same hand-checked losses.
fn xmi_fixture(name: &str) -> String {
    let path = format!(
        "{}/../engine/binding-xmi/fixtures/{}",
        env!("CARGO_MANIFEST_DIR"),
        name
    );
    std::fs::read_to_string(path).expect("fixture must exist")
}

#[tokio::test]
async fn the_import_page_offers_the_registry_binding() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    router
        .clone()
        .oneshot(post("/projects", serde_json::json!({ "name": "coffee" })))
        .await
        .unwrap();

    let response = router
        .oneshot(get("/ui/projects/coffee/import"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;
    assert!(
        html.contains("sysml-v1-xmi@2.4"),
        "the registry binding must be offered"
    );
    assert!(
        html.contains(r#"name="artifact""#),
        "the artifact field must render"
    );
    assert!(
        html.contains(r#"name="binding""#),
        "the binding selector must render"
    );
}

#[tokio::test]
async fn a_lossy_import_shows_its_losses_and_requires_acceptance_before_committing() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    router
        .clone()
        .oneshot(post("/projects", serde_json::json!({ "name": "coffee" })))
        .await
        .unwrap();

    let artifact = xmi_fixture("coffee-grinder.xmi");
    let expected_hash = server::store::blob_hash(artifact.as_bytes());

    // Without acceptance the import is refused, and the page shows the losses by name.
    let refused = router
        .clone()
        .oneshot(post_form(
            "/ui/projects/coffee/import",
            &[
                ("binding", "sysml-v1-xmi@2.4"),
                ("branch", "main"),
                ("message", "import coffee-grinder"),
                ("artifact", artifact.as_str()),
            ],
        ))
        .await
        .unwrap();
    assert_eq!(refused.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let html = body_text(refused).await;

    // The page must not claim a lossless migration the report contradicts.
    assert!(
        html.contains("lossy"),
        "the page must say the import is lossy"
    );
    assert!(
        !html.contains("lossless"),
        "the page must not claim a lossless migration"
    );
    assert!(
        !html.contains("Import committed"),
        "a refused import must not render success"
    );

    // Blocking entries are named, grouped by verdict, each naming its subject.
    assert!(
        html.contains("<h3>Lossy</h3>"),
        "the lossy group must render"
    );
    assert!(
        html.contains("uml:Model model-grinder"),
        "a subject must be named"
    );
    assert!(
        html.contains("uml:Package pkg-structure (Structure)"),
        "a subject must be named"
    );
    assert!(
        html.contains("uml:Comment doc-grinder"),
        "a subject must be named"
    );

    // The acceptance checkbox submits the ENTRY IDENTITY, not the raw subject, so what a
    // human ticks is exactly what the server records.
    assert!(
        html.contains(r#"value="uml:Model model-grinder [lossy]""#),
        "the acceptance checkbox must carry the entry identity"
    );

    // What was retained is stated on the page, not only in a log.
    assert!(
        html.contains(expected_hash.as_str()),
        "the retained artifact hash must be named"
    );

    // Accepting the six named losses commits the import, through the SAME core the
    // endpoint calls.
    let accepted = router
        .clone()
        .oneshot(post_form(
            "/ui/projects/coffee/import",
            &[
                ("binding", "sysml-v1-xmi@2.4"),
                ("branch", "main"),
                ("message", "import coffee-grinder"),
                ("artifact", artifact.as_str()),
                ("accept_loss_0", "uml:Model model-grinder [lossy]"),
                ("accept_loss_1", "uml:Comment doc-grinder [lossy]"),
                ("accept_loss_2", "uml:Property prop-motor [lossy]"),
                ("accept_loss_3", "uml:Property prop-capacity [lossy]"),
                ("accept_loss_4", "uml:Dependency dep-satisfy [lossy]"),
                (
                    "accept_loss_5",
                    "uml:Package pkg-structure (Structure) [lossy]",
                ),
            ],
        ))
        .await
        .unwrap();
    assert_eq!(accepted.status(), StatusCode::CREATED);
    let html = body_text(accepted).await;
    assert!(
        html.contains("Import committed"),
        "the success page must render"
    );
    assert!(
        html.contains(expected_hash.as_str()),
        "the retained artifact hash must be named on success"
    );

    // The commit landed: the page and the endpoint share the same commit path.
    let commits = router
        .oneshot(get("/projects/coffee/commits?branch=main"))
        .await
        .unwrap();
    let commits = json_body(commits).await;
    assert_eq!(
        commits.as_array().unwrap().len(),
        1,
        "the accepted import must have committed exactly one commit"
    );
}

#[tokio::test]
async fn bulk_accept_imports_every_loss_in_one_click() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    router
        .clone()
        .oneshot(post("/projects", serde_json::json!({ "name": "coffee" })))
        .await
        .unwrap();

    let artifact = xmi_fixture("coffee-grinder.xmi");
    let hash = server::store::blob_hash(artifact.as_bytes());

    // The refusal page carries the bulk controls with a visible count.
    let refused = router
        .clone()
        .oneshot(post_form(
            "/ui/projects/coffee/import",
            &[
                ("binding", "sysml-v1-xmi@2.4"),
                ("branch", "main"),
                ("message", "import coffee-grinder"),
                ("artifact", artifact.as_str()),
            ],
        ))
        .await
        .unwrap();
    assert_eq!(refused.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let html = body_text(refused).await;
    assert!(
        html.contains("<strong>6</strong>"),
        "the visible count of blocking losses must render"
    );
    assert!(
        html.contains("blocking losses"),
        "the count label must render"
    );
    assert!(
        html.contains("0 selected"),
        "the initial selected count must render"
    );
    assert!(
        html.contains("Accept all 6 losses and import"),
        "the accept-all action must render"
    );
    assert!(
        html.contains("mw-select-all"),
        "the select-all toggle must render"
    );
    assert!(
        html.contains("all_losses"),
        "the all-losses hidden field must render"
    );

    // One click: accept_all plus the newline-joined identities, no per-loss checkboxes.
    let all_losses = [
        "uml:Model model-grinder [lossy]",
        "uml:Comment doc-grinder [lossy]",
        "uml:Property prop-motor [lossy]",
        "uml:Property prop-capacity [lossy]",
        "uml:Dependency dep-satisfy [lossy]",
        "uml:Package pkg-structure (Structure) [lossy]",
    ]
    .join(
        "
",
    );

    let accepted = router
        .clone()
        .oneshot(post_form(
            "/ui/projects/coffee/import/accept",
            &[
                ("artifactHash", hash.as_str()),
                ("branch", "main"),
                ("message", "accept all losses"),
                ("accept_all", "1"),
                ("all_losses", all_losses.as_str()),
            ],
        ))
        .await
        .unwrap();
    assert_eq!(accepted.status(), StatusCode::CREATED, "{:?}", accepted);
    let html = body_text(accepted).await;
    assert!(html.contains("Import committed"), "success must render");

    // Exactly one commit: the whole report was accepted at once.
    let commits = router
        .oneshot(get("/projects/coffee/commits?branch=main"))
        .await
        .unwrap();
    assert_eq!(json_body(commits).await.as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn an_unmapped_import_groups_by_verdict_with_blocking_first() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    router
        .clone()
        .oneshot(post("/projects", serde_json::json!({ "name": "coffee" })))
        .await
        .unwrap();

    let artifact = xmi_fixture("unknown-element.xmi");
    let refused = router
        .clone()
        .oneshot(post_form(
            "/ui/projects/coffee/import",
            &[
                ("binding", "sysml-v1-xmi@2.4"),
                ("branch", "main"),
                ("message", "import unknown"),
                ("artifact", artifact.as_str()),
            ],
        ))
        .await
        .unwrap();
    assert_eq!(refused.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let html = body_text(refused).await;

    // Both verdict groups render, with the unmapped subjects named.
    assert!(
        html.contains("<h3>Unmappable</h3>"),
        "the unmappable group must render"
    );
    assert!(
        html.contains("<h3>Lossy</h3>"),
        "the lossy group must render"
    );
    assert!(
        html.contains("uml:StateMachine sm-1"),
        "the unmapped element must be named"
    );
    assert!(
        html.contains("uml:Class block-1 attribute"),
        "the unmapped attribute must be named"
    );
    assert!(
        html.contains("uml:Model model-unknown"),
        "the lossy subject must be named"
    );

    // Blocking entries render first: unmappable (most severe) precedes lossy.
    let unmappable_at = html.find("<h3>Unmappable</h3>");
    let lossy_at = html.find("<h3>Lossy</h3>");
    assert!(
        unmappable_at.is_some() && lossy_at.is_some() && unmappable_at.unwrap() < lossy_at.unwrap(),
        "the blocking unmappable group must render before the lossy group"
    );
}

#[tokio::test]
async fn a_viewer_cannot_start_an_import() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(SqliteStore::open(&dir.path().join("mw.db")).unwrap());
    store.create_project("coffee", None).unwrap();
    let router = server::app(AppState {
        store,
        evidence_dir: dir.path().to_path_buf(),
        auth: AuthConfig::fixed(viewer()),
    });

    let response = router
        .oneshot(post_form(
            "/ui/projects/coffee/import",
            &[
                ("binding", "sysml-v1-xmi@2.4"),
                ("branch", "main"),
                ("message", "steal"),
                ("artifact", "<xmi/>"),
            ],
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let html = body_text(response).await;
    assert!(html.contains("<html"), "a 403 must be a page, not JSON");
    assert!(
        html.contains("write permission required"),
        "the page must name the permission refusal"
    );
}
// ---------------------------------------------------------------------------
// The creation pages: project, model, element and branch.

/// The Location header of a redirect response, so a test can follow where the form landed
/// the caller and read the commit hash out of the model URL.
fn location(response: &axum::response::Response) -> Option<String> {
    response
        .headers()
        .get("location")
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
}

#[tokio::test]
async fn a_project_is_created_from_the_project_list() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));

    let list = router.clone().oneshot(get("/ui")).await.unwrap();
    let list_html = body_text(list).await;
    assert!(
        list_html.contains("New project"),
        "the list must offer the create form, got:\n{}",
        list_html
    );
    assert!(
        list_html.contains("No projects yet. Create one below."),
        "the empty list must say what to do next, got:\n{}",
        list_html
    );

    let created = router
        .clone()
        .oneshot(post_form("/ui", &[("name", "coffee")]))
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        location(&created).as_deref(),
        Some("/ui/projects/coffee"),
        "creating a project must land the caller on it"
    );

    let list = router.oneshot(get("/ui")).await.unwrap();
    let html = body_text(list).await;
    assert!(html.contains("coffee"), "the new project must be listed");
}

#[tokio::test]
async fn a_refused_project_create_renders_the_reason_and_a_duplicate_conflicts() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));

    let invalid = router
        .clone()
        .oneshot(post_form("/ui", &[("name", "bad name!")]))
        .await
        .unwrap();
    assert_eq!(invalid.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let html = body_text(invalid).await;
    assert!(
        html.contains("project name may contain only letters, digits, dot, underscore and hyphen"),
        "the refusal must name the reason, got:\n{}",
        html
    );

    let created = router
        .clone()
        .oneshot(post_form("/ui", &[("name", "coffee")]))
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::SEE_OTHER);

    let duplicate = router
        .oneshot(post_form("/ui", &[("name", "coffee")]))
        .await
        .unwrap();
    assert_eq!(duplicate.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn a_viewer_is_not_offered_creation_and_cannot_create() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state_with_auth(dir.path(), AuthConfig::fixed(viewer())));

    let list = router.clone().oneshot(get("/ui")).await.unwrap();
    let html = body_text(list).await;
    assert!(
        !html.contains("New project"),
        "a viewer must not be offered the create form"
    );

    let created = router
        .oneshot(post_form("/ui", &[("name", "coffee")]))
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::FORBIDDEN);
    let html = body_text(created).await;
    assert!(
        html.contains("admin permission required"),
        "the refusal must name the permission"
    );
}

#[tokio::test]
async fn an_empty_project_offers_to_start_a_model() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    router
        .clone()
        .oneshot(post_form("/ui", &[("name", "coffee")]))
        .await
        .unwrap();

    let page = router
        .clone()
        .oneshot(get("/ui/projects/coffee/model"))
        .await
        .unwrap();
    assert_eq!(page.status(), StatusCode::OK);
    let html = body_text(page).await;
    assert!(
        html.contains("This project has no model yet — start one or import a legacy model."),
        "the empty state must say what to do next, got:\n{}",
        html
    );
    assert!(
        html.contains("Start from empty"),
        "the start-empty action must be there"
    );
    assert!(
        html.contains("Create from pasted document"),
        "the paste action must be there"
    );
    assert!(
        html.contains("Import a legacy model"),
        "the import action must be there"
    );
}

#[tokio::test]
async fn a_model_can_be_started_from_empty() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    router
        .clone()
        .oneshot(post_form("/ui", &[("name", "coffee")]))
        .await
        .unwrap();

    let created = router
        .clone()
        .oneshot(post_form(
            "/ui/projects/coffee/model/new",
            &[
                ("mode", "empty"),
                ("branch", "main"),
                ("message", "start the model"),
            ],
        ))
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::SEE_OTHER);
    assert!(
        location(&created)
            .unwrap()
            .starts_with("/ui/projects/coffee/model?commit="),
        "starting a model must land on it"
    );

    let page = router
        .clone()
        .oneshot(get("/ui/projects/coffee/model"))
        .await
        .unwrap();
    assert_eq!(page.status(), StatusCode::OK);
    let html = body_text(page).await;
    assert!(
        !html.contains("no model yet"),
        "the empty state must be gone"
    );
    assert!(
        html.contains("This model has no structure elements."),
        "an empty model renders, not the empty-state page"
    );

    let commits = router
        .oneshot(get("/projects/coffee/commits?branch=main"))
        .await
        .unwrap();
    let body = json_body(commits).await;
    assert_eq!(body.as_array().unwrap().len(), 1);
    assert_eq!(body[0]["message"], "start the model");
    assert_eq!(body[0]["author"], "anonymous");
}

#[tokio::test]
async fn a_model_can_be_created_from_a_pasted_document() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    router
        .clone()
        .oneshot(post_form("/ui", &[("name", "coffee")]))
        .await
        .unwrap();

    let pasted = serde_json::json!({
        "project": "coffee",
        "stateMachine": { "name": "sm", "regions": [] },
        "structure": [{ "id": "b1", "name": "Block", "kind": "block", "stereotypes": [], "attributes": [], "documentation": "" }],
        "requirements": [],
        "graph": { "nodes": [{ "id": "b1", "kind": "block", "name": "Block" }], "edges": [] }
    })
    .to_string();

    let created = router
        .clone()
        .oneshot(post_form(
            "/ui/projects/coffee/model/new",
            &[
                ("mode", "paste"),
                ("branch", "main"),
                ("message", "paste it"),
                ("okf", pasted.as_str()),
            ],
        ))
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::SEE_OTHER);

    let page = router
        .clone()
        .oneshot(get("/ui/projects/coffee/model"))
        .await
        .unwrap();
    let html = body_text(page).await;
    assert!(
        html.contains("Block"),
        "the pasted block must render, got:\n{}",
        html
    );
}

#[tokio::test]
async fn a_pasted_document_that_is_not_json_is_refused_with_the_reason() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    router
        .clone()
        .oneshot(post_form("/ui", &[("name", "coffee")]))
        .await
        .unwrap();

    let refused = router
        .oneshot(post_form(
            "/ui/projects/coffee/model/new",
            &[
                ("mode", "paste"),
                ("branch", "main"),
                ("message", "bad"),
                ("okf", "this is not json"),
            ],
        ))
        .await
        .unwrap();
    assert_eq!(refused.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let html = body_text(refused).await;
    assert!(
        html.contains("not valid JSON"),
        "the refusal must name the parse failure, got:\n{}",
        html
    );
}

#[tokio::test]
async fn a_requirement_needs_a_req_id_and_a_duplicate_id_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    router
        .clone()
        .oneshot(post_form("/ui", &[("name", "coffee")]))
        .await
        .unwrap();
    router
        .clone()
        .oneshot(post_form(
            "/ui/projects/coffee/model/new",
            &[("mode", "empty"), ("branch", "main"), ("message", "start")],
        ))
        .await
        .unwrap();

    let no_req_id = router
        .clone()
        .oneshot(post_form(
            "/ui/projects/coffee/element/new",
            &[
                ("kind", "requirement"),
                ("id", "r1"),
                ("name", "Heat"),
                ("req_id", ""),
                ("text", "shall heat"),
                ("branch", "main"),
                ("message", "add requirement"),
            ],
        ))
        .await
        .unwrap();
    assert_eq!(no_req_id.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let html = body_text(no_req_id).await;
    assert!(
        html.contains("a requirement needs a reqId"),
        "the refusal must name the missing reqId"
    );

    // Add one block successfully, then try to add a second block with the same id.
    let first = router
        .clone()
        .oneshot(post_form(
            "/ui/projects/coffee/element/new",
            &[
                ("kind", "block"),
                ("id", "b1"),
                ("name", "Heater"),
                ("req_id", ""),
                ("text", ""),
                ("branch", "main"),
                ("message", "add block"),
            ],
        ))
        .await
        .unwrap();
    assert_eq!(first.status(), StatusCode::SEE_OTHER);

    let duplicate = router
        .oneshot(post_form(
            "/ui/projects/coffee/element/new",
            &[
                ("kind", "block"),
                ("id", "b1"),
                ("name", "Other"),
                ("req_id", ""),
                ("text", ""),
                ("branch", "main"),
                ("message", "duplicate"),
            ],
        ))
        .await
        .unwrap();
    assert_eq!(duplicate.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let html = body_text(duplicate).await;
    assert!(
        html.contains("duplicate element id b1"),
        "the validator's duplicate-id error must render, got:\n{}",
        html
    );
}

#[tokio::test]
async fn the_full_flow_builds_a_model_with_a_block_a_requirement_and_a_branch() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));

    // 1. Create the project through the UI.
    let created = router
        .clone()
        .oneshot(post_form("/ui", &[("name", "coffee")]))
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::SEE_OTHER);
    assert_eq!(location(&created).as_deref(), Some("/ui/projects/coffee"));

    // 2. Start the first model from empty.
    let started = router
        .clone()
        .oneshot(post_form(
            "/ui/projects/coffee/model/new",
            &[
                ("mode", "empty"),
                ("branch", "main"),
                ("message", "start the model"),
            ],
        ))
        .await
        .unwrap();
    assert_eq!(started.status(), StatusCode::SEE_OTHER);

    // 3. Add a block.
    let block = router
        .clone()
        .oneshot(post_form(
            "/ui/projects/coffee/element/new",
            &[
                ("kind", "block"),
                ("id", "b1"),
                ("name", "Heater"),
                ("req_id", ""),
                ("text", ""),
                ("branch", "main"),
                ("message", "add the heater block"),
            ],
        ))
        .await
        .unwrap();
    assert_eq!(block.status(), StatusCode::SEE_OTHER);

    // 4. Add a requirement.
    let requirement = router
        .clone()
        .oneshot(post_form(
            "/ui/projects/coffee/element/new",
            &[
                ("kind", "requirement"),
                ("id", "r1"),
                ("name", "Heat"),
                ("req_id", "1.1"),
                ("text", "the machine shall heat"),
                ("branch", "main"),
                ("message", "add the heat requirement"),
            ],
        ))
        .await
        .unwrap();
    assert_eq!(requirement.status(), StatusCode::SEE_OTHER);

    // The model page shows both new elements.
    let page = router
        .clone()
        .oneshot(get("/ui/projects/coffee/model"))
        .await
        .unwrap();
    assert_eq!(page.status(), StatusCode::OK);
    let html = body_text(page).await;
    assert!(html.contains("Heater"), "the block must render");
    assert!(
        html.contains("the machine shall heat"),
        "the requirement text must render"
    );

    // The commits are exactly the three writes, oldest first.
    let commits = router
        .clone()
        .oneshot(get("/projects/coffee/commits?branch=main"))
        .await
        .unwrap();
    let body = json_body(commits).await;
    let list = body.as_array().unwrap();
    assert_eq!(
        list.len(),
        3,
        "three commits must exist, got {}",
        list.len()
    );
    let messages: Vec<&str> = list
        .iter()
        .map(|c| c["message"].as_str().unwrap())
        .collect();
    assert_eq!(
        messages,
        vec![
            "start the model",
            "add the heater block",
            "add the heat requirement"
        ]
    );

    // 5. Create a branch from main's tip.
    let tip = list[2]["hash"].as_str().unwrap().to_string();
    let branched = router
        .clone()
        .oneshot(post_form(
            "/ui/projects/coffee/branch",
            &[("name", "feature"), ("from", tip.as_str())],
        ))
        .await
        .unwrap();
    assert_eq!(branched.status(), StatusCode::SEE_OTHER);

    let project_page = router.oneshot(get("/ui/projects/coffee")).await.unwrap();
    let project_html = body_text(project_page).await;
    assert!(
        project_html.contains("feature"),
        "the new branch must be listed, got:\n{}",
        project_html
    );
}

// ---------------------------------------------------------------------------
// The model-page IDE enhancement: one static asset, progressive and additive. With
// JavaScript disabled the page is the same server-rendered list; with it enabled the
// asset builds the containment tree and properties panel from data the page already
// carries, so the enhancement is never a second source of truth.

#[tokio::test]
async fn the_app_js_asset_is_served() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));

    let response = router.oneshot(get("/ui/app.js")).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let content_type = response
        .headers()
        .get("content-type")
        .expect("the asset must declare its type")
        .to_str()
        .unwrap()
        .to_string();
    assert!(
        content_type.starts_with("text/javascript"),
        "the asset must be served as JavaScript, got {}",
        content_type
    );
    let body = body_text(response).await;
    assert!(
        body.contains("SPDX-License-Identifier: AGPL-3.0-or-later"),
        "the asset must carry the AGPL header"
    );
    assert!(
        body.contains("modelwrite"),
        "the asset must be the workbench enhancement script"
    );
}

#[tokio::test]
async fn the_no_js_model_page_keeps_the_server_rendered_content() {
    // The enhancement must be ADDITIVE. The script tag and data-* attributes are inert
    // without the script, so a no-JS reader sees the same four sections, the same counts
    // and the same engine coverage this page has always rendered.
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    router
        .clone()
        .oneshot(post("/projects", serde_json::json!({ "name": "coffee" })))
        .await
        .unwrap();
    let expected: serde_json::Value =
        serde_json::from_str(&test_support::load_okf_expected()).unwrap();
    let committed = router
        .clone()
        .oneshot(post(
            "/projects/coffee/commits",
            serde_json::json!({ "branch": "main", "author": "alex", "message": "import the exported model", "okf": expected }),
        ))
        .await
        .unwrap();
    assert_eq!(committed.status(), StatusCode::CREATED);

    let response = router
        .oneshot(get("/ui/projects/coffee/model"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;

    assert!(html.contains("<h2>Structure</h2>"), "structure section");
    assert!(
        html.contains("<h2>Requirements</h2>"),
        "requirements section"
    );
    assert!(
        html.contains("<h2>Traceability</h2>"),
        "traceability section"
    );
    assert!(
        html.contains("<h2>State and activity</h2>"),
        "state and activity section"
    );
    assert_eq!(
        count(&html, "<li class=\"element\""),
        49,
        "49 structure elements"
    );
    assert_eq!(count(&html, "<li class=\"signal\""), 9, "9 signals");
    assert_eq!(
        count(&html, "<tr class=\"requirement\""),
        25,
        "25 requirements"
    );
    assert_eq!(
        count(&html, "<tr class=\"trace-row\""),
        25,
        "25 matrix rows"
    );
    assert_eq!(count(&html, "class=\"activity\""), 8, "8 activities");
    assert_eq!(count(&html, "class=\"state\""), 7, "7 states");
    assert_eq!(count(&html, "class=\"transition\""), 8, "8 transitions");
    assert!(
        html.contains("<span class=\"covered\">15 covered</span>")
            && html.contains("<span class=\"uncovered\">10 uncovered</span>"),
        "the engine coverage summary must be unchanged"
    );
    assert!(
        html.contains("Coffee Machine"),
        "the root block must render"
    );
    assert!(
        html.contains("Heater Initialization"),
        "a covered requirement must render"
    );
}

#[tokio::test]
async fn the_model_page_embeds_the_data_the_enhancement_reads() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    router
        .clone()
        .oneshot(post("/projects", serde_json::json!({ "name": "coffee" })))
        .await
        .unwrap();
    let expected: serde_json::Value =
        serde_json::from_str(&test_support::load_okf_expected()).unwrap();
    let committed = router
        .clone()
        .oneshot(post(
            "/projects/coffee/commits",
            serde_json::json!({ "branch": "main", "author": "alex", "message": "import the exported model", "okf": expected }),
        ))
        .await
        .unwrap();
    assert_eq!(committed.status(), StatusCode::CREATED);

    let response = router
        .oneshot(get("/ui/projects/coffee/model"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;

    assert!(
        html.contains("<script src=\"/ui/app.js\""),
        "the page must load the single enhancement asset"
    );

    // Every selectable element carries its identity: 49 structure + 9 signals + 25
    // requirements + 8 activities = 91.
    assert_eq!(count(&html, "data-mw-id=\""), 91, "91 selectable elements");
    assert!(
        html.contains("data-mw-name=\"Coffee Machine\""),
        "the root block's name must be readable"
    );
    assert!(
        html.contains("data-mw-kind=\"block\""),
        "a kind must be readable"
    );

    // The fields a properties panel shows, carried as data rather than recomputed.
    assert!(
        html.contains("data-mw-stereotypes=\""),
        "stereotypes must be present"
    );
    assert!(
        html.contains("data-mw-attributes=\""),
        "attributes must be present"
    );
    assert!(
        html.contains("data-mw-documentation=\""),
        "documentation must be present"
    );

    // Requirements carry reqId, text and the ENGINE's coverage verdict (15 covered / 10
    // uncovered, the same numbers the traceability section renders).
    assert!(
        html.contains("data-mw-reqid=\"1\""),
        "a requirement's reqId must be readable"
    );
    assert!(
        html.contains("data-mw-reqtext=\""),
        "a requirement's text must be readable"
    );
    assert_eq!(
        count(&html, "data-mw-coverage=\"covered\""),
        15,
        "15 covered"
    );
    assert_eq!(
        count(&html, "data-mw-coverage=\"uncovered\""),
        10,
        "10 uncovered"
    );
}

#[tokio::test]
async fn the_diagram_carries_the_selection_hook_for_sync() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    router
        .clone()
        .oneshot(post("/projects", serde_json::json!({ "name": "coffee" })))
        .await
        .unwrap();
    let expected: serde_json::Value =
        serde_json::from_str(&test_support::load_okf_expected()).unwrap();
    let committed = router
        .clone()
        .oneshot(post(
            "/projects/coffee/commits",
            serde_json::json!({ "branch": "main", "author": "alex", "message": "import the exported model", "okf": expected }),
        ))
        .await
        .unwrap();
    assert_eq!(committed.status(), StatusCode::CREATED);

    let response = router
        .oneshot(get("/ui/projects/coffee/diagram"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;
    let svg = extract_svg(&html);

    // Every graph node is addressable, so selecting a tree node can highlight its node.
    assert_eq!(
        count(svg, "data-mw-id='"),
        99,
        "every graph node must carry its id"
    );
    assert!(
        svg.contains("class='node' data-mw-id='"),
        "the hook must sit on the node group"
    );
    // The diagram page loads the enhancement too, so ?select=<id> highlights on arrival.
    assert!(
        html.contains("<script src=\"/ui/app.js\""),
        "the diagram page must load the enhancement asset"
    );
}

// ---------------------------------------------------------------------------
// The model-health view: one screen for "what is broken in my model?".

#[tokio::test]
async fn the_global_search_finds_elements_by_name_fragment() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    router
        .clone()
        .oneshot(post("/projects", serde_json::json!({ "name": "coffee" })))
        .await
        .unwrap();
    let expected: serde_json::Value =
        serde_json::from_str(&test_support::load_okf_expected()).unwrap();
    let committed = router
        .clone()
        .oneshot(post(
            "/projects/coffee/commits",
            serde_json::json!({ "branch": "main", "author": "alex", "message": "seed", "okf": expected }),
        ))
        .await
        .unwrap();
    assert_eq!(committed.status(), StatusCode::CREATED);

    // The header box is on the Changes page (the first page after landing), so "find the
    // pump" is one click + one field instead of a hidden tree filter on select pages.
    let changes = router
        .clone()
        .oneshot(get("/ui/projects/coffee"))
        .await
        .unwrap();
    assert_eq!(changes.status(), StatusCode::OK);
    let html = body_text(changes).await;
    assert!(
        html.contains(r#"name="q""#),
        "the header search box must render on the Changes page"
    );
    assert!(
        html.contains("/ui/projects/coffee/search"),
        "the header search must target the search page"
    );

    // Searching a name fragment surfaces the matching block and requirement, each with an
    // open action and a reveal-in-diagram action.
    let search = router
        .oneshot(get("/ui/projects/coffee/search?q=pump"))
        .await
        .unwrap();
    assert_eq!(search.status(), StatusCode::OK);
    let html = body_text(search).await;
    assert!(
        html.contains("Water Pump"),
        "the Water Pump block must match"
    );
    assert!(
        html.contains("Pump Pressure"),
        "the Pump Pressure requirement must match"
    );
    assert!(
        html.contains("/model?branch=main&amp;select="),
        "each match must offer an open action carrying the select hook"
    );
    assert!(
        html.contains("/diagram?branch=main&amp;select="),
        "each match must offer a reveal-in-diagram action"
    );
}

#[tokio::test]
async fn the_health_page_answers_what_is_broken_in_one_screen() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    router
        .clone()
        .oneshot(post("/projects", serde_json::json!({ "name": "coffee" })))
        .await
        .unwrap();
    let expected: serde_json::Value =
        serde_json::from_str(&test_support::load_okf_expected()).unwrap();
    let committed = router
        .clone()
        .oneshot(post(
            "/projects/coffee/commits",
            serde_json::json!({ "branch": "main", "author": "alex", "message": "seed", "okf": expected }),
        ))
        .await
        .unwrap();
    assert_eq!(committed.status(), StatusCode::CREATED);

    let response = router
        .clone()
        .oneshot(get("/ui/projects/coffee/health"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;

    // The corpus has no orphans and one component, but two dangling Satisfy links and
    // ten uncovered requirements — exactly what the audit assembled across six pages.
    assert!(
        html.contains("Orphaned nodes (0)"),
        "orphan count must render, got:\n{}",
        html
    );
    assert!(
        html.contains("Isolated groups (0)"),
        "isolated-group count must render, got:\n{}",
        html
    );
    assert!(
        html.contains("Dangling links (2)"),
        "dangling-link count must render, got:\n{}",
        html
    );
    assert!(
        html.contains("_2026x_1_12a70364_1789524431087_459748_5711"),
        "the first dangling endpoint must be named, got:\n{}",
        html
    );
    assert!(
        html.contains("Uncovered requirements (10)"),
        "uncovered count must render, got:\n{}",
        html
    );
    // The navigator and the overview section links carry Health, so a visitor reaches it
    // from landing in two clicks (open the project, click Health).
    assert!(
        html.contains(">Health<"),
        "the navigator must offer Health, got:\n{}",
        html
    );
}

#[tokio::test]
async fn the_health_page_names_every_finding_by_id_and_name() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    router
        .clone()
        .oneshot(post("/projects", serde_json::json!({ "name": "coffee" })))
        .await
        .unwrap();
    // One orphan block, one orphan requirement, one two-node group cut off from the main
    // body, one dangling Satisfy link, and one uncovered requirement.
    let broken = serde_json::json!({
        "project": "coffee",
        "summary": {},
        "stateMachine": {"name": "sm", "regions": []},
        "structure": [
            {"id": "b1", "name": "Heater Block", "kind": "block", "stereotypes": [], "attributes": [], "documentation": ""},
            {"id": "b2", "name": "Orphan Block", "kind": "block", "stereotypes": [], "attributes": [], "documentation": ""}
        ],
        "requirements": [
            {"id": "r1", "name": "Heat requirement", "kind": "requirement", "stereotypes": [], "attributes": [], "documentation": "", "reqId": "1.1", "reqText": "heats"},
            {"id": "r2", "name": "Cold requirement", "kind": "requirement", "stereotypes": [], "attributes": [], "documentation": "", "reqId": "1.2", "reqText": "cools"}
        ],
        "graph": {
            "nodes": [
                {"id": "b1", "kind": "block", "name": "Heater Block"},
                {"id": "b2", "kind": "block", "name": "Orphan Block"},
                {"id": "r1", "kind": "requirement", "name": "Heat requirement"},
                {"id": "r2", "kind": "requirement", "name": "Cold requirement"},
                {"id": "g1", "kind": "block", "name": "Group One"},
                {"id": "g2", "kind": "block", "name": "Group Two"}
            ],
            "edges": [
                {"source": "b1", "target": "r1", "kind": "dependency", "label": "Satisfy"},
                {"source": "g1", "target": "g2", "kind": "part", "label": ""},
                {"source": "b1", "target": "missing-node", "kind": "dependency", "label": "Satisfy"}
            ]
        }
    });
    let committed = router
        .clone()
        .oneshot(post(
            "/projects/coffee/commits",
            serde_json::json!({ "branch": "main", "author": "alex", "message": "broken", "okf": broken }),
        ))
        .await
        .unwrap();
    assert_eq!(
        committed.status(),
        StatusCode::CREATED,
        "broken model must commit"
    );

    let response = router
        .oneshot(get("/ui/projects/coffee/health"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;

    // Orphaned nodes, named by id and name.
    assert!(html.contains("Orphaned nodes (2)"), "got:\n{}", html);
    assert!(
        html.contains("b2") && html.contains("Orphan Block"),
        "orphan block named, got:\n{}",
        html
    );
    assert!(
        html.contains("r2") && html.contains("Cold requirement"),
        "orphan requirement named, got:\n{}",
        html
    );

    // Isolated groups, each with its members.
    assert!(html.contains("Isolated groups (1)"), "got:\n{}", html);
    assert!(
        html.contains("g1") && html.contains("Group One"),
        "group member named, got:\n{}",
        html
    );
    assert!(
        html.contains("g2") && html.contains("Group Two"),
        "group member named, got:\n{}",
        html
    );

    // Dangling links, naming the missing endpoint.
    assert!(html.contains("Dangling links (1)"), "got:\n{}", html);
    assert!(
        html.contains("missing-node"),
        "missing endpoint named, got:\n{}",
        html
    );

    // Uncovered requirements, named by id and name.
    assert!(
        html.contains("Uncovered requirements (1)"),
        "got:\n{}",
        html
    );
    assert!(
        html.contains("r2") && html.contains("Cold requirement"),
        "uncovered named, got:\n{}",
        html
    );
}

// ---------------------------------------------------------------------------
// The STPA completeness screen: the five checks on one page.

fn stpa_fixture(name: &str) -> serde_json::Value {
    let path = test_support::repo_root().join("sample/stpa").join(name);
    let text = std::fs::read_to_string(path).expect("STPA fixture must exist");
    serde_json::from_str(&text).expect("STPA fixture must parse")
}

async fn commit_stpa_fixture(router: &axum::Router, project: &str, fixture: &str, message: &str) {
    let committed = router
        .clone()
        .oneshot(post(
            &format!("/projects/{project}/commits"),
            serde_json::json!({ "branch": "main", "author": "alex", "message": message, "okf": stpa_fixture(fixture) }),
        ))
        .await
        .unwrap();
    assert_eq!(
        committed.status(),
        StatusCode::CREATED,
        "STPA fixture must commit"
    );
}

#[tokio::test]
async fn the_stpa_page_fires_every_check_on_the_defective_model() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    router
        .clone()
        .oneshot(post("/projects", serde_json::json!({ "name": "fs" })))
        .await
        .unwrap();
    commit_stpa_fixture(
        &router,
        "fs",
        "fire-suppression-defective.json",
        "defective",
    )
    .await;

    let response = router
        .clone()
        .oneshot(get("/ui/projects/fs/stpa"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;

    // The honest boundary is on the page, verbatim.
    assert!(
        html.contains("The analysis is AUTHORED; the check is COMPUTED."),
        "the honest boundary must be stated, got:\n{}",
        html
    );
    assert!(
        html.contains("Modelwrite does not perform STPA"),
        "the page must never imply it performed the analysis, got:\n{}",
        html
    );

    // Check 1: one control action, missing two of the four types, named.
    assert!(
        html.contains("Unanalysed control actions (1)"),
        "got:\n{}",
        html
    );
    assert!(
        html.contains("ca-discharge") && html.contains("wrong-timing-or-order"),
        "the action and its missing types must be named, got:\n{}",
        html
    );

    // Check 2: one open control loop, the controller named.
    assert!(
        html.contains("Control loops with no feedback (1)"),
        "got:\n{}",
        html
    );
    assert!(
        html.contains("controller"),
        "the controller must be named, got:\n{}",
        html
    );

    // Check 3: one hazard with no constraint, one constraint reaching no element.
    assert!(
        html.contains("Hazards with no constraint (1) · constraints reaching no element (1)"),
        "got:\n{}",
        html
    );
    assert!(
        html.contains("haz-1") && html.contains("sc-1"),
        "hazard and constraint must be named, got:\n{}",
        html
    );

    // Check 4: one UCA with no scenario.
    assert!(
        html.contains("UCAs with no loss scenario (1)"),
        "got:\n{}",
        html
    );
    assert!(html.contains("uca-p"), "got:\n{}", html);

    // The four document checks plus the two check-3 halves are five findings.
    assert!(html.contains("5 findings found."), "got:\n{}", html);

    // Check 5: one commit on the branch.
    assert!(
        html.contains("Trend across baselines (1)"),
        "got:\n{}",
        html
    );
}

#[tokio::test]
async fn the_stpa_page_is_silent_on_the_correct_model() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    router
        .clone()
        .oneshot(post("/projects", serde_json::json!({ "name": "fs" })))
        .await
        .unwrap();
    commit_stpa_fixture(&router, "fs", "fire-suppression-correct.json", "correct").await;

    let response = router
        .clone()
        .oneshot(get("/ui/projects/fs/stpa"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;

    assert!(
        html.contains("Unanalysed control actions (0)"),
        "got:\n{}",
        html
    );
    assert!(
        html.contains("Control loops with no feedback (0)"),
        "got:\n{}",
        html
    );
    assert!(
        html.contains("Hazards with no constraint (0) · constraints reaching no element (0)"),
        "got:\n{}",
        html
    );
    assert!(
        html.contains("UCAs with no loss scenario (0)"),
        "got:\n{}",
        html
    );
    assert!(
        html.contains("Complete: no missing UCA types"),
        "a complete analysis must read as complete, got:\n{}",
        html
    );
}

#[tokio::test]
async fn the_stpa_trend_tracks_the_counts_across_commits() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    router
        .clone()
        .oneshot(post("/projects", serde_json::json!({ "name": "fs" })))
        .await
        .unwrap();
    commit_stpa_fixture(
        &router,
        "fs",
        "fire-suppression-defective.json",
        "defective",
    )
    .await;
    commit_stpa_fixture(&router, "fs", "fire-suppression-correct.json", "correct").await;

    let response = router
        .clone()
        .oneshot(get("/ui/projects/fs/stpa"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;

    // Two commits on the branch, both trended, oldest first.
    assert!(
        html.contains("Trend across baselines (2)"),
        "got:\n{}",
        html
    );
    assert!(
        html.contains("defective") && html.contains("correct"),
        "both commit messages must appear in the trend, got:\n{}",
        html
    );
    // The current (correct) tip still reads as complete on the same screen.
    assert!(
        html.contains("Complete: no missing UCA types"),
        "the tip commit's verdict must render, got:\n{}",
        html
    );
}

#[tokio::test]
async fn the_control_structure_view_renders_controllers_and_feedback() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    router
        .clone()
        .oneshot(post("/projects", serde_json::json!({ "name": "fs" })))
        .await
        .unwrap();
    commit_stpa_fixture(&router, "fs", "fire-suppression-correct.json", "correct").await;

    let response = router
        .clone()
        .oneshot(get("/ui/projects/fs/diagram?view=control"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;

    // The control view is offered as a toggle and the controllers/process/feedback render.
    assert!(html.contains("Control"), "got:\n{}", html);
    assert!(
        html.contains("Fire Suppression Controller"),
        "the controller must render, got:\n{}",
        html
    );
    assert!(
        html.contains("Suppressant Discharge System"),
        "the controlled process must render, got:\n{}",
        html
    );
    assert!(
        html.contains("Discharge status"),
        "the feedback signal must render, got:\n{}",
        html
    );
}
