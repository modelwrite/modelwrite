// SPDX-License-Identifier: AGPL-3.0-or-later
//! The OpenAI-compatible live backend, proven against a loopback server that stands in for
//! the organisation's fleet. It asserts the REQUEST SHAPE the reasoner puts on the wire
//! (path, model, messages, and the bearer header only when a key is configured), and that a
//! well-formed answer becomes a validated proposal while a malformed one is refused with the
//! validator's own error - never trusted. No test here reaches a real endpoint.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

use agent::ProposedAction;
use okf::types::OkfRoot;
use serde_json::{json, Value};
use server::assist::{ModelReasoner, OpenAiReasoner};

/// What the loopback stub saw on the wire.
struct Captured {
    path: String,
    authorization: Option<String>,
    body: Value,
}

/// A minimal, one-request loopback server. It records the request line, the Authorization
/// header and the JSON body, then replies with the OpenAI-compatible chat-completion envelope
/// whose `choices[0].message.content` is `content`. Returns the base URL the reasoner should
/// be pointed at and a slot the test reads after the request.
fn serve_once(content: String) -> (String, Arc<Mutex<Option<Captured>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    let addr = listener.local_addr().expect("local address");
    let captured: Arc<Mutex<Option<Captured>>> = Arc::new(Mutex::new(None));
    let slot = captured.clone();
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept the one request");
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(10)))
            .ok();
        let mut buf = Vec::new();
        let mut chunk = [0u8; 8192];
        // Read the headers and the body the Content-Length declares.
        let (header_end, content_length) = loop {
            let n = stream.read(&mut chunk).expect("read request");
            assert!(n > 0, "client closed before sending a full request");
            buf.extend_from_slice(&chunk[..n]);
            if let Some(pos) = find_bytes(&buf, b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&buf[..pos]).to_string();
                let content_length = headers
                    .lines()
                    .filter_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .and_then(|v| v.trim().parse::<usize>().ok())
                    })
                    .next()
                    .unwrap_or(0);
                if buf.len() >= pos + 4 + content_length {
                    break (pos + 4, content_length);
                }
            }
        };
        let headers = String::from_utf8_lossy(&buf[..header_end - 4]).to_string();
        let body =
            String::from_utf8_lossy(&buf[header_end..header_end + content_length]).to_string();
        let path = headers
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .unwrap_or("")
            .to_string();
        let authorization = headers.lines().find_map(|line| {
            let (name, value) = line.split_once(':')?;
            if name.eq_ignore_ascii_case("authorization") {
                Some(value.trim().to_string())
            } else {
                None
            }
        });
        let body: Value = serde_json::from_str(&body).expect("request body must be JSON");
        *slot.lock().unwrap() = Some(Captured {
            path,
            authorization,
            body,
        });
        let response_body = json!({
            "choices": [{ "message": { "role": "assistant", "content": content } }]
        })
        .to_string();
        let response = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
            response_body.len(),
            response_body
        );
        stream
            .write_all(response.as_bytes())
            .expect("write response");
    });
    (format!("http://{}/v1", addr), captured)
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// A minimal, valid model the reasoner builds on; its exact content is irrelevant to the
/// wire shape but proves the request carries the current model for the model to read.
fn minimal_current() -> OkfRoot {
    serde_json::from_value(json!({
        "okf": "1.0",
        "project": "coffee",
        "summary": {
            "blocks": 0, "requirements": 0, "interfaces": 0, "signals": 0,
            "activities": 0, "graphNodes": 1, "graphEdges": 0
        },
        "stateMachine": { "name": "sm", "regions": [] },
        "graph": {
            "nodes": [{ "id": "root", "kind": "block", "name": "root", "stereotypes": [] }],
            "edges": []
        }
    }))
    .expect("minimal model deserialises")
}

fn wait_for_request(slot: &Arc<Mutex<Option<Captured>>>) -> Captured {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if let Some(captured) = slot.lock().unwrap().take() {
            return captured;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the loopback stub never saw a request"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

#[test]
fn the_openai_reasoner_posts_the_shape_and_validates_a_good_answer() {
    let good = json!({
        "changes": [{
            "action": "EditElement",
            "element": { "id": "heater-block", "name": "Heater Block", "kind": "block" },
            "requirement": null,
            "rationale": "add a heater block",
            "confidence": "High"
        }]
    });
    let (base_url, slot) = serve_once(good.to_string());
    let reasoner = OpenAiReasoner::new(base_url, "qwen3.8-27b-fp8", None);
    let current = minimal_current();

    let changes = reasoner
        .propose(
            "coffee",
            "add a heater block with a water inlet port",
            &current,
        )
        .expect("a well-formed answer must be accepted");

    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].action, ProposedAction::EditElement);
    assert_eq!(changes[0].element.as_ref().unwrap().id, "heater-block");

    let captured = wait_for_request(&slot);
    assert_eq!(captured.path, "/v1/chat/completions");
    assert_eq!(
        captured.authorization, None,
        "no key means no bearer header"
    );
    assert_eq!(captured.body["model"], "qwen3.8-27b-fp8");
    assert_eq!(captured.body["temperature"], 0);
    assert_eq!(
        captured.body["max_tokens"], 2048,
        "the answer must be bounded"
    );
    assert_eq!(
        captured.body["response_format"]["type"], "json_object",
        "the strict-JSON response_format must be kept"
    );
    let messages = captured.body["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0]["role"], "system");
    assert!(
        messages[0]["content"].as_str().unwrap().contains("changes"),
        "the strict-JSON system prompt must be the one sent"
    );
    assert_eq!(messages[1]["role"], "user");
    let user = messages[1]["content"].as_str().unwrap();
    assert!(user.contains("add a heater block with a water inlet port"));
    assert!(user.contains("coffee"));
    // The COMPACT inventory is sent, not the full document: the graph node count is reported
    // but the node's own id (and the state machine) are not.
    assert!(
        user.contains("\"graphNodes\":1"),
        "the inventory must carry the graph node count"
    );
    assert!(
        !user.contains("stateMachine"),
        "the full document must not be sent"
    );
    assert!(
        !user.contains("\"root\""),
        "graph node ids must not be sent, only the count"
    );
}

#[test]
fn the_openai_reasoner_sends_the_bearer_key_when_configured() {
    let good = json!({ "changes": [] });
    let (base_url, slot) = serve_once(good.to_string());
    let reasoner = OpenAiReasoner::new(base_url, "qwen3.8-27b-fp8", Some("secret".to_string()));

    reasoner
        .propose("coffee", "add a heater", &minimal_current())
        .expect("an empty change list is still a valid answer");

    let captured = wait_for_request(&slot);
    assert_eq!(captured.authorization.as_deref(), Some("Bearer secret"));
}

#[test]
fn the_openai_reasoner_refuses_a_malformed_proposal() {
    // Valid JSON, but an action that is not a ProposedAction variant.
    let malformed = json!({
        "changes": [{
            "action": "Edit",
            "element": null,
            "requirement": null,
            "rationale": "",
            "confidence": "High"
        }]
    });
    let (base_url, _slot) = serve_once(malformed.to_string());
    let reasoner = OpenAiReasoner::new(base_url, "qwen3.8-27b-fp8", None);

    let err = reasoner
        .propose("coffee", "add a heater", &minimal_current())
        .expect_err("a malformed proposal must be refused");
    assert!(
        err.message.contains("malformed proposal"),
        "the refusal must name what was malformed, got: {}",
        err.message
    );
}

#[test]
fn the_openai_reasoner_refuses_a_non_json_answer() {
    let (base_url, _slot) = serve_once("this is not json at all".to_string());
    let reasoner = OpenAiReasoner::new(base_url, "qwen3.8-27b-fp8", None);

    let err = reasoner
        .propose("coffee", "add a heater", &minimal_current())
        .expect_err("a non-JSON answer must be refused");
    assert!(
        err.message.contains("malformed JSON"),
        "the refusal must name what was malformed, got: {}",
        err.message
    );
}

#[test]
fn the_openai_reasoner_extracts_json_from_a_fenced_text_answer() {
    // The model ignored response_format and wrapped the JSON in a markdown fence with prose.
    let fenced = r#"Sure, here it is:
```json
{"changes":[{"action":"EditElement","element":{"id":"heater-block"},"requirement":null,"rationale":"add a heater","confidence":"High"}]}
```
Hope that helps."#;
    let (base_url, _slot) = serve_once(fenced.to_string());
    let reasoner = OpenAiReasoner::new(base_url, "qwen3.8-27b-fp8", None);

    let changes = reasoner
        .propose("coffee", "add a heater", &minimal_current())
        .expect("a fenced JSON answer must be parsed defensively, not refused");
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].element.as_ref().unwrap().id, "heater-block");
}
