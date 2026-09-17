// SPDX-License-Identifier: AGPL-3.0-or-later
use serde_json::{json, Value};

fn tool_error(message: &str) -> Value {
    json!({ "content": [{ "type": "text", "text": message }], "isError": true })
}

fn tool_ok(text: String) -> Value {
    json!({ "content": [{ "type": "text", "text": text }], "isError": false })
}

fn call_tool(msg: &Value) -> Value {
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
        _ => tool_error(&format!("unknown tool: {}", name)),
    }
}

pub fn handle_request(line: &str) -> String {
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
        "tools/list" => json!({ "tools": [
            {
                "name": "okf.validate",
                "description": "Validate an OKF JSON document",
                "inputSchema": {
                    "type": "object",
                    "properties": { "okf": { "type": "string" } },
                    "required": ["okf"]
                }
            },
            {
                "name": "graph.stats",
                "description": "Graph health of an OKF document",
                "inputSchema": {
                    "type": "object",
                    "properties": { "okf": { "type": "string" } },
                    "required": ["okf"]
                }
            },
            {
                "name": "gate.run",
                "description": "Round-trip fidelity gate between two OKF documents",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "reference": { "type": "string" },
                        "candidate": { "type": "string" },
                        "strictCoverage": { "type": "boolean" }
                    },
                    "required": ["reference", "candidate"]
                }
            },
            {
                "name": "okf.diff",
                "description": "Semantic diff between two OKF documents: elements, edges and attributes",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "reference": { "type": "string" },
                        "candidate": { "type": "string" }
                    },
                    "required": ["reference", "candidate"]
                }
            }
        ] }),
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
        "tools/call" => call_tool(&msg),
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
