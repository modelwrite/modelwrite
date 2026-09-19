// SPDX-License-Identifier: AGPL-3.0-or-later
//! Repository mode, proven without a live service or a real network call.
//!
//! Every test here drives the MCP tools through an INJECTED transport (an in-memory
//! recording transport) or, for the real client, a LOOPBACK server bound to an ephemeral
//! 127.0.0.1 port. No test reaches a running modelwrite service and no test dials an external
//! host, so this suite passes on a build machine with no network.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use mcp::repository::{HttpResponse, Repository, Transport};
use serde_json::{json, Value};

/// One request observed on the wire: method, full URL, the Authorization header value and the
/// body. The read-and-propose-only proof is exactly this log.
#[derive(Debug, Clone)]
struct Recorded {
    method: String,
    url: String,
    authorization: Option<String>,
    body: Option<String>,
}

/// A canned route: (method, path) -> (status, body).
type Route<'a> = ((&'a str, &'a str), (u16, &'a str));
/// The repository plus the wire log its transport wrote.
type RepoAndLog = (Repository, Arc<Mutex<Vec<Recorded>>>);

struct RecordingTransport {
    routes: HashMap<(String, String), (u16, String)>,
    log: Arc<Mutex<Vec<Recorded>>>,
}

impl Transport for RecordingTransport {
    fn request(
        &self,
        method: &str,
        url: &str,
        authorization: Option<&str>,
        body: Option<&str>,
    ) -> Result<HttpResponse, String> {
        let path = path_of(url);
        self.log.lock().unwrap().push(Recorded {
            method: method.to_string(),
            url: url.to_string(),
            authorization: authorization.map(str::to_string),
            body: body.map(str::to_string),
        });
        let key = (method.to_string(), path.clone());
        match self.routes.get(&key) {
            Some((status, body)) => Ok(HttpResponse {
                status: *status,
                body: body.clone(),
            }),
            None => Ok(HttpResponse {
                status: 500,
                body: format!("no canned answer for {} {}", method, path),
            }),
        }
    }
}

fn path_of(url: &str) -> String {
    match url.find("://") {
        Some(i) => {
            let rest = &url[i + 3..];
            match rest.find('/') {
                Some(j) => rest[j..].to_string(),
                None => "/".to_string(),
            }
        }
        None => url.to_string(),
    }
}

/// A repository wired to the recording transport, plus the log the transport writes.
fn repo_with(routes: Vec<Route<'_>>) -> RepoAndLog {
    let log = Arc::new(Mutex::new(Vec::new()));
    let mut map = HashMap::new();
    for ((method, path), (status, body)) in routes {
        map.insert(
            (method.to_string(), path.to_string()),
            (status, body.to_string()),
        );
    }
    let transport = RecordingTransport {
        routes: map,
        log: log.clone(),
    };
    let repo = Repository::configured_with_transport(
        "http://modelwrite.test",
        "agent-tok",
        Box::new(transport),
    );
    (repo, log)
}

/// Drive one tool through the full JSON-RPC envelope and return the parsed response.
fn call(repo: &Repository, name: &str, arguments: Value) -> Value {
    let line = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": { "name": name, "arguments": arguments }
    })
    .to_string();
    let out = mcp::handle_request_with(&line, repo);
    serde_json::from_str(&out).expect("a valid JSON-RPC response")
}

/// The text a successful tool returns, parsed as JSON.
fn tool_json(resp: &Value) -> Value {
    assert!(
        !resp["result"]["isError"].as_bool().unwrap(),
        "tool failed: {}",
        resp
    );
    let text = resp["result"]["content"][0]["text"].as_str().unwrap();
    serde_json::from_str(text).expect("the tool result must be JSON")
}

#[test]
fn repository_mode_reads_a_project_a_model_and_a_loss_report() {
    let okf = test_support::load_okf_expected();
    let (repo, _log) = repo_with(vec![
        (
            ("GET", "/projects"),
            (
                200,
                r#"[{"name":"coffee","createdAt":"2026-01-01T00:00:00Z"}]"#,
            ),
        ),
        (
            ("GET", "/projects/coffee/branches"),
            (200, r#"[{"name":"main","tip":"abc123"}]"#),
        ),
        (
            ("GET", "/projects/coffee/commits?branch=main"),
            (
                200,
                r#"[{"hash":"abc123","author":"alex","message":"first","branch":"main","parents":[],"okfHash":"h","createdAt":"2026-01-01T00:00:00Z","provenance":{"kind":"authored"}}]"#,
            ),
        ),
        (
            ("GET", "/projects/coffee/commits/abc123"),
            (200, okf.as_str()),
        ),
        (
            ("GET", "/projects/coffee/commits/abc123/record"),
            (
                200,
                r#"{"hash":"abc123","project":"coffee","branch":"main","parents":[],"okfHash":"h","author":"alex","message":"first","createdAt":"2026-01-01T00:00:00Z","provenance":{"kind":"authored"}}"#,
            ),
        ),
        (
            ("GET", "/projects/coffee/import/artifact0000/report"),
            (
                200,
                r#"{"artifactHash":"artifact0000","bindingId":"sysml-v1-xmi","bindingVersion":"2.4","lossReport":{"binding":{"id":"sysml-v1-xmi","version":"2.4"},"mappings":[]},"fidelity":{"equal":true,"missingElements":[]}}"#,
            ),
        ),
        (
            ("POST", "/projects/coffee/proposals"),
            (
                201,
                r#"{"id":"prop1","project":"coffee","agent":"loss-report-resolver","taskGoal":"resolve the blocking losses","artifactHash":"artifact0000","binding":"sysml-v1-xmi@2.4"}"#,
            ),
        ),
    ]);

    // A project is listed.
    let projects = tool_json(&call(&repo, "repo.projects", json!({})));
    assert_eq!(projects[0]["name"], "coffee");

    // A model is read, with its commit record (provenance).
    let read = tool_json(&call(
        &repo,
        "repo.read",
        json!({ "project": "coffee", "hash": "abc123" }),
    ));
    assert_eq!(read["commit"]["hash"], "abc123");
    assert_eq!(read["commit"]["provenance"]["kind"], "authored");
    assert!(
        read["document"].get("project").is_some(),
        "a model is returned"
    );

    // A migration's loss report and fidelity measurement are returned.
    let report = tool_json(&call(
        &repo,
        "repo.importReport",
        json!({ "project": "coffee", "artifactHash": "artifact0000" }),
    ));
    assert_eq!(report["artifactHash"], "artifact0000");
    assert_eq!(report["bindingId"], "sysml-v1-xmi");
    assert_eq!(report["lossReport"]["binding"]["id"], "sysml-v1-xmi");
    assert_eq!(report["fidelity"]["equal"], true);

    // Two commits are diffed and gated locally, returning the verdict and the evidence.
    let gate = tool_json(&call(
        &repo,
        "repo.diff",
        json!({ "project": "coffee", "reference": "abc123", "candidate": "abc123" }),
    ));
    assert_eq!(gate["passed"], true);
    assert_eq!(gate["evidence"]["passed"], true);

    // The loop closes: the agent records a proposal and the service answers with its id.
    let proposal = tool_json(&call(
        &repo,
        "repo.propose",
        json!({
            "project": "coffee",
            "reviewArtifact": {
                "agent": "loss-report-resolver",
                "task": { "goal": "resolve the blocking losses", "material": { "LossReport": { "artifact_hash": "artifact0000", "binding": { "id": "sysml-v1-xmi", "version": "2.4" } } } }
            }
        }),
    ));
    assert_eq!(proposal["id"], "prop1");
    assert_eq!(proposal["agent"], "loss-report-resolver");
}

#[test]
fn repository_mode_is_read_and_propose_only_on_the_wire() {
    let okf = test_support::load_okf_expected();
    let (repo, log) = repo_with(vec![
        (("GET", "/projects"), (200, "[]")),
        (("GET", "/projects/coffee/branches"), (200, "[]")),
        (("GET", "/projects/coffee/commits?branch=main"), (200, "[]")),
        (
            ("GET", "/projects/coffee/commits/abc123"),
            (200, okf.as_str()),
        ),
        (
            ("GET", "/projects/coffee/commits/abc123/record"),
            (200, r#"{"hash":"abc123","provenance":{"kind":"authored"}}"#),
        ),
        (
            ("GET", "/projects/coffee/commits/abc123/checks"),
            (200, r#"{"commit":"abc123","checked":false,"checks":[]}"#),
        ),
        (
            ("GET", "/projects/coffee/import/artifact0000/report"),
            (
                200,
                r#"{"artifactHash":"artifact0000","bindingId":"x","bindingVersion":"1","lossReport":{},"fidelity":{}}"#,
            ),
        ),
        (("GET", "/projects/coffee/audit"), (200, "[]")),
        (
            ("POST", "/projects/coffee/proposals"),
            (201, r#"{"id":"p1"}"#),
        ),
    ]);

    // Drive every repository tool, so the log is a complete picture of what the toolset can
    // do on the wire.
    call(&repo, "repo.projects", json!({}));
    call(&repo, "repo.branches", json!({ "project": "coffee" }));
    call(&repo, "repo.commits", json!({ "project": "coffee" }));
    call(
        &repo,
        "repo.read",
        json!({ "project": "coffee", "hash": "abc123" }),
    );
    call(
        &repo,
        "repo.importReport",
        json!({ "project": "coffee", "artifactHash": "artifact0000" }),
    );
    call(
        &repo,
        "repo.diff",
        json!({ "project": "coffee", "reference": "abc123", "candidate": "abc123" }),
    );
    call(&repo, "repo.audit", json!({ "project": "coffee" }));
    call(
        &repo,
        "repo.checks",
        json!({ "project": "coffee", "hash": "abc123" }),
    );
    call(
        &repo,
        "repo.propose",
        json!({ "project": "coffee", "reviewArtifact": { "task": { "goal": "g" } } }),
    );

    let log = log.lock().unwrap();
    let mut posts = 0usize;
    for rec in log.iter() {
        assert_eq!(
            rec.authorization.as_deref(),
            Some("Bearer agent-tok"),
            "every request must carry the configured bearer token"
        );
        if rec.method == "POST" {
            assert!(
                rec.body.as_deref().unwrap_or("").contains("task"),
                "the propose body must carry the review artifact"
            );
        }
        if rec.method == "POST" {
            posts += 1;
            assert!(
                rec.url.ends_with("/projects/coffee/proposals"),
                "the only mutating route may be the propose route, got {} {}",
                rec.method,
                rec.url
            );
        } else {
            assert_eq!(
                rec.method, "GET",
                "a repository tool issued a non-GET method: {} {}",
                rec.method, rec.url
            );
        }
        assert!(
            !rec.url.contains("/accept") && !rec.url.contains("/refuse"),
            "no acceptance or refusal route may be called: {}",
            rec.url
        );
    }
    assert_eq!(posts, 1, "exactly one propose, nothing else mutates");
}

#[test]
fn a_403_refusal_is_surfaced_as_a_tool_error_not_a_crash() {
    let (repo, log) = repo_with(vec![(
        ("GET", "/projects/coffee/commits/abc123/checks"),
        (403, r#"{"error":"write or review permission required"}"#),
    )]);

    let resp = call(
        &repo,
        "repo.checks",
        json!({ "project": "coffee", "hash": "abc123" }),
    );
    // The JSON-RPC envelope stays a success; the refusal is a tool error the agent reads.
    assert!(
        resp.get("error").is_none(),
        "unexpected transport error: {}",
        resp
    );
    assert_eq!(resp["result"]["isError"], true);
    let text = resp["result"]["content"][0]["text"].as_str().unwrap();
    assert!(
        text.contains("HTTP 403") && text.contains("write or review permission required"),
        "the refusal must surface the status and the service's message: {}",
        text
    );
    // Exactly one request was made: a refusal is never retried.
    assert_eq!(log.lock().unwrap().len(), 1);
}

#[test]
fn repository_tools_without_configuration_answer_not_configured_and_make_no_network_call() {
    // The air-gapped default: no configuration, no transport, so no socket can be opened. The
    // repository tool answers "not configured" from the dispatcher before any I/O exists.
    let repo = Repository::disabled();
    assert!(!repo.is_configured());

    let resp = call(&repo, "repo.projects", json!({}));
    assert!(
        resp.get("error").is_none(),
        "unexpected transport error: {}",
        resp
    );
    assert_eq!(resp["result"]["isError"], true);
    let text = resp["result"]["content"][0]["text"].as_str().unwrap();
    assert!(
        text.contains("not configured")
            && text.contains("MW_MCP_SERVICE_URL")
            && text.contains("MW_MCP_TOKEN"),
        "the refusal must name the opt-in variables and restate the air gap: {}",
        text
    );

    // The default entry point behaves identically: tools are LISTED (the contract is stable)
    // but inert.
    let listed = mcp::handle_request(r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#);
    let listed: Value = serde_json::from_str(&listed).unwrap();
    let names: Vec<&str> = listed["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert!(
        names.contains(&"repo.projects"),
        "repo tools stay listed without config"
    );
}

#[test]
fn tcp_transport_speaks_http_to_a_loopback_server() {
    use mcp::repository::TcpTransport;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind an ephemeral loopback port");
    let addr = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        // Read until the end of the request headers (the client's GET has no body).
        let mut buf: Vec<u8> = Vec::new();
        let mut tmp = [0u8; 512];
        loop {
            let n = stream.read(&mut tmp).unwrap();
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&tmp[..n]);
            if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                break;
            }
        }
        let request = String::from_utf8_lossy(&buf);
        assert!(
            request.starts_with("GET /projects/coffee HTTP/1.1"),
            "unexpected request: {}",
            request
        );
        assert!(
            request.contains("Authorization: Bearer agent-tok"),
            "the bearer token must be sent as the Authorization header: {}",
            request
        );
        let body = r#"[{"name":"coffee"}]"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\n\r\n{}",
            body.len(),
            body
        );
        stream.write_all(response.as_bytes()).unwrap();
    });

    let transport = TcpTransport::default();
    let resp = transport
        .request(
            "GET",
            &format!("http://127.0.0.1:{}/projects/coffee", addr.port()),
            Some("Bearer agent-tok"),
            None,
        )
        .expect("the loopback request succeeds");
    server.join().unwrap();

    assert_eq!(resp.status, 200);
    let projects: Value = serde_json::from_str(&resp.body).unwrap();
    assert_eq!(projects[0]["name"], "coffee");
}
// === TLS over a loopback listener ===

// A self-signed CA and a loopback server certificate it signed, generated once and embedded
// so the TLS tests need no certificate tooling, no network and no certificate dev-dependency.
// The server cert names 127.0.0.1 (and localhost); both are valid 2020-2120.
const CA_CERT_DER: &[u8] = &[
    0x30, 0x82, 0x01, 0x6f, 0x30, 0x82, 0x01, 0x16, 0xa0, 0x03, 0x02, 0x01, 0x02, 0x02, 0x14, 0x37,
    0x8c, 0x5b, 0xaf, 0x20, 0xcc, 0x8e, 0x60, 0xfd, 0x1d, 0x9b, 0xea, 0xf9, 0x2c, 0x24, 0x96, 0x0d,
    0x4a, 0xdb, 0x95, 0x30, 0x0a, 0x06, 0x08, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x04, 0x03, 0x02, 0x30,
    0x1d, 0x31, 0x1b, 0x30, 0x19, 0x06, 0x03, 0x55, 0x04, 0x03, 0x0c, 0x12, 0x6d, 0x6f, 0x64, 0x65,
    0x6c, 0x77, 0x72, 0x69, 0x74, 0x65, 0x20, 0x74, 0x65, 0x73, 0x74, 0x20, 0x43, 0x41, 0x30, 0x20,
    0x17, 0x0d, 0x32, 0x30, 0x30, 0x31, 0x30, 0x31, 0x30, 0x30, 0x30, 0x30, 0x30, 0x30, 0x5a, 0x18,
    0x0f, 0x32, 0x31, 0x32, 0x30, 0x30, 0x31, 0x30, 0x31, 0x30, 0x30, 0x30, 0x30, 0x30, 0x30, 0x5a,
    0x30, 0x1d, 0x31, 0x1b, 0x30, 0x19, 0x06, 0x03, 0x55, 0x04, 0x03, 0x0c, 0x12, 0x6d, 0x6f, 0x64,
    0x65, 0x6c, 0x77, 0x72, 0x69, 0x74, 0x65, 0x20, 0x74, 0x65, 0x73, 0x74, 0x20, 0x43, 0x41, 0x30,
    0x59, 0x30, 0x13, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01, 0x06, 0x08, 0x2a, 0x86,
    0x48, 0xce, 0x3d, 0x03, 0x01, 0x07, 0x03, 0x42, 0x00, 0x04, 0x9b, 0x77, 0xec, 0xfe, 0x8b, 0xa7,
    0x4f, 0x9b, 0xeb, 0x9c, 0x40, 0x3d, 0xfd, 0x2c, 0xcd, 0xe6, 0x94, 0x68, 0x92, 0x93, 0x91, 0xfd,
    0x27, 0x2d, 0xf9, 0xb7, 0x59, 0x8b, 0x72, 0xf6, 0xf7, 0x70, 0x83, 0x11, 0x80, 0x9e, 0xaf, 0xaf,
    0x08, 0x87, 0x1a, 0x35, 0xb8, 0x12, 0xa0, 0x3a, 0xab, 0x3f, 0xa4, 0xaa, 0xf0, 0x77, 0xdd, 0xc6,
    0x75, 0xd4, 0x7d, 0x7b, 0x12, 0x1a, 0x9d, 0xbf, 0x59, 0xa5, 0xa3, 0x32, 0x30, 0x30, 0x30, 0x1d,
    0x06, 0x03, 0x55, 0x1d, 0x0e, 0x04, 0x16, 0x04, 0x14, 0x68, 0x3e, 0x4b, 0x20, 0xc1, 0x5a, 0xe6,
    0x4e, 0x82, 0xa4, 0xdc, 0xe4, 0x1a, 0xc2, 0xb4, 0x0e, 0x37, 0x6a, 0xbb, 0x93, 0x30, 0x0f, 0x06,
    0x03, 0x55, 0x1d, 0x13, 0x01, 0x01, 0xff, 0x04, 0x05, 0x30, 0x03, 0x01, 0x01, 0xff, 0x30, 0x0a,
    0x06, 0x08, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x04, 0x03, 0x02, 0x03, 0x47, 0x00, 0x30, 0x44, 0x02,
    0x20, 0x63, 0x29, 0x5f, 0x31, 0xdb, 0xbb, 0x1c, 0x7d, 0x33, 0xd6, 0xaf, 0xc3, 0xcb, 0xaf, 0x67,
    0x92, 0x9f, 0x4a, 0x88, 0xea, 0x83, 0xba, 0xac, 0xd1, 0xaa, 0x65, 0x65, 0x06, 0x77, 0x27, 0x6e,
    0xb8, 0x02, 0x20, 0x43, 0xb4, 0x17, 0xbe, 0xbe, 0x3e, 0x78, 0x53, 0x85, 0x58, 0xf6, 0x1e, 0xfe,
    0xd2, 0x51, 0xa5, 0xd9, 0xb7, 0x2c, 0xa7, 0xfd, 0x7c, 0x79, 0x25, 0x32, 0xb8, 0x0c, 0x12, 0xef,
    0xeb, 0x3a, 0x84,
];

const SERVER_CERT_DER: &[u8] = &[
    0x30, 0x82, 0x01, 0x52, 0x30, 0x81, 0xf9, 0xa0, 0x03, 0x02, 0x01, 0x02, 0x02, 0x14, 0x03, 0xf4,
    0xf9, 0x57, 0x09, 0x8c, 0xd5, 0x18, 0x57, 0xd6, 0x92, 0xb6, 0x41, 0x9d, 0xec, 0x21, 0x53, 0x6d,
    0xa2, 0x35, 0x30, 0x0a, 0x06, 0x08, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x04, 0x03, 0x02, 0x30, 0x1d,
    0x31, 0x1b, 0x30, 0x19, 0x06, 0x03, 0x55, 0x04, 0x03, 0x0c, 0x12, 0x6d, 0x6f, 0x64, 0x65, 0x6c,
    0x77, 0x72, 0x69, 0x74, 0x65, 0x20, 0x74, 0x65, 0x73, 0x74, 0x20, 0x43, 0x41, 0x30, 0x20, 0x17,
    0x0d, 0x32, 0x30, 0x30, 0x31, 0x30, 0x31, 0x30, 0x30, 0x30, 0x30, 0x30, 0x30, 0x5a, 0x18, 0x0f,
    0x32, 0x31, 0x32, 0x30, 0x30, 0x31, 0x30, 0x31, 0x30, 0x30, 0x30, 0x30, 0x30, 0x30, 0x5a, 0x30,
    0x14, 0x31, 0x12, 0x30, 0x10, 0x06, 0x03, 0x55, 0x04, 0x03, 0x0c, 0x09, 0x6c, 0x6f, 0x63, 0x61,
    0x6c, 0x68, 0x6f, 0x73, 0x74, 0x30, 0x59, 0x30, 0x13, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d,
    0x02, 0x01, 0x06, 0x08, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07, 0x03, 0x42, 0x00, 0x04,
    0x74, 0xea, 0xe7, 0x5b, 0xad, 0x68, 0xb2, 0x4d, 0x4b, 0x5b, 0x3d, 0xe3, 0xb1, 0x9f, 0x09, 0x34,
    0x4d, 0x85, 0x2d, 0x9d, 0x31, 0xa5, 0xa8, 0x72, 0x1f, 0x35, 0xa8, 0xa9, 0x28, 0x53, 0x63, 0x19,
    0x91, 0x52, 0x62, 0x36, 0x2c, 0xb3, 0x26, 0x8f, 0x0c, 0xc7, 0xa1, 0x60, 0xb2, 0x3c, 0xbd, 0x07,
    0xe6, 0x47, 0xb3, 0x94, 0x38, 0xa9, 0x0c, 0x3e, 0x0c, 0xbf, 0xe0, 0x41, 0x3b, 0x00, 0xc3, 0x76,
    0xa3, 0x1e, 0x30, 0x1c, 0x30, 0x1a, 0x06, 0x03, 0x55, 0x1d, 0x11, 0x04, 0x13, 0x30, 0x11, 0x82,
    0x09, 0x6c, 0x6f, 0x63, 0x61, 0x6c, 0x68, 0x6f, 0x73, 0x74, 0x87, 0x04, 0x7f, 0x00, 0x00, 0x01,
    0x30, 0x0a, 0x06, 0x08, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x04, 0x03, 0x02, 0x03, 0x48, 0x00, 0x30,
    0x45, 0x02, 0x20, 0x11, 0x25, 0x3e, 0xab, 0x66, 0x73, 0x2a, 0x34, 0xb6, 0x3e, 0xef, 0x15, 0xb7,
    0xf9, 0xcc, 0x4a, 0x38, 0x97, 0x8d, 0xfe, 0x07, 0x14, 0x37, 0xbf, 0x50, 0x82, 0x82, 0x21, 0x44,
    0x17, 0x9b, 0x1d, 0x02, 0x21, 0x00, 0x90, 0xd5, 0x6d, 0x99, 0x9c, 0x03, 0x28, 0x8d, 0x67, 0x3f,
    0x1e, 0x34, 0x97, 0xfc, 0x07, 0xfb, 0x30, 0xc4, 0xab, 0x59, 0x09, 0x29, 0xbf, 0x00, 0xd9, 0xa4,
    0xd2, 0xde, 0x18, 0x40, 0x88, 0x64,
];

const SERVER_KEY_DER: &[u8] = &[
    0x30, 0x81, 0x87, 0x02, 0x01, 0x00, 0x30, 0x13, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02,
    0x01, 0x06, 0x08, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07, 0x04, 0x6d, 0x30, 0x6b, 0x02,
    0x01, 0x01, 0x04, 0x20, 0xc9, 0x3c, 0x44, 0xde, 0x1d, 0x0a, 0xf8, 0xf0, 0x47, 0x64, 0x71, 0x83,
    0x66, 0x24, 0xeb, 0xa6, 0xab, 0x42, 0xf4, 0x8d, 0xc7, 0xd9, 0x55, 0xbd, 0x52, 0x8f, 0xb4, 0xa6,
    0xd5, 0x1a, 0xd2, 0x2f, 0xa1, 0x44, 0x03, 0x42, 0x00, 0x04, 0x74, 0xea, 0xe7, 0x5b, 0xad, 0x68,
    0xb2, 0x4d, 0x4b, 0x5b, 0x3d, 0xe3, 0xb1, 0x9f, 0x09, 0x34, 0x4d, 0x85, 0x2d, 0x9d, 0x31, 0xa5,
    0xa8, 0x72, 0x1f, 0x35, 0xa8, 0xa9, 0x28, 0x53, 0x63, 0x19, 0x91, 0x52, 0x62, 0x36, 0x2c, 0xb3,
    0x26, 0x8f, 0x0c, 0xc7, 0xa1, 0x60, 0xb2, 0x3c, 0xbd, 0x07, 0xe6, 0x47, 0xb3, 0x94, 0x38, 0xa9,
    0x0c, 0x3e, 0x0c, 0xbf, 0xe0, 0x41, 0x3b, 0x00, 0xc3, 0x76,
];

/// Bind a loopback TLS listener that answers one GET with a canned JSON body. It returns the
/// bound address and the server thread. The thread exits quietly if the handshake fails (the
/// verification-on test aborts it) or after answering the one request.
fn spawn_tls_server() -> (std::net::SocketAddr, std::thread::JoinHandle<()>) {
    use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
    use rustls::{ServerConfig, ServerConnection, StreamOwned};
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::Arc;

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind an ephemeral loopback port");
    let addr = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let Ok((stream, _)) = listener.accept() else {
            return;
        };
        let config = Arc::new(
            ServerConfig::builder()
                .with_no_client_auth()
                .with_single_cert(
                    vec![
                        CertificateDer::from(SERVER_CERT_DER.to_vec()),
                        CertificateDer::from(CA_CERT_DER.to_vec()),
                    ],
                    PrivateKeyDer::from(PrivatePkcs8KeyDer::from(SERVER_KEY_DER.to_vec())),
                )
                .expect("the embedded loopback certificate parses"),
        );
        let conn = match ServerConnection::new(config) {
            Ok(c) => c,
            Err(_) => return,
        };
        let mut tls = StreamOwned::new(conn, stream);
        let mut buf: Vec<u8> = Vec::new();
        let mut tmp = [0u8; 512];
        loop {
            match tls.read(&mut tmp) {
                Ok(0) => return,
                Ok(n) => {
                    buf.extend_from_slice(&tmp[..n]);
                    if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                        break;
                    }
                }
                Err(_) => return, // the verification-on client aborted the handshake
            }
        }
        assert!(buf.starts_with(b"GET /projects/coffee HTTP/1.1"));
        let body = r#"[{"name":"coffee"}]"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\n\r\n{}",
            body.len(),
            body
        );
        let _ = tls.write_all(response.as_bytes());
        // A clean TLS close so the client's read terminates on close_notify.
        tls.conn.send_close_notify();
        let _ = tls.flush();
    });
    (addr, server)
}

#[test]
fn tcp_transport_verifies_tls_and_refuses_a_self_signed_certificate() {
    use mcp::repository::TcpTransport;

    let (addr, server) = spawn_tls_server();
    // The default transport trusts only the bundled webpki roots. There is no flag to disable
    // verification, so a self-signed certificate must fail the handshake - this test is the
    // proof that verification is on and that no unverified workaround exists.
    let transport = TcpTransport::default();
    let err = transport
        .request(
            "GET",
            &format!("https://127.0.0.1:{}/projects/coffee", addr.port()),
            Some("Bearer agent-tok"),
            None,
        )
        .err()
        .expect("a self-signed certificate must be refused, never accepted");
    server.join().unwrap();

    let lower = err.to_lowercase();
    assert!(
        lower.contains("tls") || lower.contains("handshake") || lower.contains("certificate"),
        "the refusal must name the TLS/certificate problem, got: {}",
        err
    );
}

#[test]
fn tcp_transport_speaks_https_to_a_loopback_server_with_a_trusted_ca() {
    use mcp::repository::TcpTransport;
    use rustls::pki_types::CertificateDer;

    let (addr, server) = spawn_tls_server();
    // Trust the loopback CA explicitly. This is adding a trust anchor, not disabling
    // verification: the peer must still chain to it, and a certificate outside the store is
    // still refused (the test above proves that).
    let mut roots = rustls::RootCertStore::empty();
    roots
        .add(CertificateDer::from(CA_CERT_DER.to_vec()))
        .unwrap();
    let transport = TcpTransport::with_roots(roots);
    let resp = transport
        .request(
            "GET",
            &format!("https://127.0.0.1:{}/projects/coffee", addr.port()),
            Some("Bearer agent-tok"),
            None,
        )
        .expect("the loopback TLS request succeeds against the trusted CA");
    server.join().unwrap();

    assert_eq!(resp.status, 200);
    let projects: Value = serde_json::from_str(&resp.body).unwrap();
    assert_eq!(projects[0]["name"], "coffee");
}
