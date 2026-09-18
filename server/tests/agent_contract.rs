// SPDX-License-Identifier: AGPL-3.0-or-later
//! The published agent contract is enforced against the live surfaces, not trusted as prose.
//!
//! `docs/agents/mcp-tools.json` is the machine-readable contract: its `tools` array names
//! the document-level MCP tools, and its `repositoryHttpSurface.endpoints` array names every
//! repository HTTP endpoint an agent may call. This test reads that file at RUNTIME and
//! drives the real surfaces: the `server::app` router for every HTTP endpoint, and the MCP
//! server's own `tools/list` handler for the tool names and their required arguments. A
//! route or tool that is renamed, removed, or added without a contract entry fails here —
//! before an agent calls a surface that no longer exists in production.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::response::Response;
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;

use server::auth::{AuthConfig, Identity};
use server::store::{sqlite::SqliteStore, Store};
use server::{app, AppState};

/// The manifest every agent and this test read. Resolved at runtime, so an edit to the
/// contract is an edit to what this test enforces.
const MANIFEST: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../docs/agents/mcp-tools.json");

/// The document-level MCP tools, taken from the MCP server's OWN `tools/list` handler at
/// runtime. `engine/mcp` is the authority for what the MCP surface exposes, so the manifest
/// must name exactly these tools with the same required arguments; a rename or schema change
/// there fails this test before an agent is written against a stale contract.
fn mcp_tools_list() -> Vec<(String, Vec<String>)> {
    let response = mcp::handle_request(r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#);
    let value: Value = serde_json::from_str(&response).expect("tools/list must return JSON");
    value["result"]["tools"]
        .as_array()
        .expect("tools/list must return a tools array")
        .iter()
        .map(|tool| {
            let name = tool["name"]
                .as_str()
                .expect("a tool must have a name")
                .to_string();
            let required: Vec<String> = tool["inputSchema"]["required"]
                .as_array()
                .expect("a tool input schema must have a required array")
                .iter()
                .map(|arg| {
                    arg.as_str()
                        .expect("required arguments are strings")
                        .to_string()
                })
                .collect();
            (name, required)
        })
        .collect()
}

/// The document-level MCP tools named in the prose contract's tools table
/// (`docs/agents/mcp-agent.md`), each with its required arguments. The table rows are
/// `| \`name\` | \`arg\`, \`arg\` | summary |`; a tool name always contains a dot, which is
/// what distinguishes these rows from the permission table above them.
fn markdown_tool_rows() -> Vec<(String, Vec<String>)> {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../docs/agents/mcp-agent.md");
    let text = std::fs::read_to_string(path).expect("the prose contract must exist");
    text.lines()
        .filter(|line| line.trim_start().starts_with("| `"))
        .filter(|line| line.contains('.'))
        .map(|line| {
            let cells: Vec<&str> = line.split('|').map(str::trim).collect();
            let name = cells[1].trim_matches('`').to_string();
            let args: Vec<String> = cells[2]
                .split(',')
                .map(|arg| arg.trim().trim_matches('`').to_string())
                .filter(|arg| !arg.is_empty())
                .collect();
            (name, args)
        })
        .collect()
}

/// A marker the router's `fallback` returns for a path that does not exist, so a missing
/// route is distinguishable from a handler's own 404 (e.g. "project not found").
async fn route_missing() -> (StatusCode, &'static str) {
    (StatusCode::NOT_FOUND, "contract: route not found")
}

/// A marker the router returns when the path exists but the method does not, so a contract
/// that names the wrong verb fails rather than silently matching another route.
async fn method_missing() -> (StatusCode, &'static str) {
    (
        StatusCode::METHOD_NOT_ALLOWED,
        "contract: method not allowed",
    )
}

fn load_manifest() -> Value {
    let text = std::fs::read_to_string(MANIFEST)
        .expect("the agent contract manifest must exist at docs/agents/mcp-tools.json");
    serde_json::from_str(&text).expect("the agent contract manifest must be valid JSON")
}

fn app_with_auth(auth: AuthConfig) -> (axum::Router, Arc<SqliteStore>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(SqliteStore::open(&dir.path().join("mw.db")).unwrap());
    let state = AppState {
        store: store.clone(),
        evidence_dir: dir.path().to_path_buf(),
        auth,
    };
    (app(state), store, dir)
}

/// Substitute the route-table placeholders with concrete segments so each path can be sent.
fn concrete_path(path: &str) -> String {
    path.replace(":artifactHash", "artifact0000")
        .replace(":hash", "commit0000")
        .replace(":name", "main")
        .replace(":project", "coffee")
}

/// A request for one contract endpoint. GETs carry no body; other verbs carry `body`.
fn request_for(endpoint: &Value, body: &str) -> Request<Body> {
    let method = endpoint["method"].as_str().expect("endpoint method");
    let path = endpoint["path"].as_str().expect("endpoint path");
    let mut builder = Request::builder().method(method).uri(concrete_path(path));
    if method != "GET" {
        builder = builder.header("content-type", "application/json");
    }
    builder
        .body(if method == "GET" {
            Body::empty()
        } else {
            Body::from(body.to_string())
        })
        .unwrap()
}

/// A body that deserialises for each endpoint, so the permission test reaches the handler's
/// own permission decision rather than the body extractor's rejection.
fn valid_body(endpoint: &Value) -> Value {
    let method = endpoint["method"].as_str().unwrap();
    let path = endpoint["path"].as_str().unwrap();
    match (method, path) {
        ("POST", "/projects") => json!({ "name": "coffee" }),
        ("POST", "/projects/:project/commits") => {
            json!({ "branch": "main", "message": "m", "okf": {} })
        }
        ("POST", "/projects/:project/branches") => {
            json!({ "name": "review", "from": "abc" })
        }
        ("POST", "/projects/:project/branches/:name/reset") => {
            json!({ "to": "abc", "message": "m" })
        }
        ("POST", "/projects/:project/gate") => json!({ "reference": "a", "candidate": "b" }),
        ("POST", "/projects/:project/import") => json!({
            "binding": "b@1", "branch": "main", "message": "m", "artifact": "x"
        }),
        ("POST", "/projects/:project/merge") => {
            json!({ "branch": "main", "other": "feature", "message": "m" })
        }
        ("POST", "/projects/:project/locks") => json!({
            "branch": "main", "elements": ["b1"], "holder": "alex", "ttlSeconds": 300
        }),
        ("POST", "/projects/:project/locks/release") | ("DELETE", "/projects/:project/locks") => {
            json!({ "holder": "alex", "ids": ["x"] })
        }
        _ => json!({}),
    }
}

async fn body_text(response: Response) -> String {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8_lossy(&bytes).to_string()
}

async fn json_body(response: Response) -> Value {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
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

fn bearer_get(uri: &str, token: &str) -> Request<Body> {
    Request::builder()
        .uri(uri)
        .header("authorization", format!("Bearer {}", token))
        .body(Body::empty())
        .unwrap()
}

#[tokio::test]
async fn every_contract_endpoint_resolves_on_the_live_router() {
    let manifest = load_manifest();
    let endpoints = manifest["repositoryHttpSurface"]["endpoints"]
        .as_array()
        .expect("repositoryHttpSurface.endpoints must be an array");
    assert!(!endpoints.is_empty(), "the contract must name endpoints");

    let (router, _store, _dir) = app_with_auth(AuthConfig::Open);
    let router = router
        .fallback(route_missing)
        .method_not_allowed_fallback(method_missing);

    for endpoint in endpoints {
        let method = endpoint["method"].as_str().unwrap();
        let path = endpoint["path"].as_str().unwrap();
        let response = router
            .clone()
            .oneshot(request_for(endpoint, "{}"))
            .await
            .unwrap();
        let body = body_text(response).await;
        assert!(
            !body.contains("contract: route not found"),
            "{} {} is named in the contract but does not resolve on the router",
            method,
            path
        );
        assert!(
            !body.contains("contract: method not allowed"),
            "{} {} is named in the contract but the router does not serve that method",
            method,
            path
        );
    }
}

#[tokio::test]
async fn every_contract_endpoint_enforces_its_permission() {
    let manifest = load_manifest();
    let endpoints = manifest["repositoryHttpSurface"]["endpoints"]
        .as_array()
        .expect("repositoryHttpSurface.endpoints must be an array");

    // A no-role identity holds no permission, so every permissioned route must refuse it
    // 403 before touching the store. Only the two public liveness/information routes may
    // answer.
    let (router, _store, _dir) = app_with_auth(AuthConfig::fixed(Identity {
        subject: "nobody".to_string(),
        roles: Vec::new(),
        projects: vec!["*".to_string()],
    }));

    for endpoint in endpoints {
        let method = endpoint["method"].as_str().unwrap();
        let path = endpoint["path"].as_str().unwrap();
        let permission = endpoint["permission"].as_str().unwrap();
        let body = valid_body(endpoint).to_string();
        let response = router
            .clone()
            .oneshot(request_for(endpoint, &body))
            .await
            .unwrap();
        let status = response.status();
        if permission == "public" {
            assert_eq!(
                status,
                StatusCode::OK,
                "{} {} is marked public and must answer with no identity",
                method,
                path
            );
        } else {
            assert_eq!(
                status,
                StatusCode::FORBIDDEN,
                "{} {} declares permission {:?} but is not refused 403 for a caller with no role",
                method,
                path,
                permission
            );
        }
    }
}

#[test]
fn the_manifest_names_exactly_the_mcp_servers_tools() {
    let manifest = load_manifest();
    let tools = manifest["tools"]
        .as_array()
        .expect("tools must be an array");
    let real = mcp_tools_list();

    let manifest_names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    let real_names: Vec<&str> = real.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(
        manifest_names, real_names,
        "the manifest must name exactly the tools the MCP server's tools/list exposes"
    );

    for tool in tools {
        let name = tool["name"].as_str().unwrap();
        let required: Vec<&str> = tool["inputSchema"]["required"]
            .as_array()
            .expect("inputSchema.required must be an array")
            .iter()
            .map(|s| s.as_str().unwrap())
            .collect();
        let (_, real_required) = real
            .iter()
            .find(|(n, _)| n == name)
            .expect("the tool must be present in tools/list");
        assert_eq!(
            required,
            real_required.iter().map(String::as_str).collect::<Vec<_>>(),
            "{} must keep its required arguments",
            name
        );
    }
}

#[test]
fn the_prose_contract_names_exactly_the_manifest_tools() {
    // The prose contract's MCP tool table must not drift from the manifest: both name the
    // same tools with the same required arguments. The manifest is itself checked against
    // the MCP server's tools/list above, so this closes the markdown half of the drift the
    // two documents could otherwise develop.
    let manifest = load_manifest();
    let json_tools: Vec<(String, Vec<String>)> = manifest["tools"]
        .as_array()
        .expect("tools must be an array")
        .iter()
        .map(|tool| {
            let name = tool["name"].as_str().unwrap().to_string();
            let required: Vec<String> = tool["inputSchema"]["required"]
                .as_array()
                .unwrap()
                .iter()
                .map(|arg| arg.as_str().unwrap().to_string())
                .collect();
            (name, required)
        })
        .collect();

    assert_eq!(
        markdown_tool_rows(),
        json_tools,
        "the prose contract's MCP tool table must match the manifest (name and required arguments)"
    );
}

#[tokio::test]
async fn the_agent_mechanism_authenticates_a_named_agent_and_lets_it_read() {
    let (router, store, _dir) = app_with_auth(
        AuthConfig::agent_token("agent-secret", "mw-agent", "alex", &["reviewer"], &["*"]).unwrap(),
    );

    // Health reports the mechanism ("agent"), never the token.
    let health = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(health.status(), StatusCode::OK);
    let health_body = json_body(health).await;
    assert_eq!(health_body["authMode"], "agent");

    // The agent may read a project its roles reach. It may not write: the agent mechanism has
    // no write path, which `an_agent_is_a_client_not_a_privileged_path` and the per-route
    // denial test in `agent_audit.rs` prove.
    store.create_project("coffee", None).unwrap();
    let read = router
        .clone()
        .oneshot(bearer_get("/projects", "agent-secret"))
        .await
        .unwrap();
    assert_eq!(read.status(), StatusCode::OK);
    let projects = json_body(read).await;
    assert_eq!(
        projects.as_array().unwrap().len(),
        1,
        "an agent may read the project its roles reach"
    );
}

#[tokio::test]
async fn an_agent_is_a_client_not_a_privileged_path() {
    let (router, store, _dir) = app_with_auth(
        AuthConfig::agent_token("agent-secret", "mw-agent", "alex", &["viewer"], &["*"]).unwrap(),
    );
    store.create_project("coffee", None).unwrap();

    let response = router
        .oneshot(bearer_post(
            "/projects/coffee/commits",
            json!({ "branch": "main", "author": "mw-agent", "message": "m", "okf": {} }),
            "agent-secret",
        ))
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::FORBIDDEN,
        "an agent holding only the viewer role must be refused a write"
    );
}
