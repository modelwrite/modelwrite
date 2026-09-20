// SPDX-License-Identifier: AGPL-3.0-or-later
//! The variant impact panel's cross-model view (S2): comparing two platform versions that
//! declare DIFFERENT cross-model edges must show a real, checkable difference in what each
//! option delivers - not two identical graphs hiding a changed reference. The two sides here
//! are the cafe-stand's own shape: main integrates floor-robot (the Cup Collector), and
//! with-microduck integrates microduck (the Gripper Arm), both satisfying CS-4.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

fn state(dir: &std::path::Path) -> server::AppState {
    let store = server::store::sqlite::SqliteStore::open(&dir.join("mw.db")).unwrap();
    server::AppState {
        store: std::sync::Arc::new(store),
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
        .oneshot(post("/projects", serde_json::json!({ "name": name })))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
}

async fn commit(
    router: &axum::Router,
    project: &str,
    branch: &str,
    okf: serde_json::Value,
) -> String {
    let response = router
        .clone()
        .oneshot(post(
            &format!("/projects/{}/commits", project),
            serde_json::json!({ "branch": branch, "author": "alex", "message": "model", "okf": okf }),
        ))
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::CREATED,
        "commit to {}",
        branch
    );
    json_body(response).await["hash"]
        .as_str()
        .unwrap()
        .to_string()
}

/// A minimal but valid subsystem carrying one named block (the element a cross-model edge
/// points at) and one requirement it satisfies.
fn subsystem(project: &str, element_id: &str, element_name: &str) -> serde_json::Value {
    serde_json::json!({
        "project": project,
        "exportedAt": "2026-09-19T00:00:00Z",
        "summary": {},
        "stateMachine": { "name": "sm", "regions": [] },
        "structure": [
            { "id": element_id, "name": element_name, "kind": "block", "stereotypes": ["Block"], "attributes": [], "documentation": "" }
        ],
        "requirements": [
            { "id": "req-1", "name": "Req", "kind": "requirement", "stereotypes": ["Requirement"], "attributes": [], "documentation": "", "reqId": "1.1", "reqText": "shall satisfy" }
        ],
        "graph": {
            "nodes": [
                { "id": element_id, "kind": "block", "name": element_name },
                { "id": "req-1", "kind": "requirement", "name": "Req" }
            ],
            "edges": [
                { "source": element_id, "target": "req-1", "kind": "dependency", "label": "Satisfy" }
            ]
        }
    })
}

/// A cafe-stand-shaped platform: one requirement CS-4 satisfied locally by the Clear the
/// Floor activity, and ONE cross-model edge declaring CS-4 satisfiedBy the subsystem element
/// `to` inside the floor-care reference.
fn cafe_stand(subsystem_project: &str, revision: &str, to: &str) -> serde_json::Value {
    serde_json::json!({
        "project": "cafe-stand",
        "exportedAt": "2026-09-19T00:00:00Z",
        "summary": {},
        "stateMachine": { "name": "sm", "regions": [] },
        "references": [
            {
                "project": subsystem_project,
                "revision": revision,
                "role": "floor-care",
                "bounds": [],
                "crossModelEdges": [
                    { "from": "req-floor-clear", "relation": "satisfiedBy", "to": to }
                ]
            }
        ],
        "requirements": [
            { "id": "req-floor-clear", "name": "Floor Kept Clear", "kind": "requirement", "stereotypes": ["Requirement"], "attributes": [], "documentation": "", "reqId": "CS-4", "reqText": "The stand shall keep the seated floor free of dropped cups." }
        ],
        "graph": {
            "nodes": [
                { "id": "cafe-stand", "kind": "block", "name": "Cafe Stand" },
                { "id": "act-clear-floor", "kind": "activity", "name": "Clear the Floor" },
                { "id": "req-floor-clear", "kind": "requirement", "name": "Floor Kept Clear" }
            ],
            "edges": [
                { "source": "cafe-stand", "target": "act-clear-floor", "kind": "part", "label": "" },
                { "source": "act-clear-floor", "target": "req-floor-clear", "kind": "dependency", "label": "Satisfy" }
            ]
        }
    })
}

#[tokio::test]
async fn two_cafe_stand_versions_show_a_different_checkable_requirement_impact() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));

    create_project(&router, "floor-robot").await;
    let floor_robot = commit(
        &router,
        "floor-robot",
        "main",
        subsystem("floor-robot", "collector", "Cup Collector"),
    )
    .await;
    create_project(&router, "microduck").await;
    let microduck = commit(
        &router,
        "microduck",
        "main",
        subsystem("microduck", "gripper-arm", "Gripper Arm"),
    )
    .await;

    create_project(&router, "cafe-stand").await;
    let main_hash = commit(
        &router,
        "cafe-stand",
        "main",
        cafe_stand("floor-robot", &floor_robot, "collector"),
    )
    .await;
    let branched = router
        .clone()
        .oneshot(post(
            "/projects/cafe-stand/branches",
            serde_json::json!({ "name": "with-microduck", "from": main_hash }),
        ))
        .await
        .unwrap();
    assert_eq!(branched.status(), StatusCode::CREATED);
    commit(
        &router,
        "cafe-stand",
        "with-microduck",
        cafe_stand("microduck", &microduck, "gripper-arm"),
    )
    .await;

    let response = router
        .oneshot(get(
            "/ui/projects/cafe-stand/compare?from=main&to=with-microduck",
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;

    // The two sides resolve DIFFERENT cross-model edges: the same requirement CS-4 is
    // satisfied by the Cup Collector in one and the Gripper Arm in the other, each named
    // by element name and project@revision - a real, checkable difference.
    assert!(
        html.contains("Cup Collector"),
        "main must name the Cup Collector, got:\n{}",
        html
    );
    assert!(
        html.contains("Gripper Arm"),
        "with-microduck must name the Gripper Arm, got:\n{}",
        html
    );
    assert!(
        html.contains("floor-robot"),
        "the panel must name floor-robot"
    );
    assert!(html.contains("microduck"), "the panel must name microduck");
    assert!(
        html.contains("satisfied by"),
        "the panel must say what satisfies the requirement"
    );

    // The narrowed boundary: the global graph property is now the one thing still asserted,
    // not coverage inside a referenced subsystem.
    assert!(
        html.contains("global graph property"),
        "the boundary must name the global graph property, got:\n{}",
        html
    );
    assert!(
        !html.contains("not a first-class cross-model edge"),
        "the stale allocatedTo boundary wording must be gone, got:\n{}",
        html
    );
}
