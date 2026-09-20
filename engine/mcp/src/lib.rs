// SPDX-License-Identifier: AGPL-3.0-or-later
pub mod repository;

use serde_json::{json, Value};

use repository::Repository;

fn tool_error(message: &str) -> Value {
    json!({ "content": [{ "type": "text", "text": message }], "isError": true })
}

fn tool_ok(text: String) -> Value {
    json!({ "content": [{ "type": "text", "text": text }], "isError": false })
}

fn pretty(value: &Value) -> String {
    serde_json::to_string_pretty(value).expect("tool result serializes")
}

/// The document-level tools: they operate on an OKF document supplied in the request and
/// never touch a repository or the network. They are always present, with or without
/// repository-mode configuration.
fn document_tool_definitions() -> Vec<Value> {
    vec![
        json!({
            "name": "okf.validate",
            "description": "Validate an OKF JSON document supplied in the request (document-level; no repository, no network)",
            "inputSchema": {
                "type": "object",
                "properties": { "okf": { "type": "string" } },
                "required": ["okf"]
            }
        }),
        json!({
            "name": "graph.stats",
            "description": "Graph health of an OKF document supplied in the request (document-level; no repository, no network)",
            "inputSchema": {
                "type": "object",
                "properties": { "okf": { "type": "string" } },
                "required": ["okf"]
            }
        }),
        json!({
            "name": "gate.run",
            "description": "Round-trip fidelity gate between two OKF documents supplied in the request (document-level; no repository, no network)",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "reference": { "type": "string" },
                    "candidate": { "type": "string" },
                    "strictCoverage": { "type": "boolean" }
                },
                "required": ["reference", "candidate"]
            }
        }),
        json!({
            "name": "okf.diff",
            "description": "Semantic diff between two OKF documents supplied in the request: elements, edges and attributes (document-level; no repository, no network)",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "reference": { "type": "string" },
                    "candidate": { "type": "string" }
                },
                "required": ["reference", "candidate"]
            }
        }),
    ]
}

/// The repository tools: read-and-propose tools over the running modelwrite service. They
/// are OPT-IN: each requires repository mode to be configured via MW_MCP_SERVICE_URL and
/// MW_MCP_TOKEN, and answers "not configured" (without any network call) when it is not. They
/// are always LISTED so the published contract stays stable, but they are inert until the
/// operator opts in.
fn repository_tool_definitions() -> Vec<Value> {
    vec![
        json!({
            "name": "repo.projects",
            "description": "List the projects the configured token may see. Repository mode (opt-in): requires MW_MCP_SERVICE_URL and MW_MCP_TOKEN; read permission; no network call is made unless both are set.",
            "inputSchema": {
                "type": "object",
                "properties": {},
                "required": []
            }
        }),
        json!({
            "name": "repo.branches",
            "description": "List a project's branches and their tips. Repository mode (opt-in): requires MW_MCP_SERVICE_URL and MW_MCP_TOKEN; read permission.",
            "inputSchema": {
                "type": "object",
                "properties": { "project": { "type": "string" } },
                "required": ["project"]
            }
        }),
        json!({
            "name": "repo.commits",
            "description": "List the commits on a project's branch (default main). Repository mode (opt-in): requires MW_MCP_SERVICE_URL and MW_MCP_TOKEN; read permission.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project": { "type": "string" },
                    "branch": { "type": "string" }
                },
                "required": ["project"]
            }
        }),
        json!({
            "name": "repo.read",
            "description": "Read the OKF model behind a commit and its commit record (provenance). Repository mode (opt-in): requires MW_MCP_SERVICE_URL and MW_MCP_TOKEN; read permission.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project": { "type": "string" },
                    "hash": { "type": "string" }
                },
                "required": ["project", "hash"]
            }
        }),
        json!({
            "name": "repo.find",
            "description": "Search a revision's elements by name, id or stereotype fragment, returning each match's id, name and kind. Repository mode (opt-in): requires MW_MCP_SERVICE_URL and MW_MCP_TOKEN; read permission.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project": { "type": "string" },
                    "hash": { "type": "string" },
                    "query": { "type": "string" }
                },
                "required": ["project", "hash", "query"]
            }
        }),
        json!({
            "name": "repo.element",
            "description": "Read one element by id from a revision: its attributes and its graph edges, so an agent can answer a one-line question without pulling the whole model. Repository mode (opt-in): requires MW_MCP_SERVICE_URL and MW_MCP_TOKEN; read permission.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project": { "type": "string" },
                    "hash": { "type": "string" },
                    "id": { "type": "string" }
                },
                "required": ["project", "hash", "id"]
            }
        }),
        json!({
            "name": "repo.coverage",
            "description": "Requirement coverage for a revision: covered and uncovered, each named, from the engine's requirement_coverage function (never a reimplementation). Repository mode (opt-in): requires MW_MCP_SERVICE_URL and MW_MCP_TOKEN; read permission.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project": { "type": "string" },
                    "hash": { "type": "string" }
                },
                "required": ["project", "hash"]
            }
        }),
        json!({
            "name": "repo.references",
            "description": "A model's typed subsystem references, with an optional resolve flag that reports each reference's resolve state via the SAME resolution the UI and API use. Repository mode (opt-in): requires MW_MCP_SERVICE_URL and MW_MCP_TOKEN; read permission.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project": { "type": "string" },
                    "hash": { "type": "string" },
                    "resolve": { "type": "boolean" }
                },
                "required": ["project", "hash"]
            }
        }),
        json!({
            "name": "repo.importReport",
            "description": "Read a migration's loss report and the engine's fidelity measurement, by retained artifact hash. Repository mode (opt-in): requires MW_MCP_SERVICE_URL and MW_MCP_TOKEN; read permission.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project": { "type": "string" },
                    "artifactHash": { "type": "string" }
                },
                "required": ["project", "artifactHash"]
            }
        }),
        json!({
            "name": "repo.lossSummary",
            "description": "A migration's loss report AGGREGATED by construct and verdict with counts, plus paging over the full list - the full list stays available and nothing is dropped. Repository mode (opt-in): requires MW_MCP_SERVICE_URL and MW_MCP_TOKEN; read permission.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project": { "type": "string" },
                    "artifactHash": { "type": "string" },
                    "offset": { "type": "integer" },
                    "limit": { "type": "integer" }
                },
                "required": ["project", "artifactHash"]
            }
        }),
        json!({
            "name": "repo.artifact",
            "description": "The retained original bytes of a migrated artifact, byte for byte, so an agent can verify a migration rather than trust a summary. Repository mode (opt-in): requires MW_MCP_SERVICE_URL and MW_MCP_TOKEN; read permission.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project": { "type": "string" },
                    "artifactHash": { "type": "string" }
                },
                "required": ["project", "artifactHash"]
            }
        }),
        json!({
            "name": "repo.diff",
            "description": "Diff two commits and run the round-trip gate LOCALLY from the two models read from the repository, returning the verdict and the evidence. It never records a run (recording is a write). Repository mode (opt-in): requires MW_MCP_SERVICE_URL and MW_MCP_TOKEN; read permission.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project": { "type": "string" },
                    "reference": { "type": "string" },
                    "candidate": { "type": "string" },
                    "strictCoverage": { "type": "boolean" }
                },
                "required": ["project", "reference", "candidate"]
            }
        }),
        json!({
            "name": "repo.audit",
            "description": "Read a project's append-only audit log. Repository mode (opt-in): requires MW_MCP_SERVICE_URL and MW_MCP_TOKEN; read permission.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project": { "type": "string" },
                    "limit": { "type": "integer" }
                },
                "required": ["project"]
            }
        }),
        json!({
            "name": "repo.checks",
            "description": "Read the gate checks recorded against a commit (what has been checked about this model). Repository mode (opt-in): requires MW_MCP_SERVICE_URL and MW_MCP_TOKEN; review permission.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project": { "type": "string" },
                    "hash": { "type": "string" }
                },
                "required": ["project", "hash"]
            }
        }),
        json!({
            "name": "repo.proposals",
            "description": "The project's proposals with their decisions and who made them, so an agent can see what happened to what it proposed. Repository mode (opt-in): requires MW_MCP_SERVICE_URL and MW_MCP_TOKEN; read permission.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project": { "type": "string" }
                },
                "required": ["project"]
            }
        }),
        json!({
            "name": "repo.analytics",
            "description": "The portfolio question: which models meet which requirements, read from the project analytics route. Repository mode (opt-in): requires MW_MCP_SERVICE_URL and MW_MCP_TOKEN; read permission.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project": { "type": "string" },
                    "requirements": { "type": "string" },
                    "branch": { "type": "string" },
                    "commit": { "type": "string" }
                },
                "required": ["project", "requirements"]
            }
        }),
        json!({
            "name": "repo.propose",
            "description": "Record a proposal for a human to review and decide. This is the agent's output: it persists a proposal (review permission) and never writes a change - a human accepts it. Repository mode (opt-in): requires MW_MCP_SERVICE_URL and MW_MCP_TOKEN.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project": { "type": "string" },
                    "reviewArtifact": { "type": "object" }
                },
                "required": ["project", "reviewArtifact"]
            }
        }),
    ]
}

fn all_tool_definitions() -> Vec<Value> {
    let mut tools = document_tool_definitions();
    tools.extend(repository_tool_definitions());
    tools
}

fn call_tool(msg: &Value, repo: &Repository) -> Value {
    let params = msg.get("params");
    let name = params
        .and_then(|p| p.get("name"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let args = params
        .and_then(|p| p.get("arguments"))
        .cloned()
        .unwrap_or_else(|| json!({}));
    match name {
        "okf.validate" => {
            let Some(okf) = args.get("okf").and_then(Value::as_str) else {
                return tool_error("missing okf argument");
            };
            match serde_json::from_str::<okf::types::OkfRoot>(okf) {
                Err(e) => tool_error(&format!("parse error: {}", e)),
                Ok(root) => {
                    let report = okf::validate::validate(&root);
                    tool_ok(serde_json::to_string_pretty(&report).expect("report serializes"))
                }
            }
        }
        "graph.stats" => {
            let Some(okf) = args.get("okf").and_then(Value::as_str) else {
                return tool_error("missing okf argument");
            };
            match serde_json::from_str::<okf::types::OkfRoot>(okf) {
                Err(e) => tool_error(&format!("parse error: {}", e)),
                Ok(root) => {
                    // The engine's graph helpers require a graph, so an invalid candidate
                    // must be reported as a tool error rather than crashing the server: a
                    // repair loop calls this tool on exactly these documents.
                    if root.graph.is_none() {
                        return tool_error("candidate has no graph section");
                    }
                    let stats = graph::graph_stats(&root);
                    tool_ok(serde_json::to_string_pretty(&stats).expect("stats serialize"))
                }
            }
        }
        "gate.run" => {
            let reference = args.get("reference").and_then(Value::as_str);
            let candidate = args.get("candidate").and_then(Value::as_str);
            let strict = args
                .get("strictCoverage")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let (Some(reference), Some(candidate)) = (reference, candidate) else {
                return tool_error("missing reference or candidate argument");
            };
            let parse = |s: &str| serde_json::from_str::<okf::types::OkfRoot>(s);
            match (parse(reference), parse(candidate)) {
                (Err(e), _) | (_, Err(e)) => tool_error(&format!("parse error: {}", e)),
                (Ok(reference), Ok(candidate)) => {
                    let outcome = gate::run(&reference, &candidate, strict);
                    tool_ok(
                        serde_json::to_string_pretty(&outcome.evidence)
                            .expect("evidence serializes"),
                    )
                }
            }
        }
        "okf.diff" => {
            let reference = args.get("reference").and_then(Value::as_str);
            let candidate = args.get("candidate").and_then(Value::as_str);
            let (Some(reference), Some(candidate)) = (reference, candidate) else {
                return tool_error("missing reference or candidate argument");
            };
            let parse = |s: &str| serde_json::from_str::<okf::types::OkfRoot>(s);
            match (parse(reference), parse(candidate)) {
                (Err(e), _) | (_, Err(e)) => tool_error(&format!("parse error: {}", e)),
                (Ok(reference), Ok(candidate)) => {
                    // DiffReport already serializes as camelCase, so the report is the
                    // payload: no hand-rolled key mapping can drift from the type.
                    let report = okf::diff::diff(&reference, &candidate);
                    tool_ok(serde_json::to_string_pretty(&report).expect("diff serializes"))
                }
            }
        }
        // Every repository tool routes through the single dispatch in repository::run_tool,
        // which issues only read (GET) routes and the one propose (POST /proposals) route.
        "repo.projects" | "repo.branches" | "repo.commits" | "repo.read" | "repo.find"
        | "repo.element" | "repo.coverage" | "repo.references" | "repo.importReport"
        | "repo.lossSummary" | "repo.diff" | "repo.audit" | "repo.checks" | "repo.proposals"
        | "repo.analytics" | "repo.propose" => match repo.run_tool(name, &args) {
            Ok(value) => tool_ok(pretty(&value)),
            Err(e) => tool_error(&e),
        },
        // The one repository tool whose answer is raw bytes, not JSON: it must reach the agent
        // verbatim - byte for byte - so it bypasses the JSON formatter.
        "repo.artifact" => match repo.run_raw_tool(name, &args) {
            Ok(text) => tool_ok(text),
            Err(e) => tool_error(&e),
        },
        _ => tool_error(&format!("unknown tool: {}", name)),
    }
}

pub fn handle_request(line: &str) -> String {
    handle_request_with(line, &Repository::disabled())
}

pub fn handle_request_with(line: &str, repo: &Repository) -> String {
    let msg: Value = match serde_json::from_str(line) {
        Ok(m) => m,
        Err(e) => {
            return json!({
                "jsonrpc": "2.0",
                "id": null,
                "error": { "code": -32700, "message": format!("parse error: {}", e) }
            })
            .to_string();
        }
    };
    let id = msg.get("id").cloned().unwrap_or(Value::Null);
    let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
    if method.is_empty() {
        return json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": { "code": -32600, "message": "method missing" }
        })
        .to_string();
    }
    if id.is_null() {
        return String::new();
    }
    let result = match method {
        "initialize" => json!({
            "protocolVersion": "2024-11-05",
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "modelwrite-mcp", "version": env!("CARGO_PKG_VERSION") }
        }),
        "tools/list" => json!({ "tools": all_tool_definitions() }),
        "resources/list" => json!({ "resources": [
            {
                "uri": "mw://okf/1.0/spec",
                "name": "OKF 1.0 specification",
                "mimeType": "text/markdown"
            },
            {
                "uri": "mw://evidence/latest",
                "name": "latest gate evidence",
                "mimeType": "application/json"
            }
        ] }),
        "resources/read" => {
            let uri = msg
                .get("params")
                .and_then(|p| p.get("uri"))
                .and_then(Value::as_str)
                .unwrap_or("");
            if uri == "mw://okf/1.0/spec" {
                let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../docs/okf/okf-1.0-spec.md");
                match std::fs::read_to_string(&path) {
                    Ok(text) => {
                        json!({ "contents": [{ "uri": uri, "mimeType": "text/markdown", "text": text }] })
                    }
                    Err(e) => {
                        json!({ "contents": [], "error": format!("cannot read spec: {}", e) })
                    }
                }
            } else {
                json!({ "contents": [] })
            }
        }
        "tools/call" => call_tool(&msg, repo),
        _ => {
            return json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32601, "message": format!("method not found: {}", method) }
            })
            .to_string();
        }
    };
    json!({ "jsonrpc": "2.0", "id": id, "result": result }).to_string()
}
