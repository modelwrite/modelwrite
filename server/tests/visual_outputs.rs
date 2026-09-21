// SPDX-License-Identifier: AGPL-3.0-or-later
//! V1 visual outputs: the SVG export and the full-screen presentation view.
//!
//! The export is generated from the COMMIT - the same deterministic server-side renderer the
//! diagram page uses - and it carries its provenance (project, commit, renderer version) in the
//! file itself. Two exports of the same revision are byte-identical. The presentation view drops
//! the workbench chrome, keeps the project/revision caption, and works with JavaScript disabled.

use std::collections::BTreeSet;
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

// ---------------------------------------------------------------------------
// Scoping, and the palette. Two defects the controller found by LOOKING at a rendered export:
// the artefact wore the old blue brand the pages had retired, and at model scale it was a
// hairball - ninety-nine labelled boxes whose labels land at single-digit pixels on a slide.
// ---------------------------------------------------------------------------

/// The coffee corpus, pinned: 99 elements, 165 relationships, 67 of them blocks or requirements.
const COFFEE_ELEMENTS: usize = 99;
const COFFEE_RELATIONSHIPS: usize = 165;
const COFFEE_BLOCKS: usize = 42;
const COFFEE_REQUIREMENTS: usize = 25;

/// The palette literals the instrument rebrand retired. The export used to hardcode these in a
/// stylesheet of its own, which is exactly how a downloaded artefact came to wear the old blue
/// brand after every page had been rebranded.
const RETIRED_IN_STYLESHEET: &[&str] = &[
    "#2563eb", "#1d4ed8", "#8b949e", "#59636e", "#1f2328", "#1f2937", "#8250df", "#0550ae",
    "#9a6700", "#1a7f37", "#0a7ea4", "#bc4c00", "#bf3989", "#cf222e", "#d1d9e0", "#f6f8fa",
    "#ffebe9", "#fff3cd",
];

/// The retired colours that must not survive anywhere in the file: the brand blue, the old ink
/// and the old hairlines. These painted the product and no longer paint anything.
const RETIRED_ANYWHERE: &[&str] = &[
    "#2563eb", "#1d4ed8", "#8b949e", "#59636e", "#1f2328", "#8250df", "#0550ae", "#9a6700",
    "#1a7f37", "#0a7ea4", "#bc4c00", "#bf3989", "#cf222e", "#d1d9e0", "#f6f8fa", "#ffebe9",
];

/// The model's graph, read the way a reviewer would: from the corpus itself, so what a scope
/// SHOULD contain is computed independently of the code that filters it.
fn coffee_graph() -> serde_json::Value {
    let okf: serde_json::Value = serde_json::from_str(&test_support::load_okf_expected()).unwrap();
    okf["graph"].clone()
}

fn nodes_of(graph: &serde_json::Value) -> Vec<(String, String)> {
    graph["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|node| {
            (
                node["id"].as_str().unwrap().to_string(),
                node["kind"].as_str().unwrap_or_default().to_string(),
            )
        })
        .collect()
}

fn edges_of(graph: &serde_json::Value) -> Vec<(String, String, String)> {
    graph["edges"]
        .as_array()
        .unwrap()
        .iter()
        .map(|edge| {
            (
                edge["source"].as_str().unwrap().to_string(),
                edge["target"].as_str().unwrap().to_string(),
                edge["kind"].as_str().unwrap_or_default().to_string(),
            )
        })
        .collect()
}

/// Fetch an export and return it as text.
async fn export_svg(router: &axum::Router, url: &str) -> String {
    body_text(router.clone().oneshot(get(url)).await.unwrap()).await
}

/// The stylesheet an exported document carries.
fn style_block(svg: &str) -> &str {
    let start = svg
        .find("<style>")
        .expect("the export inlines a stylesheet")
        + "<style>".len();
    let end = svg[start..]
        .find("</style>")
        .expect("the stylesheet closes")
        + start;
    &svg[start..end]
}

/// The drawing's viewBox, as (width, height).
fn view_box(svg: &str) -> (f64, f64) {
    let start = svg.find("viewBox='").expect("a viewBox") + "viewBox='".len();
    let end = svg[start..].find('\'').expect("the viewBox closes") + start;
    let numbers: Vec<f64> = svg[start..end]
        .split_whitespace()
        .map(|number| number.parse().unwrap())
        .collect();
    (numbers[2], numbers[3])
}

/// The ids of the node boxes a drawing places.
fn drawn_ids(svg: &str) -> BTreeSet<String> {
    let mut ids = BTreeSet::new();
    let mut rest = svg;
    while let Some(at) = rest.find("data-mw-id='") {
        let after = &rest[at + "data-mw-id='".len()..];
        let end = after.find('\'').expect("the id attribute closes");
        ids.insert(after[..end].to_string());
        rest = &after[end..];
    }
    ids
}

/// The size a node label lands at when a drawing of this canvas is fitted into one 1920x1080
/// slide. Written out here rather than imported, so the measurement is independent of the code
/// under test - which is the only way measuring it proves anything.
fn slide_label_px(svg: &str) -> f64 {
    let (width, height) = view_box(svg);
    13.0 * (1920.0 / width).min(1080.0 / height)
}

/// The arrowhead classes a drawing emits.
fn arrow_classes(svg: &str) -> BTreeSet<String> {
    let mut classes = BTreeSet::new();
    let mut rest = svg;
    while let Some(at) = rest.find("class='arrow-") {
        let after = &rest[at + "class='".len()..];
        let end = after.find('\'').expect("the class attribute closes");
        classes.insert(after[..end].to_string());
        rest = &after[end..];
    }
    classes
}

/// The label size an export states on its own face, read back out of its caption.
fn caption_label_px(svg: &str) -> f64 {
    let at = svg
        .find("labels ")
        .expect("the caption states the label size")
        + "labels ".len();
    let rest = &svg[at..];
    let end = rest.find(" px").expect("the stated label size is in px");
    rest[..end]
        .parse()
        .expect("the stated label size is a number")
}

/// The neighbourhood of an element, as a reviewer would work it out from the corpus.
fn neighbourhood(graph: &serde_json::Value, element: &str, hops: usize) -> BTreeSet<String> {
    let mut adjacency: Vec<(String, String)> = Vec::new();
    for (source, target, _) in edges_of(graph) {
        if source != target {
            adjacency.push((source.clone(), target.clone()));
            adjacency.push((target, source));
        }
    }
    let mut seen = BTreeSet::new();
    seen.insert(element.to_string());
    for _ in 0..hops {
        let mut next: Vec<String> = Vec::new();
        for (source, target) in &adjacency {
            if seen.contains(source) && !seen.contains(target) {
                next.push(target.clone());
            }
        }
        for id in next {
            seen.insert(id);
        }
    }
    seen
}

/// The descendants of a container, through part/contains edges, as a reviewer would work them out.
fn containment(graph: &serde_json::Value, element: &str) -> BTreeSet<String> {
    let children: Vec<(String, String)> = edges_of(graph)
        .into_iter()
        .filter(|(_, _, kind)| kind == "part" || kind == "contains")
        .map(|(source, target, _)| (source, target))
        .collect();
    let mut seen = BTreeSet::new();
    seen.insert(element.to_string());
    let mut frontier = vec![element.to_string()];
    while let Some(id) = frontier.pop() {
        for (source, target) in &children {
            if *source == id && seen.insert(target.clone()) {
                frontier.push(target.clone());
            }
        }
    }
    seen
}

#[tokio::test]
async fn the_export_wears_the_workbench_palette_and_none_of_the_retired_colours() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    let hash = seed_coffee(&router).await;
    let svg = body_text(
        router
            .clone()
            .oneshot(get(&format!(
                "/ui/projects/coffee/diagram.svg?commit={hash}&view=structure"
            )))
            .await
            .unwrap(),
    )
    .await;

    // The export resolves every token from the ONE stylesheet before writing the file, so the
    // downloaded SVG needs no custom-property support and carries no palette of its own.
    let style = style_block(&svg);
    assert!(
        !style.contains("var("),
        "every design token must be resolved in the exported file, got:\n{style}"
    );
    for retired in RETIRED_IN_STYLESHEET {
        assert!(
            !style.contains(retired),
            "the export stylesheet still wears the retired colour {retired}"
        );
    }
    for retired in RETIRED_ANYWHERE {
        assert!(
            !svg.contains(retired),
            "the exported file still contains the retired colour {retired}"
        );
    }

    // The live workbench tokens are what paints it now.
    for token in [
        "#0b6e99", // --accent / --kind-block: block nodes, dependency edges, arrowheads
        "#101418", // --text: node names
        "#5f6973", // --text-3: node strokes, neutral edges, neutral arrows
        "#49535c", // --text-2: containment edges, edge labels, kind lines
        "#e6f2f7", // --kind-block-bg
        "#fbf1da", // --kind-requirement-bg
        "#f5f7f9", // --surface-1: the caption band
    ] {
        assert!(
            style.contains(token),
            "the export must paint with the shared token value {token}"
        );
    }
    // A requirement is filled with its KIND token, not with the pre-rebrand amber it still carries
    // as a presentation attribute (which is pinned by the page's own test and beats nothing: a CSS
    // declaration wins over a presentation attribute, so this rule is what actually paints).
    assert!(
        style.contains("fill: #fbf1da"),
        "the requirement fill must come from the shared token"
    );
    assert!(
        svg.contains("fill='#fff3cd'"),
        "the renderer still emits the requirement presentation attribute for the page contract"
    );

    // Every arrowhead class the renderer emits must have a rule in the stylesheet. One that does
    // not falls back to SVG's DEFAULT fill - black - which is what had happened to every
    // dependency arrowhead: its line was accent-coloured and its arrow was not, on the page and
    // in the export, because the renderer named the class arrow-dependency while the stylesheet
    // called the rule .arrow-accent. Measured, not read: the rendered export painted rgb(0, 0, 0).
    let arrows = arrow_classes(&svg);
    assert!(!arrows.is_empty(), "the corpus draws arrowheads");
    for class in &arrows {
        assert!(
            style.contains(&format!(".{class} ")),
            "the stylesheet must style .{class}, or SVG's default fill paints it black"
        );
    }
    assert!(
        arrows.contains("arrow-accent"),
        "a dependency arrowhead must wear the accent, got {arrows:?}"
    );
    assert!(
        !arrows.contains("arrow-dependency"),
        "the renderer must not emit an arrow class no rule styles"
    );
}

#[tokio::test]
async fn a_scoped_export_is_byte_identical_and_names_its_scope() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    let hash = seed_coffee(&router).await;
    let url = |scope: &str| {
        format!("/ui/projects/coffee/diagram.svg?commit={hash}&view=structure&{scope}")
    };

    // The whole model is the default, and asking for it by name is the same bytes.
    let default_bytes = body_bytes(router.clone().oneshot(get(&url(""))).await.unwrap()).await;
    let explicit_bytes = body_bytes(
        router
            .clone()
            .oneshot(get(&url("scope=full")))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(
        default_bytes, explicit_bytes,
        "the default scope IS the full scope"
    );
    let full = String::from_utf8(default_bytes).unwrap();
    assert_eq!(count(&full, "class='node'"), COFFEE_ELEMENTS);
    assert_eq!(count(&full, "class='edge'"), COFFEE_RELATIONSHIPS);
    assert!(
        full.contains(&format!(
            "scope=full elements={COFFEE_ELEMENTS}/{COFFEE_ELEMENTS}"
        )),
        "the full export must state that it is the whole model"
    );
    assert!(full.contains("the whole model"));

    // The SAME scope, two spellings of the kind list: one scope, therefore one drawing, byte for
    // byte. Determinism has to survive scoping, and two addresses for one picture that differed
    // would make the drawing impossible to diff or cache.
    let scoped_url = url("scope=kinds&kinds=block,requirement");
    let reordered_url = url("scope=kinds&kinds=requirement,block");
    let first = body_bytes(router.clone().oneshot(get(&scoped_url)).await.unwrap()).await;
    let second = body_bytes(router.clone().oneshot(get(&scoped_url)).await.unwrap()).await;
    let reordered = body_bytes(router.clone().oneshot(get(&reordered_url)).await.unwrap()).await;
    assert_eq!(
        first, second,
        "two exports of one scope must be byte-identical"
    );
    assert_eq!(
        first, reordered,
        "one scope has one spelling and one drawing"
    );
    let scoped = String::from_utf8(first).unwrap();

    // It draws exactly the blocks and requirements, and the relationships among them - computed
    // here from the corpus, independently of the filter under test.
    let graph = coffee_graph();
    let nodes = nodes_of(&graph);
    let wanted: BTreeSet<&str> = nodes
        .iter()
        .filter(|(_, kind)| kind.as_str() == "block" || kind.as_str() == "requirement")
        .map(|(id, _)| id.as_str())
        .collect();
    let expected_ids: BTreeSet<String> = wanted.iter().map(|id| id.to_string()).collect();
    let expected_edges = edges_of(&graph)
        .iter()
        .filter(|(source, target, _)| {
            wanted.contains(source.as_str()) && wanted.contains(target.as_str())
        })
        .count();
    assert_eq!(wanted.len(), COFFEE_BLOCKS + COFFEE_REQUIREMENTS);
    assert_eq!(
        drawn_ids(&scoped),
        expected_ids,
        "the scoped drawing must place exactly the selected elements"
    );
    assert_eq!(
        count(&scoped, "class='edge'"),
        expected_edges,
        "the scoped drawing must keep exactly the relationships among them"
    );
    assert_eq!(
        count(&scoped, "class='dangling'"),
        0,
        "a scope must not invent unresolved markers for the elements it left out"
    );
    for absent in [
        "signal",
        "activity",
        "state",
        "usecase",
        "actor",
        "stateMachine",
        "interface",
    ] {
        assert!(
            !scoped.contains(&format!("data-mw-kind='{absent}'")),
            "a block+requirement scope must not draw a {absent}"
        );
    }

    // PROVENANCE: the file names the scope and the counts, in the comment, in the metadata and on
    // its face, so a filtered picture can never be mistaken for the whole model.
    assert!(
        scoped.contains("scope=kinds:block,requirement"),
        "the provenance line must name the scope"
    );
    assert!(
        scoped.contains(&format!(
            "elements={}/{COFFEE_ELEMENTS} relationships={expected_edges}/{COFFEE_RELATIONSHIPS}",
            expected_ids.len()
        )),
        "the provenance line must state how much of the model this is"
    );
    assert!(
        scoped.contains("scope=\"kinds:block,requirement\""),
        "the metadata element must carry the scope"
    );
    assert!(
        scoped.contains("a subset of the model"),
        "the caption must say on the picture's face that it is a subset"
    );
    assert!(
        scoped.contains("only block + requirement elements"),
        "the caption must say which subset"
    );

    // The download is named for its scope, so two files in one folder cannot be confused.
    let disposition = router
        .clone()
        .oneshot(get(&scoped_url))
        .await
        .unwrap()
        .headers()
        .get("content-disposition")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert!(
        disposition.contains("structure-kinds_block_requirement.svg"),
        "the filename must name the scope, got {disposition}"
    );

    // A scope the model cannot satisfy is answered, not drawn empty.
    assert_eq!(
        router
            .clone()
            .oneshot(get(&url("scope=kinds&kinds=bogus")))
            .await
            .unwrap()
            .status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        router
            .clone()
            .oneshot(get(&url("scope=neighbourhood&element=no_such_element")))
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        router
            .clone()
            .oneshot(get(&url("scope=nonsense")))
            .await
            .unwrap()
            .status(),
        StatusCode::BAD_REQUEST
    );
}

#[tokio::test]
async fn a_neighbourhood_export_draws_the_element_and_nothing_beyond_the_radius() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    let hash = seed_coffee(&router).await;
    let graph = coffee_graph();
    let edges = edges_of(&graph);

    // The element a review would pick: the most connected block, chosen deterministically so the
    // test says the same thing on every machine.
    let mut degree: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for (source, target, _) in &edges {
        *degree.entry(source.clone()).or_default() += 1;
        *degree.entry(target.clone()).or_default() += 1;
    }
    let element = nodes_of(&graph)
        .iter()
        .filter(|(_, kind)| kind.as_str() == "block")
        .max_by_key(|(id, _)| (degree.get(id).copied().unwrap_or_default(), id.clone()))
        .map(|(id, _)| id.to_string())
        .unwrap();

    let url = |scope: &str| {
        format!("/ui/projects/coffee/diagram.svg?commit={hash}&view=structure&{scope}")
    };
    let one = export_svg(
        &router,
        &url(&format!("scope=neighbourhood&element={element}&hops=1")),
    )
    .await;
    let two = export_svg(
        &router,
        &url(&format!("scope=neighbourhood&element={element}&hops=2")),
    )
    .await;

    assert_eq!(
        drawn_ids(&one),
        neighbourhood(&graph, &element, 1),
        "a 1-hop drawing is the element and its immediate neighbours"
    );
    assert_eq!(
        drawn_ids(&two),
        neighbourhood(&graph, &element, 2),
        "a 2-hop drawing is the element and everything within two relationships"
    );
    assert!(
        drawn_ids(&one).len() < drawn_ids(&two).len(),
        "a wider radius must draw more of the model"
    );
    assert!(
        two.contains(&format!("scope=neighbourhood:{element}:2")),
        "the provenance must name the element and the radius"
    );
    assert!(
        two.contains(&format!(
            "elements={}/{COFFEE_ELEMENTS}",
            drawn_ids(&two).len()
        )),
        "the provenance must state how much of the model the neighbourhood is"
    );
    assert!(
        one.contains(&element) && two.contains(&element),
        "the element asked for must be drawn"
    );
    // The default radius is applied when none is given, and it is capped rather than honoured
    // blindly: a radius of 99 is still a drawing of the whole model, not a hang.
    let default_hops = export_svg(
        &router,
        &url(&format!("scope=neighbourhood&element={element}")),
    )
    .await;
    assert_eq!(drawn_ids(&default_hops), drawn_ids(&one));
    let capped = export_svg(
        &router,
        &url(&format!("scope=neighbourhood&element={element}&hops=99")),
    )
    .await;
    assert_eq!(
        drawn_ids(&capped),
        neighbourhood(&graph, &element, 3),
        "the radius is capped at the documented maximum"
    );
    assert_eq!(
        router
            .clone()
            .oneshot(get(&url(&format!(
                "scope=neighbourhood&element={element}&hops=two"
            ))))
            .await
            .unwrap()
            .status(),
        StatusCode::BAD_REQUEST
    );
}

#[tokio::test]
async fn a_containment_export_draws_a_branch_of_the_model() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    let hash = seed_coffee(&router).await;
    let graph = coffee_graph();

    // The branch a reviewer would pick: the container that holds the most, chosen deterministically.
    let mut roots: Vec<String> = edges_of(&graph)
        .into_iter()
        .filter(|(_, _, kind)| kind == "part" || kind == "contains")
        .map(|(source, _, _)| source)
        .collect();
    roots.sort();
    roots.dedup();
    let element = roots
        .iter()
        .max_by_key(|root| (containment(&graph, root.as_str()).len(), (*root).clone()))
        .cloned()
        .unwrap();
    let expected = containment(&graph, &element);
    assert!(
        expected.len() > 1 && expected.len() < COFFEE_ELEMENTS,
        "the corpus must have a real branch to draw"
    );

    let svg = body_text(
        router
            .clone()
            .oneshot(get(&format!(
                "/ui/projects/coffee/diagram.svg?commit={hash}&view=structure&scope=containment&element={element}"
            )))
            .await
            .unwrap(),
    )
    .await;

    assert_eq!(
        drawn_ids(&svg),
        expected,
        "a containment scope is the container and its descendants, and nothing else"
    );
    assert!(
        svg.contains(&format!("scope=containment:{element}")),
        "the provenance must name the branch"
    );
    assert!(
        svg.contains("and its containment descendants"),
        "the caption must say what the branch is"
    );
    assert!(
        svg.contains("a subset of the model"),
        "a branch is not the whole model and must say so"
    );
}

#[tokio::test]
async fn the_diagram_page_offers_the_scopes_and_says_what_each_one_gives() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    let hash = seed_coffee(&router).await;
    let html = body_text(
        router
            .clone()
            .oneshot(get(&format!("/ui/projects/coffee/diagram?commit={hash}")))
            .await
            .unwrap(),
    )
    .await;

    // The full export is still one click, and it is named as the complete one.
    assert!(
        html.contains(&format!(
            "/ui/projects/coffee/diagram.svg?commit={hash}&amp;view=structure&amp;scope=full"
        )),
        "the page must offer the whole model, explicitly, got:\n{html}"
    );
    assert!(html.contains("the whole model (complete)"));

    // By kind, and by the kinds a review asks for together.
    assert!(html.contains("scope=kinds&amp;kinds=block"));
    assert!(html.contains("scope=kinds&amp;kinds=block,requirement"));
    assert!(html.contains("only block + requirement elements"));

    // A branch of containment, one click per branch.
    assert!(html.contains("scope=containment&amp;element="));

    // An element and its neighbourhood, as a form that works with JavaScript disabled.
    assert!(html.contains("action=\"/ui/projects/coffee/diagram.svg\""));
    assert!(html.contains("name=\"scope\" value=\"neighbourhood\""));
    assert!(html.contains("select name=\"element\""));
    assert!(html.contains("select name=\"hops\""));
    assert!(html.contains("Download the neighbourhood"));

    // What each option gives you: the counts, and the label size it lands at on a slide. That
    // measure is the whole point of offering the choice, so it is on the page and not only in a
    // report.
    assert!(
        html.contains("of 99 elements") || html.contains("/99 elements"),
        "each option must state how much of the model it draws"
    );
    assert!(
        html.contains("px at slide scale"),
        "each option must state the label size it lands at on a slide"
    );
    assert!(
        html.contains("1920×1080"),
        "the page must say what slide scale means"
    );
    assert!(
        html.contains("still too small to read"),
        "the page must say honestly which options are unreadable"
    );
}

#[tokio::test]
async fn a_scoped_export_is_readable_at_slide_scale_where_the_full_one_is_not() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    let hash = seed_coffee(&router).await;
    let graph = coffee_graph();
    let edges = edges_of(&graph);

    let url = |scope: &str| {
        format!("/ui/projects/coffee/diagram.svg?commit={hash}&view=structure&{scope}")
    };

    // The element a review would centre on: the most connected block.
    let mut degree: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for (source, target, _) in &edges {
        *degree.entry(source.clone()).or_default() += 1;
        *degree.entry(target.clone()).or_default() += 1;
    }
    let element = nodes_of(&graph)
        .iter()
        .filter(|(_, kind)| kind.as_str() == "block")
        .max_by_key(|(id, _)| (degree.get(id).copied().unwrap_or_default(), id.clone()))
        .map(|(id, _)| id.to_string())
        .unwrap();

    let full = export_svg(&router, &url("")).await;
    let block_and_requirement =
        export_svg(&router, &url("scope=kinds&kinds=block,requirement")).await;
    let requirement = export_svg(&router, &url("scope=kinds&kinds=requirement")).await;
    let one_hop = export_svg(
        &router,
        &url(&format!("scope=neighbourhood&element={element}&hops=1")),
    )
    .await;
    let two_hops = export_svg(
        &router,
        &url(&format!("scope=neighbourhood&element={element}&hops=2")),
    )
    .await;
    let mut roots: Vec<String> = edges_of(&graph)
        .into_iter()
        .filter(|(_, _, kind)| kind == "part" || kind == "contains")
        .map(|(source, _, _)| source)
        .collect();
    roots.sort();
    roots.dedup();
    let root = roots
        .iter()
        .max_by_key(|root| (containment(&graph, root.as_str()).len(), (*root).clone()))
        .cloned()
        .unwrap();
    let branch = export_svg(&router, &url(&format!("scope=containment&element={root}"))).await;

    // MEASURED on this corpus at 1920x1080: the whole model lands at 6.6 px, and a
    // block+requirement scope lands at 6.4 px - NO BETTER, because the canvas HEIGHT follows how
    // deep the structure is, not how many boxes it has. Legibility comes from drawing a SMALL
    // PART of the model, not from a filtered whole of it. This is the honest finding of the
    // scoping tranche, and it is why the page measures every option instead of promising that
    // filtering helps.
    let legible = |svg: &str| slide_label_px(svg) >= 11.0;
    assert!(
        !legible(&full),
        "the whole model cannot be a slide picture, measured {:.1} px",
        slide_label_px(&full)
    );
    assert!(
        !legible(&block_and_requirement),
        "a kind filter over two thirds of the model is not a slide picture either, measured {:.1} px",
        slide_label_px(&block_and_requirement)
    );
    for (name, svg) in [
        ("one kind", &requirement),
        ("one hop of the hub", &one_hop),
        ("a containment branch", &branch),
    ] {
        assert!(
            legible(svg),
            "{name} must be readable at slide scale, measured {:.1} px",
            slide_label_px(svg)
        );
    }
    // The limit, stated rather than hidden: two hops of the model's most connected element is
    // half the model, and it is not a slide picture.
    assert!(
        !legible(&two_hops),
        "two hops of the hub is half the model, measured {:.1} px",
        slide_label_px(&two_hops)
    );

    // The artefact states its OWN readability, and the number it states is the number its own
    // geometry produces - measured here from the viewBox, independently of the caption.
    for svg in [
        &full,
        &block_and_requirement,
        &requirement,
        &one_hop,
        &two_hops,
        &branch,
    ] {
        let stated = caption_label_px(svg);
        let measured = slide_label_px(svg);
        assert!(
            (stated - measured).abs() < 0.05,
            "the caption says {stated} px but its own canvas measures {measured} px"
        );
    }
    assert!(
        full.contains("too small to read"),
        "the whole model must say on its face that it cannot be read at slide scale"
    );
    assert!(
        one_hop.contains("(readable)"),
        "a readable drawing must say so"
    );
}
