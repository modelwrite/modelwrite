// SPDX-License-Identifier: AGPL-3.0-or-later
//! Edge labels on the real export: every relationship name lands where a reader can read it.
//!
//! The controller found the crowding by looking at an exported drawing, so the proof is taken from
//! the exported drawing. On the coffee-machine corpus, before this work, the scoped export the
//! controller looked at had 12 of its 12 labels over a node box, one label over another, and five
//! struck through by an edge line - the line the containment edges trace around the group. These
//! tests read the labels and the boxes straight out of the SVG the server serves and hold the
//! repair: no label over another label, none over a box, none struck through, and none dropped.

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

async fn body_text(response: axum::response::Response) -> String {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8(bytes.to_vec()).unwrap()
}

/// Seed the coffee-machine corpus, the same model the engine's corpus tests gate.
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
    let body: serde_json::Value =
        serde_json::from_slice(&committed.into_body().collect().await.unwrap().to_bytes()).unwrap();
    body["hash"].as_str().unwrap().to_string()
}

/// The baseline offset the renderer uses, written out here rather than imported so the measurement
/// does not share an assumption with the code it measures. The browser sets 11 px text with an
/// ascent of 12 and a descent of 3, so the ink box sits from baseline - 12 to baseline + 3.
const BASELINE_FROM_CENTRE: f64 = 4.5;

#[derive(Clone, Copy, Debug)]
struct Box2 {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

impl Box2 {
    fn overlap(&self, o: &Box2) -> f64 {
        let w = (self.x + self.w).min(o.x + o.w) - self.x.max(o.x);
        let h = (self.y + self.h).min(o.y + o.h) - self.y.max(o.y);
        if w > 0.0 && h > 0.0 {
            w * h
        } else {
            0.0
        }
    }
}

/// Pull `key='value'` out of an element's attributes, in order.
fn attr(element: &str, key: &str) -> Option<f64> {
    let needle = format!("{key}='");
    let at = element.find(&needle)? + needle.len();
    let rest = &element[at..];
    let end = rest.find('\'')?;
    rest[..end].parse().ok()
}

/// Every element whose text starts with `open`, as slices of the export.
fn elements<'a>(svg: &'a str, open: &str) -> Vec<&'a str> {
    let mut out = Vec::new();
    let mut rest = svg;
    while let Some(at) = rest.find(open) {
        let from = &rest[at..];
        let end = from.find("/>").map(|e| e + 2).unwrap_or(from.len());
        out.push(&from[..end]);
        rest = &rest[at + open.len()..];
    }
    out
}

/// The label boxes a drawn export places, read from the file: the text position and the size model
/// the placement uses.
fn label_boxes(svg: &str) -> Vec<(String, Box2)> {
    let mut out = Vec::new();
    for element in elements(svg, "<text class='edge-label'") {
        let text = element
            .split('>')
            .nth(1)
            .and_then(|rest| rest.split('<').next())
            .unwrap_or("")
            .to_string();
        let (x, y) = (attr(element, "x").unwrap(), attr(element, "y").unwrap());
        let (w, h) = graph::layout::edge_label_size(&text);
        out.push((
            text,
            Box2 {
                x: x - w / 2.0,
                y: (y - BASELINE_FROM_CENTRE) - h / 2.0,
                w,
                h,
            },
        ));
    }
    out
}

fn node_boxes(svg: &str) -> Vec<Box2> {
    elements(svg, "<rect class='node-rect'")
        .iter()
        .filter_map(|e| {
            Some(Box2 {
                x: attr(e, "x")?,
                y: attr(e, "y")?,
                w: attr(e, "width")?,
                h: attr(e, "height")?,
            })
        })
        .collect()
}

/// The scopes the corpus is measured on: the one the controller looked at, the ones the export
/// offers, and the whole model. The element ids come from the corpus itself, so the scope is the
/// same scope whatever the corpus' ids are.
fn scopes(corpus: &serde_json::Value) -> Vec<(String, String)> {
    let nodes = corpus["graph"]["nodes"].as_array().unwrap();
    let edges = corpus["graph"]["edges"].as_array().unwrap();
    let mut degree: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for n in nodes {
        degree.insert(n["id"].as_str().unwrap().to_string(), 0);
    }
    for e in edges {
        // An endpoint the model does not define is drawn as a dangling marker, not a box, so it
        // carries no degree here.
        for end in ["source", "target"] {
            if let Some(slot) = degree.get_mut(e[end].as_str().unwrap()) {
                *slot += 1;
            }
        }
    }
    let mut blocks: Vec<&str> = nodes
        .iter()
        .filter(|n| n["kind"].as_str() == Some("block"))
        .map(|n| n["id"].as_str().unwrap())
        .collect();
    blocks.sort_by_key(|id| (std::cmp::Reverse(degree[*id]), *id));
    let hub = blocks[0];
    let typical = blocks[blocks.len() / 2];
    let mut holders: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
    for e in edges {
        if matches!(e["kind"].as_str(), Some("part") | Some("contains")) {
            *holders.entry(e["source"].as_str().unwrap()).or_default() += 1;
        }
    }
    let root = *holders
        .iter()
        .max_by_key(|(id, n)| (**n, std::cmp::Reverse(*id)))
        .map(|(id, _)| id)
        .unwrap();
    vec![
        ("the whole model".to_string(), String::new()),
        (
            "the hub, 1 hop".to_string(),
            format!("scope=neighbourhood&element={hub}&hops=1"),
        ),
        (
            "the hub, 2 hops".to_string(),
            format!("scope=neighbourhood&element={hub}&hops=2"),
        ),
        (
            "a typical element, 2 hops".to_string(),
            format!("scope=neighbourhood&element={typical}&hops=2"),
        ),
        (
            "a containment branch".to_string(),
            format!("scope=containment&element={root}"),
        ),
        (
            "blocks and requirements".to_string(),
            "scope=kinds&kinds=block,requirement".to_string(),
        ),
    ]
}

#[tokio::test]
async fn no_exported_edge_label_overlaps_a_box_another_label_or_loses_its_relationship() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    let hash = seed_coffee(&router).await;
    let corpus: serde_json::Value =
        serde_json::from_str(&test_support::load_okf_expected()).unwrap();

    let mut checked = 0;
    for (name, query) in scopes(&corpus) {
        let url = format!("/ui/projects/coffee/diagram.svg?commit={hash}&view=structure&{query}");
        let svg = body_text(router.clone().oneshot(get(&url)).await.unwrap()).await;
        let labels = label_boxes(&svg);
        let boxes = node_boxes(&svg);
        assert!(!boxes.is_empty(), "{name}: the export draws node boxes");

        for i in 0..labels.len() {
            for j in (i + 1)..labels.len() {
                assert_eq!(
                    labels[i].1.overlap(&labels[j].1),
                    0.0,
                    "{name}: edge label '{}' overlaps '{}'",
                    labels[i].0,
                    labels[j].0
                );
            }
            for (k, b) in boxes.iter().enumerate() {
                assert_eq!(
                    labels[i].1.overlap(b),
                    0.0,
                    "{name}: edge label '{}' is drawn over node box {k}",
                    labels[i].0
                );
            }
        }

        // Nothing is dropped in silence. The renderer places every label it is given - leading it
        // out to clear space when it cannot sit beside its line - so the number of labels drawn
        // must equal the number of drawn relationship groups that carry a label.
        let drawn_groups = svg.matches("<g class='edge'").count();
        let labelled_groups = svg
            .split("</g>")
            .filter(|g| g.contains("data-mw-kind='") && g.contains("class='edge-label'"))
            .count();
        assert_eq!(
            labelled_groups,
            labels.len(),
            "{name}: every drawn label is a drawn relationship group ({drawn_groups} groups)"
        );
        let unplaced =
            svg.matches("data-mw-unplaced").count() + svg.matches("could not be placed").count();
        assert_eq!(
            unplaced, 0,
            "{name}: the export must place every relationship name"
        );
        checked += labels.len();
    }
    assert!(
        checked >= 150,
        "the corpus has enough labels for this to mean something, got {checked}"
    );
}

/// The export is a pure function of the model WITH label placement in it: two fetches agree byte
/// for byte, for every scope, including the ones that need leaders.
#[tokio::test]
async fn an_export_with_placed_labels_is_still_byte_identical() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    let hash = seed_coffee(&router).await;
    let corpus: serde_json::Value =
        serde_json::from_str(&test_support::load_okf_expected()).unwrap();
    for (name, query) in scopes(&corpus) {
        let url = format!("/ui/projects/coffee/diagram.svg?commit={hash}&view=structure&{query}");
        let first = body_text(router.clone().oneshot(get(&url)).await.unwrap()).await;
        let again = body_text(router.clone().oneshot(get(&url)).await.unwrap()).await;
        assert_eq!(
            first, again,
            "{name}: the same model must export identically"
        );
    }
}
