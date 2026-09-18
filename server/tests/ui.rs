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

use server::auth::{AuthConfig, Identity};
use server::store::{sqlite::SqliteStore, Store};
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
        "/ui/projects/coffee/compare?from=a&to=b",
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

    // The engine's coverage numbers, rendered verbatim rather than recomputed.
    assert!(
        html.contains("25 requirements: 15 covered, 10 uncovered"),
        "the coverage summary must come from the engine, got:\n{}",
        html
    );
    assert!(
        html.contains("20 satisfy, 3 refine, 1 verify, 0 allocate"),
        "the per-kind coverage counts must come from the engine"
    );

    // Ten uncovered requirements are visibly marked, fifteen are marked covered.
    assert_eq!(
        count(&html, "class=\"uncovered\""),
        10,
        "10 uncovered marked"
    );
    assert_eq!(count(&html, "class=\"covered\""), 15, "15 covered marked");
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
    assert!(
        html.contains(&format!("server-{}-{}.json", imported, corrupted)),
        "the evidence path must name the two full hashes, got:\n{}",
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
