// SPDX-License-Identifier: AGPL-3.0-or-later
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

use server::store::Store;

fn open_store(dir: &std::path::Path) -> Arc<dyn Store> {
    let store = server::store::sqlite::SqliteStore::open(&dir.join("mw.db")).unwrap();
    Arc::new(store)
}

fn state(dir: &std::path::Path) -> server::AppState {
    server::AppState {
        store: open_store(dir),
        evidence_dir: dir.to_path_buf(),
        auth: server::auth::AuthConfig::Open,
    }
}

fn post(uri: &str, body: serde_json::Value) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

async fn json_body(response: axum::response::Response) -> serde_json::Value {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

async fn create_project(router: &axum::Router, name: &str) {
    let response = router
        .clone()
        .oneshot(post("/projects", serde_json::json!({ "name": name })))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
}

async fn commit(router: &axum::Router, project: &str, okf: serde_json::Value) -> String {
    let response = router
        .clone()
        .oneshot(post(
            &format!("/projects/{}/commits", project),
            serde_json::json!({ "branch": "main", "author": "alex", "message": "model", "okf": okf }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    json_body(response).await["hash"]
        .as_str()
        .unwrap()
        .to_string()
}

async fn gate(
    router: &axum::Router,
    project: &str,
    reference: &str,
    candidate: &str,
) -> serde_json::Value {
    let response = router
        .clone()
        .oneshot(post(
            &format!("/projects/{}/gate", project),
            serde_json::json!({ "reference": reference, "candidate": candidate }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    json_body(response).await
}

/// A minimal, gateable model: one block satisfying one requirement, so a self-gate passes
/// (no isolated nodes, one connected component, full coverage).
fn minimal(name: &str) -> serde_json::Value {
    serde_json::json!({
        "project": name,
        "exportedAt": "2026-09-19T00:00:00Z",
        "summary": {},
        "stateMachine": { "name": "sm", "regions": [] },
        "requirements": [
            { "id": "r1", "name": "Req", "kind": "requirement", "stereotypes": ["Requirement"], "attributes": [], "documentation": "", "reqId": "1.1", "reqText": "shall satisfy" }
        ],
        "graph": {
            "nodes": [
                { "id": "b1", "kind": "block", "name": "Block" },
                { "id": "r1", "kind": "requirement", "name": "Req" }
            ],
            "edges": [
                { "source": "b1", "target": "r1", "kind": "dependency", "label": "Satisfy" }
            ]
        }
    })
}

/// A platform model that binds to the subsystem's block `b1`. One requirement (pr1) is
/// covered by a local block; the other (pr2) is covered ONLY by the cross-model bound
/// element (the proxy node `b1` carrying a Satisfy edge to pr2). The association edge keeps
/// the proxy in the platform's single connected component, so the single-model gate passes
/// and only the coverage split distinguishes local from cross-model.
fn platform_with_cross_model_coverage(radar_hash: &str) -> serde_json::Value {
    serde_json::json!({
        "project": "ship",
        "exportedAt": "2026-09-19T00:00:00Z",
        "summary": {},
        "stateMachine": { "name": "sm", "regions": [] },
        "references": [
            { "project": "radar", "revision": radar_hash, "role": "radar", "bounds": ["b1"] }
        ],
        "requirements": [
            { "id": "pr1", "name": "Local", "kind": "requirement", "stereotypes": ["Requirement"], "attributes": [], "documentation": "", "reqId": "2.1", "reqText": "covered locally" },
            { "id": "pr2", "name": "Cross", "kind": "requirement", "stereotypes": ["Requirement"], "attributes": [], "documentation": "", "reqId": "2.2", "reqText": "covered cross-model" }
        ],
        "graph": {
            "nodes": [
                { "id": "pb1", "kind": "block", "name": "Platform Block" },
                { "id": "pr1", "kind": "requirement", "name": "Local" },
                { "id": "pr2", "kind": "requirement", "name": "Cross" },
                { "id": "b1", "kind": "block", "name": "Radar Block (bound)" }
            ],
            "edges": [
                { "source": "pb1", "target": "pr1", "kind": "dependency", "label": "Satisfy" },
                { "source": "b1", "target": "pr2", "kind": "dependency", "label": "Satisfy" },
                { "source": "pb1", "target": "b1", "kind": "association", "label": "" }
            ]
        }
    })
}

#[tokio::test]
async fn a_gated_platform_model_passes_and_carries_the_measured_vs_asserted_boundary() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    create_project(&router, "radar").await;
    let radar_hash = commit(&router, "radar", minimal("radar")).await;
    gate(&router, "radar", &radar_hash, &radar_hash).await;

    create_project(&router, "ship").await;
    let mut platform = minimal("ship");
    platform["references"] = serde_json::json!([
        { "project": "radar", "revision": radar_hash, "role": "radar" }
    ]);
    let ship_hash = commit(&router, "ship", platform).await;

    let evidence = gate(&router, "ship", &ship_hash, &ship_hash).await;
    assert_eq!(evidence["passed"], true, "evidence: {}", evidence);

    // The boundary is data, not prose: the gate says what it measured and what it
    // asserted, in the same breath.
    let boundary = &evidence["composition"]["boundary"];
    let measured = boundary["measured"].as_array().unwrap();
    assert_eq!(measured.len(), 4, "four measured checks: {:?}", measured);
    let asserted = boundary["asserted"].as_array().unwrap();
    assert_eq!(asserted.len(), 1, "one asserted claim: {:?}", asserted);
    assert!(!asserted[0].as_str().unwrap().contains("measured"));
    assert!(
        asserted[0]
            .as_str()
            .unwrap()
            .contains("global graph property"),
        "the asserted claim must name the global graph property: {}",
        asserted[0]
    );

    // Every measured finding carries its kind, so a report cannot blur a proof and a
    // claim the way a column of green ticks would.
    assert_eq!(evidence["composition"]["resolution"][0]["resolves"], true);
    assert_eq!(evidence["composition"]["resolution"][0]["kind"], "measured");
    assert_eq!(evidence["composition"]["wasGated"][0]["gated"], true);
    assert_eq!(evidence["composition"]["wasGated"][0]["kind"], "measured");
}

#[tokio::test]
async fn an_un_gated_subsystem_revision_fails_named() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    create_project(&router, "radar").await;
    let radar_hash = commit(&router, "radar", minimal("radar")).await;
    // Deliberately do NOT gate radar: the platform's integration of it must be refused.

    create_project(&router, "ship").await;
    let mut platform = minimal("ship");
    platform["references"] = serde_json::json!([
        { "project": "radar", "revision": radar_hash, "role": "radar" }
    ]);
    let ship_hash = commit(&router, "ship", platform).await;

    let evidence = gate(&router, "ship", &ship_hash, &ship_hash).await;
    assert_eq!(evidence["passed"], false);
    let failures = evidence["failures"].as_array().unwrap();
    assert!(
        failures.iter().any(|f| {
            let s = f.as_str().unwrap();
            s.contains("radar") && s.contains("not gated")
        }),
        "the failure must name the un-gated reference: {:?}",
        failures
    );
    assert_eq!(evidence["composition"]["wasGated"][0]["gated"], false);
}

#[tokio::test]
async fn a_stale_reference_fails_named_in_the_composition_check() {
    // A stale reference cannot even be committed (422 at commit), so the gate-level
    // resolution check is exercised directly against the store with a hand-built platform
    // model: the same measurement the commit path already enforces, re-run by the gate.
    let dir = tempfile::tempdir().unwrap();
    let store = open_store(dir.path());
    let router = server::app(server::AppState {
        store: store.clone(),
        evidence_dir: dir.path().to_path_buf(),
        auth: server::auth::AuthConfig::Open,
    });
    create_project(&router, "radar").await;
    let _radar_hash = commit(&router, "radar", minimal("radar")).await;

    let stale = "0".repeat(64);
    let mut platform: okf::types::OkfRoot = serde_json::from_value(minimal("ship")).unwrap();
    platform.references.push(okf::types::SubsystemReference {
        project: "radar".into(),
        revision: stale,
        role: "radar".into(),
        bounds: Vec::new(),
        cross_model_edges: Vec::new(),
    });

    let report = server::composition::check(store.as_ref(), &platform).unwrap();
    assert!(
        report
            .failures
            .iter()
            .any(|f| f.contains("does not resolve") && f.contains("radar")),
        "the failure must name the stale reference: {:?}",
        report.failures
    );
}

#[tokio::test]
async fn a_requirement_covered_only_cross_model_is_reported_distinctly() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    create_project(&router, "radar").await;
    let radar_hash = commit(&router, "radar", minimal("radar")).await;
    gate(&router, "radar", &radar_hash, &radar_hash).await;

    create_project(&router, "ship").await;
    let ship_hash = commit(
        &router,
        "ship",
        platform_with_cross_model_coverage(&radar_hash),
    )
    .await;

    let evidence = gate(&router, "ship", &ship_hash, &ship_hash).await;
    assert_eq!(evidence["passed"], true, "evidence: {}", evidence);

    let coverage = &evidence["composition"]["coverage"];
    assert_eq!(coverage["local"], serde_json::json!(["pr1"]));
    assert_eq!(coverage["crossModelOnly"], serde_json::json!(["pr2"]));
}

#[tokio::test]
async fn a_bound_element_that_does_not_exist_fails_named() {
    // Deliverable 1: a reference resolves only if every bound element exists within the
    // pinned revision. The commit path checks project+revision only, so a bound element that
    // names nothing is caught by the gate, named, never silently accepted.
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    create_project(&router, "radar").await;
    let radar_hash = commit(&router, "radar", minimal("radar")).await;
    gate(&router, "radar", &radar_hash, &radar_hash).await;

    create_project(&router, "ship").await;
    let mut platform = minimal("ship");
    platform["references"] = serde_json::json!([
        { "project": "radar", "revision": radar_hash, "role": "radar", "bounds": ["no-such-block"] }
    ]);
    let ship_hash = commit(&router, "ship", platform).await;

    let evidence = gate(&router, "ship", &ship_hash, &ship_hash).await;
    assert_eq!(evidence["passed"], false);
    let failures = evidence["failures"].as_array().unwrap();
    assert!(
        failures.iter().any(|f| {
            let s = f.as_str().unwrap();
            s.contains("no-such-block") && s.contains("does not resolve")
        }),
        "the failure must name the missing bound element: {:?}",
        failures
    );
}

/// A minimal model whose satisfying block has a caller-chosen id, so a subsystem revision
/// can be re-pinned to a revision where the bound element no longer exists under the old id.
fn minimal_block(name: &str, block_id: &str) -> serde_json::Value {
    serde_json::json!({
        "project": name,
        "exportedAt": "2026-09-19T00:00:00Z",
        "summary": {},
        "stateMachine": { "name": "sm", "regions": [] },
        "requirements": [
            { "id": "r1", "name": "Req", "kind": "requirement", "stereotypes": ["Requirement"], "attributes": [], "documentation": "", "reqId": "1.1", "reqText": "shall satisfy" }
        ],
        "graph": {
            "nodes": [
                { "id": block_id, "kind": "block", "name": "Block" },
                { "id": "r1", "kind": "requirement", "name": "Req" }
            ],
            "edges": [
                { "source": block_id, "target": "r1", "kind": "dependency", "label": "Satisfy" }
            ]
        }
    })
}

/// A platform model carrying one cross-model edge: its requirement r1 is satisfiedBy the
/// subsystem element named `to`. The platform's own graph still satisfies r1 locally (so the
/// single-model gate passes); the cross-model edge is the S2 traceability under test.
fn platform_with_cross_model_edge(radar_hash: &str, to: &str) -> serde_json::Value {
    let mut platform = minimal("ship");
    platform["references"] = serde_json::json!([
        {
            "project": "radar",
            "revision": radar_hash,
            "role": "radar",
            "bounds": [],
            "crossModelEdges": [
                { "from": "r1", "relation": "satisfiedBy", "to": to }
            ]
        }
    ]);
    platform
}

#[tokio::test]
async fn a_cross_model_edge_resolves_at_the_pinned_revision() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    create_project(&router, "radar").await;
    let radar_hash = commit(&router, "radar", minimal("radar")).await;
    gate(&router, "radar", &radar_hash, &radar_hash).await;

    create_project(&router, "ship").await;
    let ship_hash = commit(
        &router,
        "ship",
        platform_with_cross_model_edge(&radar_hash, "b1"),
    )
    .await;

    let evidence = gate(&router, "ship", &ship_hash, &ship_hash).await;
    assert_eq!(evidence["passed"], true, "evidence: {}", evidence);

    let edges = &evidence["composition"]["crossModelEdges"];
    assert_eq!(edges["resolved"], serde_json::json!(1), "edges: {}", edges);
    assert_eq!(
        edges["unresolved"],
        serde_json::json!(0),
        "edges: {}",
        edges
    );
    let edge = &edges["edges"][0];
    assert_eq!(edge["resolves"], true);
    assert_eq!(edge["from"], "r1");
    assert_eq!(edge["relation"], "satisfiedBy");
    assert_eq!(edge["to"], "b1");
    assert_eq!(edge["project"], "radar");
    assert_eq!(edge["revision"], radar_hash.as_str());
}

#[tokio::test]
async fn a_cross_model_edge_naming_a_missing_element_fails_by_name() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    create_project(&router, "radar").await;
    let radar_hash = commit(&router, "radar", minimal("radar")).await;
    gate(&router, "radar", &radar_hash, &radar_hash).await;

    create_project(&router, "ship").await;
    let ship_hash = commit(
        &router,
        "ship",
        platform_with_cross_model_edge(&radar_hash, "no-such-block"),
    )
    .await;

    let evidence = gate(&router, "ship", &ship_hash, &ship_hash).await;
    assert_eq!(evidence["passed"], false);
    let failures = evidence["failures"].as_array().unwrap();
    assert!(
        failures.iter().any(|f| {
            let s = f.as_str().unwrap();
            s.contains("no-such-block")
                && s.contains("cross-model edge")
                && s.contains("does not resolve")
        }),
        "the failure must name the missing element and the edge: {:?}",
        failures
    );

    let edges = &evidence["composition"]["crossModelEdges"];
    assert_eq!(edges["resolved"], serde_json::json!(0));
    assert_eq!(edges["unresolved"], serde_json::json!(1));
    let edge = &edges["edges"][0];
    assert_eq!(edge["resolves"], false);
    assert!(edge["reason"].as_str().unwrap().contains("no-such-block"));
}

#[tokio::test]
async fn a_reference_revision_change_re_resolves_the_cross_model_edge() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    create_project(&router, "radar").await;
    let radar_v1 = commit(&router, "radar", minimal_block("radar", "b1")).await;
    gate(&router, "radar", &radar_v1, &radar_v1).await;
    // A new revision where the bound element b1 no longer exists (it is now b2).
    let radar_v2 = commit(&router, "radar", minimal_block("radar", "b2")).await;
    gate(&router, "radar", &radar_v2, &radar_v2).await;

    create_project(&router, "ship").await;
    let ship_v1 = commit(
        &router,
        "ship",
        platform_with_cross_model_edge(&radar_v1, "b1"),
    )
    .await;
    let ev1 = gate(&router, "ship", &ship_v1, &ship_v1).await;
    assert_eq!(
        ev1["composition"]["crossModelEdges"]["edges"][0]["resolves"],
        true
    );

    // Re-pin to the new revision WITHOUT changing the edge: it must now fail by name,
    // because the edge is content-addressed through the reference's pinned revision.
    let ship_v2 = commit(
        &router,
        "ship",
        platform_with_cross_model_edge(&radar_v2, "b1"),
    )
    .await;
    let ev2 = gate(&router, "ship", &ship_v2, &ship_v2).await;
    assert_eq!(ev2["passed"], false, "evidence: {}", ev2);
    let edge = &ev2["composition"]["crossModelEdges"]["edges"][0];
    assert_eq!(edge["resolves"], false);
    assert!(edge["reason"].as_str().unwrap().contains("b1"));
    assert_eq!(edge["revision"], radar_v2.as_str());
}
