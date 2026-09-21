// SPDX-License-Identifier: AGPL-3.0-or-later
//! The front door's per-card model-health summary.
//!
//! The projects page is the page a person lands on. It answers "which of my twelve models
//! is broken?" from the card, not after opening twelve models: every card carries the NAMED
//! gap counts for its default branch's head commit, computed by the SAME detector the
//! model-health view renders. There is no score and no percentage anywhere in here, on
//! purpose - this product names gaps, it does not summarise them into one number - and the
//! distinction between "nothing to measure" (a project with no commits) and "nothing wrong"
//! (a clean model) is asserted, because zeros for a project with no model would be a lie.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

use server::auth::AuthConfig;
use server::store::sqlite::SqliteStore;
use server::AppState;

fn state(dir: &std::path::Path) -> AppState {
    AppState {
        store: Arc::new(SqliteStore::open(&dir.join("mw.db")).unwrap()),
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

/// Seed one project from the coffee-machine corpus and return the commit hash.
async fn seed_corpus(router: &axum::Router, project: &str) -> String {
    let created = router
        .clone()
        .oneshot(post("/projects", serde_json::json!({ "name": project })))
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::CREATED);
    let okf: serde_json::Value = serde_json::from_str(&test_support::load_okf_expected()).unwrap();
    let committed = router
        .clone()
        .oneshot(post(
            &format!("/projects/{project}/commits"),
            serde_json::json!({ "branch": "main", "author": "alex", "message": "seed", "okf": okf }),
        ))
        .await
        .unwrap();
    assert_eq!(committed.status(), StatusCode::CREATED);
    let body = body_text(committed).await;
    let commit: serde_json::Value = serde_json::from_str(&body).unwrap();
    commit["hash"].as_str().unwrap().to_string()
}

/// The <li> for one project on the list page: the card and its health summary.
fn card<'a>(html: &'a str, project: &str) -> &'a str {
    let needle = format!("href=\"/ui/projects/{project}\"");
    let start = html
        .find(&needle)
        .unwrap_or_else(|| panic!("no card for {project} in:\n{html}"));
    let end = html[start..]
        .find("</li>")
        .map(|offset| start + offset)
        .unwrap_or(html.len());
    &html[start..end]
}

/// One count the card publishes as a data attribute, so the test reads the number the card
/// actually carries rather than a string that happens to sit near it.
fn data_count(card: &str, name: &str) -> usize {
    let needle = format!("data-mw-{name}=\"");
    let start = card
        .find(&needle)
        .unwrap_or_else(|| panic!("no {needle} in card:\n{card}"))
        + needle.len();
    let end = start + card[start..].find('"').unwrap();
    card[start..end]
        .parse()
        .unwrap_or_else(|_| panic!("data-mw-{name} is not a number in card:\n{card}"))
}

/// The count the health view prints in a named heading, e.g. "Dangling links (2)".
fn heading_count(html: &str, heading: &str) -> usize {
    let needle = format!("{heading} (");
    let start = html
        .find(&needle)
        .unwrap_or_else(|| panic!("no {needle} in:\n{html}"))
        + needle.len();
    let end = start + html[start..].find(')').unwrap();
    html[start..end]
        .parse()
        .unwrap_or_else(|_| panic!("{heading} count is not a number in:\n{html}"))
}

/// The health-view URL the card links to.
fn health_url(card: &str) -> String {
    let marker = "/health?";
    let start = card
        .find(marker)
        .unwrap_or_else(|| panic!("no health link in card:\n{card}"));
    let open = card[..start].rfind('"').unwrap() + 1;
    card[open..].split('"').next().unwrap().to_string()
}

#[tokio::test]
async fn the_card_reports_the_same_counts_as_the_health_view() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    let hash = seed_corpus(&router, "coffee").await;

    let list = router.clone().oneshot(get("/ui")).await.unwrap();
    assert_eq!(list.status(), StatusCode::OK);
    let html = body_text(list).await;
    let card = card(&html, "coffee").to_string();

    // The card links to the commit it measured, BY HASH. Commits are immutable, so the page
    // a reviewer lands on is the page the counts were taken from.
    let url = health_url(&card);
    assert!(
        url.contains(&hash),
        "the card must link to the measured commit, got {url}"
    );

    let health = router.oneshot(get(&url)).await.unwrap();
    assert_eq!(health.status(), StatusCode::OK);
    let health_html = body_text(health).await;

    // ONE source of truth: every number on the card IS the number the health view prints.
    for (attribute, heading) in [
        ("orphaned", "Orphaned nodes"),
        ("isolated-groups", "Isolated groups"),
        ("dangling", "Dangling links"),
        ("uncovered", "Uncovered requirements"),
    ] {
        assert_eq!(
            data_count(&card, attribute),
            heading_count(&health_html, heading),
            "the card's {attribute} count must equal the health view's \"{heading}\" count"
        );
    }

    // ...and both are anchored to the corpus' measured shape, so the equality above cannot
    // be two zeroes agreeing with each other.
    assert_eq!(data_count(&card, "orphaned"), 0);
    assert_eq!(data_count(&card, "isolated-groups"), 0);
    assert_eq!(data_count(&card, "dangling"), 2);
    assert_eq!(data_count(&card, "uncovered"), 10);
}

#[tokio::test]
async fn a_card_with_gaps_names_every_count_and_links_to_the_health_view() {
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
    assert_eq!(committed.status(), StatusCode::CREATED);

    let list = router.clone().oneshot(get("/ui")).await.unwrap();
    let html = body_text(list).await;
    let card = card(&html, "coffee").to_string();

    // A card with gaps is visually distinct from a clean one...
    assert!(
        card.contains("has-gaps"),
        "a card with gaps must carry the gap treatment, got:\n{card}"
    );
    assert!(
        !card.contains("is-clean"),
        "a card with gaps must not read as clean, got:\n{card}"
    );

    // ...it names each count with a short label...
    assert!(card.contains("2 orphans"), "got:\n{card}");
    assert!(card.contains("1 isolated group (2 nodes)"), "got:\n{card}");
    assert!(card.contains("1 dangling link"), "got:\n{card}");
    assert!(card.contains("1 uncovered requirement"), "got:\n{card}");
    // ...and it does NOT summarise them into a score.
    assert!(
        !card.contains('%') && !card.to_lowercase().contains("health score"),
        "the product names gaps; it must not print a health score, got:\n{card}"
    );

    // The counts on the card are the counts the health view prints, for the same commit.
    assert_eq!(data_count(&card, "orphaned"), 2);
    assert_eq!(data_count(&card, "isolated-groups"), 1);
    assert_eq!(data_count(&card, "isolated-group-nodes"), 2);
    assert_eq!(data_count(&card, "dangling"), 1);
    assert_eq!(data_count(&card, "uncovered"), 1);

    let health = router.oneshot(get(&health_url(&card))).await.unwrap();
    assert_eq!(health.status(), StatusCode::OK);
    let health_html = body_text(health).await;
    assert_eq!(
        data_count(&card, "orphaned"),
        heading_count(&health_html, "Orphaned nodes")
    );
    assert_eq!(
        data_count(&card, "isolated-groups"),
        heading_count(&health_html, "Isolated groups")
    );
    assert_eq!(
        data_count(&card, "dangling"),
        heading_count(&health_html, "Dangling links")
    );
    assert_eq!(
        data_count(&card, "uncovered"),
        heading_count(&health_html, "Uncovered requirements")
    );
    // The group's total node count is the health view's group size, not a second reading.
    assert!(
        health_html.contains("Group 1 (2 nodes)"),
        "the health view names the group's size, got:\n{health_html}"
    );
}

#[tokio::test]
async fn a_card_with_no_gaps_reads_as_plainly_clean() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    router
        .clone()
        .oneshot(post("/projects", serde_json::json!({ "name": "coffee" })))
        .await
        .unwrap();
    // One block satisfying one requirement: one component, no orphans, no dangling links,
    // every requirement covered.
    let clean = serde_json::json!({
        "project": "coffee",
        "summary": {},
        "stateMachine": {"name": "sm", "regions": []},
        "structure": [
            {"id": "b1", "name": "Boiler Block", "kind": "block", "stereotypes": [], "attributes": [], "documentation": ""}
        ],
        "requirements": [
            {"id": "r1", "name": "Heat requirement", "kind": "requirement", "stereotypes": [], "attributes": [], "documentation": "", "reqId": "1.1", "reqText": "heats"}
        ],
        "graph": {
            "nodes": [
                {"id": "b1", "kind": "block", "name": "Boiler Block"},
                {"id": "r1", "kind": "requirement", "name": "Heat requirement"}
            ],
            "edges": [
                {"source": "b1", "target": "r1", "kind": "dependency", "label": "Satisfy"}
            ]
        }
    });
    let committed = router
        .clone()
        .oneshot(post(
            "/projects/coffee/commits",
            serde_json::json!({ "branch": "main", "author": "alex", "message": "clean", "okf": clean }),
        ))
        .await
        .unwrap();
    assert_eq!(committed.status(), StatusCode::CREATED);

    let list = router.clone().oneshot(get("/ui")).await.unwrap();
    let html = body_text(list).await;
    let card = card(&html, "coffee").to_string();

    assert!(
        card.contains("is-clean"),
        "a card with no gaps must read as clean, got:\n{card}"
    );
    assert!(
        !card.contains("has-gaps"),
        "a clean card must not wear the gap treatment, got:\n{card}"
    );
    assert!(card.contains("no gaps"), "got:\n{card}");
    assert_eq!(data_count(&card, "orphaned"), 0);
    assert_eq!(data_count(&card, "isolated-groups"), 0);
    assert_eq!(data_count(&card, "isolated-group-nodes"), 0);
    assert_eq!(data_count(&card, "dangling"), 0);
    assert_eq!(data_count(&card, "uncovered"), 0);

    // The health view it links to agrees that the model is clean.
    let health = router.oneshot(get(&health_url(&card))).await.unwrap();
    let health_html = body_text(health).await;
    assert!(
        health_html.contains("Healthy: no orphaned nodes"),
        "the health view must agree, got:\n{health_html}"
    );
}

#[tokio::test]
async fn a_project_with_no_commits_says_nothing_to_measure() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    router
        .clone()
        .oneshot(post("/projects", serde_json::json!({ "name": "empty" })))
        .await
        .unwrap();

    let list = router.oneshot(get("/ui")).await.unwrap();
    assert_eq!(list.status(), StatusCode::OK);
    let html = body_text(list).await;
    let card = card(&html, "empty");

    // "Nothing to measure" is not "nothing wrong": no zeros that would read as a clean model.
    assert!(
        card.contains("nothing to measure"),
        "a project with no commits must say so plainly, got:\n{card}"
    );
    assert!(
        !card.contains("is-clean"),
        "no commits is not a clean model, got:\n{card}"
    );
    assert!(
        !card.contains("has-gaps"),
        "no commits is not a model with gaps either, got:\n{card}"
    );
    assert!(
        !card.contains("data-mw-orphaned"),
        "a project with no model must not publish zero counts, got:\n{card}"
    );
    assert!(
        !card.contains("/health?"),
        "there is no health view to link to without a model, got:\n{card}"
    );
}

#[tokio::test]
async fn a_card_shows_the_default_branch_head_not_just_the_newest_commit() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    // The front door measures the DEFAULT BRANCH head (main), the same commit a bare
    // /health URL resolves to - not whichever branch happened to commit most recently.
    let hash = seed_corpus(&router, "coffee").await;

    let list = router.oneshot(get("/ui")).await.unwrap();
    let html = body_text(list).await;
    let card = card(&html, "coffee").to_string();
    assert!(
        health_url(&card).contains(&hash),
        "the card must measure main's head, got:\n{card}"
    );
}
