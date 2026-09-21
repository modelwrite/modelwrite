// SPDX-License-Identifier: AGPL-3.0-or-later
//! V1 visual outputs: the SVG export and the full-screen presentation view.
//!
//! The export is generated from the COMMIT - the same deterministic server-side renderer the
//! diagram page uses - and it carries its provenance (project, commit, renderer version) in the
//! file itself. Two exports of the same revision are byte-identical. The presentation view drops
//! the workbench chrome, keeps the project/revision caption, and works with JavaScript disabled.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

use server::auth::AuthConfig;
use server::store::sqlite::SqliteStore;
use server::AppState;

fn state(dir: &std::path::Path) -> AppState {
    let store = SqliteStore::open(&dir.join("mw.db")).unwrap();
    AppState {
        store: Arc::new(store),
        evidence_dir: dir.to_path_buf(),
        auth: AuthConfig::Open,
    }
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

async fn body_bytes(response: axum::response::Response) -> Vec<u8> {
    response
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes()
        .to_vec()
}

async fn body_text(response: axum::response::Response) -> String {
    String::from_utf8(body_bytes(response).await).unwrap()
}

async fn json_body(response: axum::response::Response) -> serde_json::Value {
    serde_json::from_slice(&body_bytes(response).await).unwrap()
}

fn count(haystack: &str, needle: &str) -> usize {
    haystack.matches(needle).count()
}

/// The drawing itself, the outer transform group. The export wraps that same group with
/// provenance and a caption; the group's bytes must not change.
fn transform_group(svg: &str) -> &str {
    let start = svg
        .find("<g class='mw-transform'>")
        .expect("transform group");
    let svg_end = svg[start..].find("</svg>").expect("svg end") + start;
    let close = svg[start..svg_end].rfind("</g>").expect("group close") + start;
    &svg[start..close + "</g>".len()]
}

/// Seed the coffee-machine corpus, the same model test_support::load_okf_expected() gates.
async fn seed_coffee(router: &axum::Router) -> String {
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
            serde_json::json!({ "branch": "main", "author": "alex", "message": "import the exported model", "okf": expected }),
        ))
        .await
        .unwrap();
    assert_eq!(committed.status(), StatusCode::CREATED);
    json_body(committed).await["hash"]
        .as_str()
        .unwrap()
        .to_string()
}

#[tokio::test]
async fn the_svg_export_is_byte_identical_and_carries_its_provenance() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    let hash = seed_coffee(&router).await;

    let url = format!("/ui/projects/coffee/diagram.svg?commit={hash}&view=structure");
    let first = router.clone().oneshot(get(&url)).await.unwrap();
    assert_eq!(first.status(), StatusCode::OK);
    let content_type = first
        .headers()
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert!(
        content_type.contains("image/svg+xml"),
        "the export must be served as SVG, got {content_type}"
    );
    let disposition = first
        .headers()
        .get("content-disposition")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert!(
        disposition.contains("attachment") && disposition.contains(&hash),
        "the download filename must name the revision, got {disposition}"
    );
    let first_bytes = body_bytes(first).await;

    let second = router.clone().oneshot(get(&url)).await.unwrap();
    assert_eq!(second.status(), StatusCode::OK);
    let second_bytes = body_bytes(second).await;

    // THE PROOF: the same revision exports BYTE-IDENTICAL SVG.
    assert_eq!(
        first_bytes, second_bytes,
        "two exports of the same revision must be byte-identical"
    );

    let svg = String::from_utf8(first_bytes).unwrap();
    assert!(
        svg.starts_with("<?xml"),
        "a downloaded SVG must be a standalone XML document, got:\n{}",
        &svg[..svg.len().min(200)]
    );
    assert!(svg.contains("viewBox="), "the export must be sized");
    assert!(
        svg.contains("<style>") && svg.contains(".mw-edge"),
        "the export must inline the stylesheet it needs"
    );

    // Provenance, on its face and in the metadata.
    assert!(
        svg.contains("modelwrite-provenance:"),
        "the export must state its provenance, got:\n{}",
        &svg[..svg.len().min(400)]
    );
    assert!(svg.contains("project=coffee"), "the project must be stated");
    assert!(
        svg.contains(&format!("commit={hash}")),
        "the commit must be stated"
    );
    assert!(
        svg.contains("renderer=mw-diagram-svg/"),
        "the renderer version must be stated"
    );
    assert!(
        svg.contains("xmlns:mw=") && svg.contains("rendererVersion="),
        "the provenance must also be a metadata element"
    );
    // The revision is on the artefact's FACE, not only in metadata.
    let short = &hash[..8];
    assert!(
        svg.contains(short),
        "the visible caption must show the revision"
    );

    // No external request can be made from an exported file.
    assert!(!svg.contains("<script"), "no script");
    assert!(!svg.contains("href"), "no href");
    assert!(!svg.contains("xlink:"), "no xlink");
    assert!(!svg.contains("url("), "no url() reference");
    assert!(!svg.contains("@import"), "no @import");
    assert!(!svg.contains("<image"), "no image");
    assert!(!svg.contains("<use "), "no use element");
    assert!(
        !svg.contains("<foreignObject"),
        "no foreignObject (which could carry markup)"
    );

    // The export is the SAME DRAWING as the page: the transform group bytes are identical.
    let page = router
        .clone()
        .oneshot(get(&format!(
            "/ui/projects/coffee/diagram?commit={hash}&view=structure"
        )))
        .await
        .unwrap();
    let page_html = body_text(page).await;
    assert_eq!(
        transform_group(&page_html),
        transform_group(&svg),
        "the export must be generated by the same renderer as the page"
    );
}

#[tokio::test]
async fn the_svg_export_is_generated_from_the_commit_not_the_branch_tip() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    router
        .clone()
        .oneshot(post("/projects", serde_json::json!({ "name": "drift" })))
        .await
        .unwrap();

    let model = |node: &str| {
        serde_json::json!({
            "project": "drift",
            "exportedAt": "2026-09-17T00:00:00Z",
            "summary": {},
            "stateMachine": { "name": "sm", "regions": [] },
            "requirements": [],
            "graph": { "nodes": [{ "id": node, "kind": "block", "name": node }], "edges": [] }
        })
    };
    let first = router
        .clone()
        .oneshot(post(
            "/projects/drift/commits",
            serde_json::json!({ "branch": "main", "author": "alex", "message": "first", "okf": model("first_only") }),
        ))
        .await
        .unwrap();
    let first_hash = json_body(first).await["hash"].as_str().unwrap().to_string();
    let second = router
        .clone()
        .oneshot(post(
            "/projects/drift/commits",
            serde_json::json!({ "branch": "main", "author": "alex", "message": "second", "okf": model("second_only") }),
        ))
        .await
        .unwrap();
    assert_eq!(second.status(), StatusCode::CREATED);

    // Exporting the FIRST commit, after the branch tip has moved on, must show the first model.
    let past = router
        .clone()
        .oneshot(get(&format!(
            "/ui/projects/drift/diagram.svg?commit={first_hash}"
        )))
        .await
        .unwrap();
    assert_eq!(past.status(), StatusCode::OK);
    let past_svg = body_text(past).await;
    assert!(
        past_svg.contains("data-mw-id='first_only'"),
        "the export must show the commit it was asked for"
    );
    assert!(
        !past_svg.contains("second_only"),
        "the export must not show the branch tip"
    );
    assert!(past_svg.contains(&format!("commit={first_hash}")));

    // The branch address resolves to the tip, and shows only the tip's model.
    let tip = router
        .clone()
        .oneshot(get("/ui/projects/drift/diagram.svg?branch=main"))
        .await
        .unwrap();
    let tip_svg = body_text(tip).await;
    assert!(tip_svg.contains("data-mw-id='second_only'"));
    assert!(!tip_svg.contains("first_only"));
}

#[tokio::test]
async fn the_presentation_view_is_chrome_free_and_shareable_and_needs_no_javascript() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    let hash = seed_coffee(&router).await;

    let url = format!("/ui/projects/coffee/present?commit={hash}&view=structure");
    let response = router.clone().oneshot(get(&url)).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;

    // The diagram itself is there, at projector scale, self-styled.
    assert!(
        html.contains("<svg") && html.contains("mw-diagram-svg"),
        "the presentation view must render the diagram, got:\n{}",
        &html[..html.len().min(400)]
    );
    assert!(
        html.contains("viewBox="),
        "the diagram must be sized for its viewport"
    );
    // The chrome is gone: no workbench header, navigator, context bar or enhancement script.
    assert!(
        !html.contains("site-header"),
        "the workbench header must be gone"
    );
    assert!(
        !html.contains("class=\"rail\""),
        "the navigator must be gone"
    );
    assert!(
        !html.contains("context-bar"),
        "the context bar must be gone"
    );
    assert!(
        !html.contains("<script"),
        "the presentation view must work with JavaScript disabled"
    );
    // The project and revision stay on the page, and the view is in the URL.
    assert!(html.contains("coffee"), "the project must be captioned");
    assert!(
        html.contains(&hash[..8]),
        "the revision must be captioned, got:\n{}",
        html
    );
    assert!(
        html.contains("view=structure"),
        "the view must be carried in the shareable links"
    );
    // The download is one click from the presentation view, at the same revision.
    assert!(
        html.contains(&format!(
            "/ui/projects/coffee/diagram.svg?commit={hash}&amp;view=structure"
        )),
        "the presentation view must link the SVG download of the same revision"
    );

    // An unknown view degrades to the structure view rather than failing.
    let fallback = router
        .clone()
        .oneshot(get(&format!(
            "/ui/projects/coffee/present?commit={hash}&view=nonsense"
        )))
        .await
        .unwrap();
    assert_eq!(fallback.status(), StatusCode::OK);
    assert!(body_text(fallback).await.contains("mw-diagram-svg"));
}

#[tokio::test]
async fn the_diagram_page_offers_the_export_and_the_presentation_view() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    let hash = seed_coffee(&router).await;

    let page = router
        .clone()
        .oneshot(get(&format!("/ui/projects/coffee/diagram?commit={hash}")))
        .await
        .unwrap();
    assert_eq!(page.status(), StatusCode::OK);
    let html = body_text(page).await;
    assert!(
        html.contains(&format!(
            "/ui/projects/coffee/diagram.svg?commit={hash}&amp;view=structure"
        )),
        "the diagram page must link the SVG download, got:\n{}",
        html
    );
    assert!(
        html.contains(&format!(
            "/ui/projects/coffee/present?commit={hash}&amp;view=structure"
        )),
        "the diagram page must link the presentation view"
    );

    // The export route refuses an unknown project and an unknown commit exactly like the page.
    assert_eq!(
        router
            .clone()
            .oneshot(get("/ui/projects/nope/diagram.svg"))
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        router
            .clone()
            .oneshot(get("/ui/projects/coffee/diagram.svg?commit=deadbeef"))
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );

    // A model with no graph has no diagram to export.
    router
        .clone()
        .oneshot(post("/projects", serde_json::json!({ "name": "plain" })))
        .await
        .unwrap();
    router
        .clone()
        .oneshot(post(
            "/projects/plain/commits",
            serde_json::json!({ "branch": "main", "author": "alex", "message": "no graph", "okf": {
                "project": "plain",
                "exportedAt": "2026-09-17T00:00:00Z",
                "summary": {},
                "stateMachine": { "name": "sm", "regions": [] },
                "requirements": []
            } }),
        ))
        .await
        .unwrap();
    assert_eq!(
        router
            .oneshot(get("/ui/projects/plain/diagram.svg"))
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
    // The unused helper is exercised deliberately so the suite stays warning-free.
    assert_eq!(count("aa", "a"), 2);
}

#[tokio::test]
async fn the_process_view_exports_with_its_own_provenance() {
    // The export must name which view it drew, not just which revision: a process picture and a
    // structure picture of the same commit are different artefacts.
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    router
        .clone()
        .oneshot(post("/projects", serde_json::json!({ "name": "flow" })))
        .await
        .unwrap();
    let committed = router
        .clone()
        .oneshot(post(
            "/projects/flow/commits",
            serde_json::json!({ "branch": "main", "author": "alex", "message": "flow", "okf": {
                "project": "flow",
                "exportedAt": "2026-09-17T00:00:00Z",
                "summary": {},
                "stateMachine": { "name": "sm", "regions": [] },
                "requirements": [],
                "graph": {
                    "nodes": [
                        { "id": "a1", "kind": "activity", "name": "Brew" },
                        { "id": "a2", "kind": "activity", "name": "Serve" }
                    ],
                    "edges": [{ "source": "a1", "target": "a2", "kind": "triggers", "label": "" }]
                }
            } }),
        ))
        .await
        .unwrap();
    assert_eq!(committed.status(), StatusCode::CREATED);
    let hash = json_body(committed).await["hash"]
        .as_str()
        .unwrap()
        .to_string();

    let structure = body_text(
        router
            .clone()
            .oneshot(get(&format!(
                "/ui/projects/flow/diagram.svg?commit={hash}&view=structure"
            )))
            .await
            .unwrap(),
    )
    .await;
    let process = body_text(
        router
            .clone()
            .oneshot(get(&format!(
                "/ui/projects/flow/diagram.svg?commit={hash}&view=process"
            )))
            .await
            .unwrap(),
    )
    .await;

    assert!(structure.contains("diagram=structure"));
    assert!(
        process.contains("diagram=process"),
        "the export must name the view it drew, got:\n{}",
        &process[..process.len().min(500)]
    );
    assert!(
        process.contains("data-mw-id='a1'") && process.contains("data-mw-id='a2'"),
        "the process drawing must contain both activities"
    );
    assert_ne!(
        structure, process,
        "the process view is a different drawing from the structure view"
    );
}

#[tokio::test]
async fn the_control_view_exports_with_its_own_provenance() {
    // The control-structure view is the third diagram type. It must export with real control
    // nodes, naming itself, just like the other two.
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    router
        .clone()
        .oneshot(post("/projects", serde_json::json!({ "name": "fs" })))
        .await
        .unwrap();
    let text = std::fs::read_to_string(
        test_support::repo_root()
            .join("sample/stpa")
            .join("fire-suppression-correct.json"),
    )
    .expect("the STPA fixture must exist");
    let model: serde_json::Value = serde_json::from_str(&text).unwrap();
    let committed = router
        .clone()
        .oneshot(post(
            "/projects/fs/commits",
            serde_json::json!({ "branch": "main", "author": "alex", "message": "control", "okf": model }),
        ))
        .await
        .unwrap();
    assert_eq!(committed.status(), StatusCode::CREATED);
    let hash = json_body(committed).await["hash"]
        .as_str()
        .unwrap()
        .to_string();

    let control = body_text(
        router
            .clone()
            .oneshot(get(&format!(
                "/ui/projects/fs/diagram.svg?commit={hash}&view=control"
            )))
            .await
            .unwrap(),
    )
    .await;
    assert!(
        control.contains("diagram=control"),
        "the export must name the view it drew"
    );
    assert!(
        control.contains("Fire Suppression Controller"),
        "the control drawing must contain the controller"
    );
}
