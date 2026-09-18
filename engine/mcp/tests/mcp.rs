// SPDX-License-Identifier: AGPL-3.0-or-later
use serde_json::{json, Value};

fn call(payload: Value) -> Value {
    let out = mcp::handle_request(&payload.to_string());
    serde_json::from_str(&out).expect("valid JSON-RPC response")
}

#[test]
fn initialize_returns_server_info() {
    let resp = call(json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {} }));
    assert_eq!(resp["id"], 1);
    assert_eq!(resp["result"]["serverInfo"]["name"], "modelwrite-mcp");
    assert_eq!(resp["result"]["protocolVersion"], "2024-11-05");
}

#[test]
fn tools_list_exposes_the_agent_toolset() {
    let resp = call(json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {} }));
    let names: Vec<&str> = resp["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        vec![
            "okf.validate",
            "graph.stats",
            "gate.run",
            "okf.diff",
            "repo.projects",
            "repo.branches",
            "repo.commits",
            "repo.read",
            "repo.importReport",
            "repo.diff",
            "repo.audit",
            "repo.checks",
            "repo.propose",
        ]
    );
}

#[test]
fn validate_tool_reports_invalid() {
    let resp = call(json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "tools/call",
        "params": { "name": "okf.validate", "arguments": { "okf": "{\"project\":\"x\"}" } }
    }));
    let text = resp["result"]["content"][0]["text"].as_str().unwrap();
    let report: Value = serde_json::from_str(text).unwrap();
    assert_eq!(report["valid"], false);
}

#[test]
fn gate_tool_runs_roundtrip() {
    let a = test_support::load_okf_expected();
    let resp = call(json!({
        "jsonrpc": "2.0",
        "id": 4,
        "method": "tools/call",
        "params": { "name": "gate.run", "arguments": { "reference": a, "candidate": a } }
    }));
    let text = resp["result"]["content"][0]["text"].as_str().unwrap();
    let evidence: Value = serde_json::from_str(text).unwrap();
    assert!(evidence["passed"].as_bool().unwrap());
}

#[test]
fn diff_tool_reports_a_removed_requirement() {
    let reference = test_support::load_okf_expected();
    let mut model: serde_json::Value = serde_json::from_str(&reference).unwrap();
    model["requirements"].as_array_mut().unwrap().pop();
    let candidate = model.to_string();
    let resp = call(json!({
        "jsonrpc": "2.0",
        "id": 6,
        "method": "tools/call",
        "params": { "name": "okf.diff", "arguments": { "reference": reference, "candidate": candidate } }
    }));
    let text = resp["result"]["content"][0]["text"].as_str().unwrap();
    let report: Value = serde_json::from_str(text).unwrap();
    assert_eq!(report["equal"], false);
    assert_eq!(report["missingElements"].as_array().unwrap().len(), 1);
}

#[test]
fn resources_list_exposes_the_spec() {
    let resp = call(json!({ "jsonrpc": "2.0", "id": 7, "method": "resources/list", "params": {} }));
    let uris: Vec<&str> = resp["result"]["resources"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["uri"].as_str().unwrap())
        .collect();
    assert!(uris.contains(&"mw://okf/1.0/spec"));
}

#[test]
fn unknown_method_returns_error() {
    let resp = call(json!({ "jsonrpc": "2.0", "id": 5, "method": "nope", "params": {} }));
    assert_eq!(resp["error"]["code"], -32601);
}
#[test]
fn notification_gets_no_response() {
    // A JSON-RPC notification carries no id and must never be answered.
    let out = mcp::handle_request(
        &json!({ "jsonrpc": "2.0", "method": "initialize", "params": {} }).to_string(),
    );
    assert!(
        out.is_empty(),
        "notifications must not be answered: {}",
        out
    );
}

#[test]
fn malformed_input_returns_a_parse_error() {
    let resp: Value = serde_json::from_str(&mcp::handle_request("this is not json"))
        .expect("a valid JSON-RPC error response");
    assert_eq!(resp["error"]["code"], -32700);
    assert!(resp["id"].is_null());
}

#[test]
fn tool_error_is_reported_in_the_result_not_the_transport() {
    // A missing argument is a tool failure, not a transport failure: the JSON-RPC
    // envelope must stay a success so an agent can read the error and adapt.
    let resp = call(json!({
        "jsonrpc": "2.0",
        "id": 9,
        "method": "tools/call",
        "params": { "name": "okf.validate", "arguments": {} }
    }));
    assert!(resp.get("error").is_none(), "unexpected error: {}", resp);
    assert_eq!(resp["result"]["isError"], true);
}
#[test]
fn graph_stats_tool_reports_health() {
    let okf = test_support::load_okf_expected();
    let resp = call(json!({
        "jsonrpc": "2.0",
        "id": 10,
        "method": "tools/call",
        "params": { "name": "graph.stats", "arguments": { "okf": okf } }
    }));
    let text = resp["result"]["content"][0]["text"].as_str().unwrap();
    let stats: Value = serde_json::from_str(text).unwrap();
    assert_eq!(stats["nodeCount"], 99);
    assert_eq!(stats["edgeCount"], 165);
    assert_eq!(stats["componentCount"], 1);
}

#[test]
fn graph_stats_tool_errors_on_a_graph_less_document() {
    // A repair loop calls this on invalid candidates, so it must answer with an error
    // rather than taking the server down.
    let resp = call(json!({
        "jsonrpc": "2.0",
        "id": 11,
        "method": "tools/call",
        "params": { "name": "graph.stats", "arguments": { "okf": "{\"project\":\"x\",\"summary\":{},\"stateMachine\":{\"name\":\"sm\",\"regions\":[]},\"graph\":null}" } }
    }));
    assert!(
        resp.get("error").is_none(),
        "unexpected transport error: {}",
        resp
    );
    assert_eq!(resp["result"]["isError"], true);
    assert!(resp["result"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("no graph section"));
}
#[test]
fn graph_stats_tool_errors_when_the_graph_key_is_absent() {
    // An absent key and an explicit null must behave identically: serde deserialises a
    // missing Option field to None, so both reach the guard rather than a panic.
    let resp = call(json!({
        "jsonrpc": "2.0",
        "id": 12,
        "method": "tools/call",
        "params": { "name": "graph.stats", "arguments": { "okf": "{\"project\":\"x\",\"summary\":{},\"stateMachine\":{\"name\":\"sm\",\"regions\":[]}}" } }
    }));
    assert!(
        resp.get("error").is_none(),
        "unexpected transport error: {}",
        resp
    );
    assert_eq!(resp["result"]["isError"], true);
}
