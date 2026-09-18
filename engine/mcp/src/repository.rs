// SPDX-License-Identifier: AGPL-3.0-or-later
//! Repository mode: read-and-propose tools over the modelwrite HTTP API.
//!
//! The MCP server is a document processor by default and must stay usable with no network
//! at all: an air-gapped organisation depends on that. Repository mode is OPT-IN, enabled
//! only when BOTH [SERVICE_URL_VAR] (the running service's base URL) and [TOKEN_VAR]
//! (the bearer token to present) are set. With neither set, the server makes no network
//! call and every repository tool answers "not configured".
//!
//! The repository tools are READ AND PROPOSE ONLY. They reach the running service over the
//! same HTTP surface a human uses, authenticated with the configured token, and they are
//! refused exactly as a human is (401/403/404/...). No tool here calls a write route: the
//! only mutating tool is repo.propose, which records a proposal (the review permission),
//! never a commit. A write attempt - which an agent token cannot even authorise, and which
//! this crate has no code path to issue - is refused by the SERVICE with 403, and any 403
//! the service returns is surfaced here as a clear tool error, never a crash and never a
//! retry.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use serde_json::{json, Value};

/// Environment variable holding the running modelwrite service's base URL (e.g.
/// http://127.0.0.1:8080). Repository mode is enabled only when this AND [TOKEN_VAR] are
/// both set and non-empty.
pub const SERVICE_URL_VAR: &str = "MW_MCP_SERVICE_URL";
/// Environment variable holding the bearer token to present to the service. It should be an
/// agent token (viewer/reviewer roles): an agent proposes and a human commits.
pub const TOKEN_VAR: &str = "MW_MCP_TOKEN";

/// The refusal a repository tool returns when repository mode is not configured. It names the
/// two variables to set and restates the air-gap guarantee: without them, no network call is
/// made.
const NOT_CONFIGURED: &str = "repository mode is not configured: set MW_MCP_SERVICE_URL (the service base URL) and MW_MCP_TOKEN (the bearer token) to reach a repository; without them this server is a pure document processor and makes no network call";

/// The repository half of the MCP server. None when repository mode is not configured - the
/// pure document processor, which makes no network call - and Some when the opt-in
/// environment variables name a service to reach.
pub struct Repository {
    client: Option<RepoClient>,
}

impl Repository {
    /// The air-gapped default: no configuration, no client, no network. Every repository
    /// tool answers "not configured" and no socket is ever opened.
    pub fn disabled() -> Repository {
        Repository { client: None }
    }

    /// Read the configuration from the environment. Enabled only when BOTH variables are
    /// set; a half-configuration (one set, one not) is a startup error rather than a silent
    /// fall back to a document processor or a token-less client that can only 401.
    pub fn from_env() -> Result<Repository, String> {
        let url = std::env::var(SERVICE_URL_VAR)
            .ok()
            .filter(|v| !v.trim().is_empty());
        let token = std::env::var(TOKEN_VAR)
            .ok()
            .filter(|v| !v.trim().is_empty());
        Repository::from_config(url.as_deref(), token.as_deref())
    }

    /// The pure decision of whether repository mode is on, split out so it can be tested
    /// without touching the process environment.
    pub fn from_config(url: Option<&str>, token: Option<&str>) -> Result<Repository, String> {
        match (url, token) {
            (None, None) => Ok(Repository::disabled()),
            (Some(u), Some(t)) => Ok(Repository::configured(u, t)),
            (Some(_), None) => Err(format!(
                "{} is set but {} is not: configure both to enable repository mode, or neither to stay a pure document processor",
                SERVICE_URL_VAR, TOKEN_VAR
            )),
            (None, Some(_)) => Err(format!(
                "{} is set but {} is not: configure both to enable repository mode, or neither to stay a pure document processor",
                TOKEN_VAR, SERVICE_URL_VAR
            )),
        }
    }

    /// Enable repository mode with the real TCP transport.
    pub fn configured(url: &str, token: &str) -> Repository {
        Repository::configured_with_transport(url, token, Box::new(TcpTransport::default()))
    }

    /// Enable repository mode with an injected transport. This is the test seam: production
    /// code passes a [TcpTransport], tests pass a recording in-memory transport so a tool can
    /// be driven against canned answers without a live service or a real socket.
    pub fn configured_with_transport(
        url: &str,
        token: &str,
        transport: Box<dyn Transport>,
    ) -> Repository {
        Repository {
            client: Some(RepoClient {
                base_url: url.trim().trim_end_matches('/').to_string(),
                token: token.trim().to_string(),
                transport,
            }),
        }
    }

    pub fn is_configured(&self) -> bool {
        self.client.is_some()
    }

    /// Dispatch a repository tool, or refuse with "not configured" when repository mode is
    /// off. This is the only entry point lib.rs uses for the repository half of the tool
    /// table, so no caller can reach a client that was never configured.
    pub(crate) fn run_tool(&self, name: &str, args: &Value) -> Result<Value, String> {
        match &self.client {
            Some(client) => run_tool(name, args, client),
            None => Err(NOT_CONFIGURED.to_string()),
        }
    }
}

/// A single HTTP response: the status code and the body as text.
pub struct HttpResponse {
    pub status: u16,
    pub body: String,
}

/// The transport the repository client speaks over. Split from [RepoClient] so a test can
/// inject a recording transport and assert what actually went over the wire - the
/// read-and-propose-only proof depends on observing the real requests.
pub trait Transport {
    fn request(
        &self,
        method: &str,
        url: &str,
        authorization: Option<&str>,
        body: Option<&str>,
    ) -> Result<HttpResponse, String>;
}

/// The real transport: a plain-HTTP/1.1 client over std::net::TcpStream. It exists so
/// repository mode needs no HTTP crate and therefore no toolchain newer than the engine's
/// declared 1.75 floor. The modelwrite service speaks plain HTTP (no TLS), so a TLS client
/// would be dead weight here.
pub struct TcpTransport {
    connect_timeout: Duration,
    read_timeout: Duration,
}

impl Default for TcpTransport {
    fn default() -> Self {
        TcpTransport {
            connect_timeout: Duration::from_secs(5),
            read_timeout: Duration::from_secs(30),
        }
    }
}

impl Transport for TcpTransport {
    fn request(
        &self,
        method: &str,
        url: &str,
        authorization: Option<&str>,
        body: Option<&str>,
    ) -> Result<HttpResponse, String> {
        let (host, port, path) = parse_http_url(url)?;
        let addr = format!("{}:{}", host, port);
        let addr: std::net::SocketAddr = addr
            .parse()
            .map_err(|e| format!("invalid service address {}: {}", addr, e))?;
        let mut stream = TcpStream::connect_timeout(&addr, self.connect_timeout)
            .map_err(|e| format!("cannot reach the modelwrite service at {}: {}", url, e))?;
        stream
            .set_read_timeout(Some(self.read_timeout))
            .map_err(|e| format!("cannot configure the read timeout: {}", e))?;
        stream
            .set_write_timeout(Some(self.read_timeout))
            .map_err(|e| format!("cannot configure the write timeout: {}", e))?;

        let mut request = format!(
            "{} {} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\nAccept: application/json\r\n",
            method, path, host
        );
        if let Some(auth) = authorization {
            request.push_str(&format!("Authorization: {}\r\n", auth));
        }
        if let Some(b) = body {
            request.push_str("Content-Type: application/json\r\n");
            request.push_str(&format!("Content-Length: {}\r\n", b.len()));
        }
        request.push_str("\r\n");
        if let Some(b) = body {
            request.push_str(b);
        }

        stream
            .write_all(request.as_bytes())
            .map_err(|e| format!("cannot send the request to {}: {}", url, e))?;
        let mut response = Vec::new();
        stream
            .read_to_end(&mut response)
            .map_err(|e| format!("cannot read the response from {}: {}", url, e))?;
        parse_http_response(&response)
    }
}

/// Split an http://host[:port]/path URL into its parts. HTTPS is refused: the service speaks
/// plain HTTP and this client implements no TLS, so a scheme that requires TLS must fail
/// loudly rather than silently send the bearer token to a TLS port in the clear.
fn parse_http_url(url: &str) -> Result<(String, u16, String), String> {
    let rest = url.strip_prefix("http://").ok_or_else(|| {
        format!(
            "the service URL must be plain http:// (no TLS), got {}",
            url
        )
    })?;
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) if !h.is_empty() => (
            h.to_string(),
            p.parse::<u16>()
                .map_err(|e| format!("invalid port in {}: {}", url, e))?,
        ),
        _ => (authority.to_string(), 80),
    };
    if host.is_empty() {
        return Err(format!("invalid service URL {}", url));
    }
    Ok((host, port, path.to_string()))
}

/// Parse a buffered HTTP/1.1 response (status line, headers, body). The body length comes
/// from Content-Length, or Transfer-Encoding: chunked, or simply everything after the
/// headers when neither is present (the connection is closed by the server, as requested).
fn parse_http_response(data: &[u8]) -> Result<HttpResponse, String> {
    let split = data
        .windows(4)
        .position(|w| w == b"\r\n\r\n".as_slice())
        .ok_or_else(|| {
            "the service sent a malformed response (no header terminator)".to_string()
        })?;
    let head = &data[..split];
    let body_bytes = &data[split + 4..];
    let head_str = String::from_utf8_lossy(head);

    let mut lines = head_str.split("\r\n");
    let status_line = lines.next().unwrap_or("");
    let status: u16 = status_line
        .split_whitespace()
        .nth(1)
        .ok_or_else(|| "the service sent a malformed status line".to_string())?
        .parse()
        .map_err(|e| format!("the service sent an unreadable status code: {}", e))?;

    let mut content_length: Option<usize> = None;
    let mut chunked = false;
    for line in lines {
        if let Some((key, value)) = line.split_once(':') {
            match key.trim().to_ascii_lowercase().as_str() {
                "content-length" => content_length = value.trim().parse::<usize>().ok(),
                "transfer-encoding" => {
                    chunked = value.trim().to_ascii_lowercase().contains("chunked")
                }
                _ => {}
            }
        }
    }

    let body = if chunked {
        dechunk(body_bytes)?
    } else if let Some(n) = content_length {
        if body_bytes.len() < n {
            return Err("the service sent a truncated response body".to_string());
        }
        body_bytes[..n].to_vec()
    } else {
        body_bytes.to_vec()
    };
    Ok(HttpResponse {
        status,
        body: String::from_utf8_lossy(&body).into_owned(),
    })
}

/// Decode an HTTP/1.1 chunked body into its bytes. Chunk sizes are hexadecimal and may carry
/// a trailing ;extension; a zero size terminates. The modelwrite service answers the routes
/// these tools use with Content-Length, but decoding chunked here keeps the client honest
/// against any future route that streams.
fn dechunk(data: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    let mut pos = 0usize;
    loop {
        let line_end = data[pos..]
            .iter()
            .position(|&b| b == b'\n')
            .map(|i| pos + i)
            .ok_or_else(|| "the service sent a malformed chunked body".to_string())?;
        let size_line = std::str::from_utf8(&data[pos..line_end])
            .map_err(|_| "the service sent a non-UTF-8 chunk size".to_string())?;
        let size_line = size_line.trim_end_matches('\r');
        let size_token = size_line.split(';').next().unwrap_or("").trim();
        let size = usize::from_str_radix(size_token, 16)
            .map_err(|_| "the service sent an invalid chunk size".to_string())?;
        pos = line_end + 1;
        if size == 0 {
            return Ok(out);
        }
        if pos + size > data.len() {
            return Err("the service sent a truncated chunk".to_string());
        }
        out.extend_from_slice(&data[pos..pos + size]);
        pos += size;
        // Skip the CRLF that terminates the chunk data.
        if data.get(pos) == Some(&b'\r') && data.get(pos + 1) == Some(&b'\n') {
            pos += 2;
        }
    }
}

struct RepoClient {
    base_url: String,
    token: String,
    transport: Box<dyn Transport>,
}

impl RepoClient {
    fn request(
        &self,
        method: &str,
        path: &str,
        body: Option<&str>,
    ) -> Result<HttpResponse, String> {
        let url = format!("{}{}", self.base_url, path);
        let authorization = format!("Bearer {}", self.token);
        self.transport
            .request(method, &url, Some(&authorization), body)
    }

    /// Send a request and demand a 2xx JSON answer. A non-2xx answer is a refusal and is
    /// surfaced as an error carrying the status and the service's own message - never a
    /// crash, and never a retry: exactly one request is made.
    fn json(&self, method: &str, path: &str, body: Option<&str>) -> Result<Value, String> {
        let response = self.request(method, path, body)?;
        if (200..300).contains(&response.status) {
            if response.body.trim().is_empty() {
                return Ok(Value::Null);
            }
            return serde_json::from_str(&response.body).map_err(|e| {
                format!(
                    "the service answered HTTP {} but the body is not JSON: {}",
                    response.status, e
                )
            });
        }
        let detail = serde_json::from_str::<Value>(&response.body)
            .ok()
            .and_then(|v| v.get("error").and_then(|e| e.as_str()).map(str::to_string))
            .unwrap_or_else(|| response.body.clone());
        Err(format!(
            "the modelwrite service refused {} {} with HTTP {}: {}",
            method, path, response.status, detail
        ))
    }
}

/// Dispatch one repository tool against the configured client. This is the ONLY place
/// repository tools are implemented, and every route it issues is a read (GET) or the single
/// propose route (POST /projects/:project/proposals, the review permission).
fn run_tool(name: &str, args: &Value, client: &RepoClient) -> Result<Value, String> {
    match name {
        "repo.projects" => client.json("GET", "/projects", None),
        "repo.branches" => {
            let project = require_str(args, "project")?;
            client.json("GET", &format!("/projects/{}/branches", pct(project)), None)
        }
        "repo.commits" => {
            let project = require_str(args, "project")?;
            let branch = arg_str(args, "branch").unwrap_or("main");
            client.json(
                "GET",
                &format!("/projects/{}/commits?branch={}", pct(project), pct(branch)),
                None,
            )
        }
        "repo.read" => {
            let project = require_str(args, "project")?;
            let hash = require_str(args, "hash")?;
            let document = client.json(
                "GET",
                &format!("/projects/{}/commits/{}", pct(project), pct(hash)),
                None,
            )?;
            let record = client.json(
                "GET",
                &format!("/projects/{}/commits/{}/record", pct(project), pct(hash)),
                None,
            )?;
            Ok(json!({ "commit": record, "document": document }))
        }
        "repo.importReport" => {
            let project = require_str(args, "project")?;
            let artifact_hash = require_str(args, "artifactHash")?;
            client.json(
                "GET",
                &format!(
                    "/projects/{}/import/{}/report",
                    pct(project),
                    pct(artifact_hash)
                ),
                None,
            )
        }
        "repo.diff" => {
            let project = require_str(args, "project")?;
            let reference = require_str(args, "reference")?;
            let candidate = require_str(args, "candidate")?;
            let strict = args
                .get("strictCoverage")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let reference_doc = client.json(
                "GET",
                &format!("/projects/{}/commits/{}", pct(project), pct(reference)),
                None,
            )?;
            let candidate_doc = client.json(
                "GET",
                &format!("/projects/{}/commits/{}", pct(project), pct(candidate)),
                None,
            )?;
            let reference_root: okf::types::OkfRoot = serde_json::from_value(reference_doc)
                .map_err(|e| {
                    format!(
                        "reference commit {} is not an OKF document: {}",
                        reference, e
                    )
                })?;
            let candidate_root: okf::types::OkfRoot = serde_json::from_value(candidate_doc)
                .map_err(|e| {
                    format!(
                        "candidate commit {} is not an OKF document: {}",
                        candidate, e
                    )
                })?;
            // The verdict is computed LOCALLY by the engine from the two documents read from
            // the repository, exactly as the service's own gate computes it. Recording a run
            // is a write (the service's POST /gate), so an agent cannot record one; the
            // computed verdict and evidence are returned, never persisted.
            let diff = okf::diff::diff(&reference_root, &candidate_root);
            let outcome = gate::run(&reference_root, &candidate_root, strict);
            Ok(json!({
                "referenceHash": reference,
                "candidateHash": candidate,
                "passed": outcome.passed,
                "diff": diff,
                "evidence": outcome.evidence,
            }))
        }
        "repo.audit" => {
            let project = require_str(args, "project")?;
            let path = match args.get("limit").and_then(Value::as_i64) {
                Some(n) => format!("/projects/{}/audit?limit={}", pct(project), n),
                None => format!("/projects/{}/audit", pct(project)),
            };
            client.json("GET", &path, None)
        }
        "repo.checks" => {
            let project = require_str(args, "project")?;
            let hash = require_str(args, "hash")?;
            client.json(
                "GET",
                &format!("/projects/{}/commits/{}/checks", pct(project), pct(hash)),
                None,
            )
        }
        "repo.propose" => {
            let project = require_str(args, "project")?;
            let artifact = args
                .get("reviewArtifact")
                .ok_or_else(|| "missing reviewArtifact argument".to_string())?;
            let body_value = match artifact {
                Value::String(s) => serde_json::from_str::<Value>(s)
                    .map_err(|e| format!("reviewArtifact is not valid JSON: {}", e))?,
                other => other.clone(),
            };
            let body = serde_json::to_string(&body_value)
                .map_err(|e| format!("reviewArtifact cannot be serialised: {}", e))?;
            client.json(
                "POST",
                &format!("/projects/{}/proposals", pct(project)),
                Some(&body),
            )
        }
        _ => Err(format!("unknown repository tool: {}", name)),
    }
}

fn arg_str<'a>(args: &'a Value, name: &str) -> Option<&'a str> {
    args.get(name).and_then(Value::as_str)
}

fn require_str<'a>(args: &'a Value, name: &str) -> Result<&'a str, String> {
    arg_str(args, name)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("missing {} argument", name))
}

/// Percent-encode a URL path segment or query value. Project names, branch names and hashes
/// are already restricted to a safe charset by the service, but encoding here keeps a stray
/// segment from ever becoming a path separator or a query injection.
fn pct(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    struct TableTransport {
        routes: Vec<((String, String), HttpResponse)>,
        calls: Arc<Mutex<Vec<String>>>,
    }

    impl Transport for TableTransport {
        fn request(
            &self,
            method: &str,
            url: &str,
            _authorization: Option<&str>,
            _body: Option<&str>,
        ) -> Result<HttpResponse, String> {
            let path = path_of(url);
            self.calls
                .lock()
                .unwrap()
                .push(format!("{} {}", method, path));
            let response = self
                .routes
                .iter()
                .find(|((m, p), _)| m.as_str() == method && p.as_str() == path.as_str())
                .map(|(_, r)| HttpResponse {
                    status: r.status,
                    body: r.body.clone(),
                })
                .unwrap_or_else(|| HttpResponse {
                    status: 500,
                    body: format!("no canned answer for {} {}", method, path),
                });
            Ok(response)
        }
    }

    /// The path (and query) portion of a URL, e.g. "/projects/coffee/commits?branch=main".
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

    fn client_for(
        routes: Vec<((String, String), HttpResponse)>,
    ) -> (RepoClient, Arc<Mutex<Vec<String>>>) {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let transport = TableTransport {
            routes,
            calls: calls.clone(),
        };
        let client = RepoClient {
            base_url: "http://modelwrite.test".to_string(),
            token: "agent-tok".to_string(),
            transport: Box::new(transport),
        };
        (client, calls)
    }

    fn ok(body: &str) -> HttpResponse {
        HttpResponse {
            status: 200,
            body: body.to_string(),
        }
    }

    #[test]
    fn from_config_requires_both_variables() {
        assert!(!Repository::from_config(None, None).unwrap().is_configured());
        assert!(Repository::from_config(Some("http://x"), Some("t"))
            .unwrap()
            .is_configured());
        assert!(Repository::from_config(Some("http://x"), None).is_err());
        assert!(Repository::from_config(None, Some("t")).is_err());
    }

    #[test]
    fn a_write_attempt_is_refused_and_surfaced_once_not_retried() {
        let (client, calls) = client_for(vec![(
            ("POST".to_string(), "/projects/coffee/commits".to_string()),
            HttpResponse {
                status: 403,
                body: r#"{"error":"write permission required"}"#.to_string(),
            },
        )]);

        let err = client
            .json("POST", "/projects/coffee/commits", Some("{}"))
            .expect_err("a write attempt must be refused");
        assert!(
            err.contains("HTTP 403") && err.contains("write permission required"),
            "the refusal must surface the status and the service's message: {}",
            err
        );
        assert_eq!(
            calls.lock().unwrap().len(),
            1,
            "a refusal must never be retried"
        );
    }

    #[test]
    fn a_missing_token_is_surfaced_as_401_not_a_crash() {
        let (client, _) = client_for(vec![(
            ("GET".to_string(), "/projects".to_string()),
            HttpResponse {
                status: 401,
                body: r#"{"error":"missing bearer token"}"#.to_string(),
            },
        )]);
        let err = client
            .json("GET", "/projects", None)
            .expect_err("a 401 must be a refusal");
        assert!(
            err.contains("HTTP 401") && err.contains("missing bearer token"),
            "{}",
            err
        );
    }

    #[test]
    fn a_success_body_is_parsed_as_json() {
        let (client, _) = client_for(vec![(
            ("GET".to_string(), "/projects".to_string()),
            ok(r#"[{"name":"coffee"}]"#),
        )]);
        let value = client.json("GET", "/projects", None).unwrap();
        assert_eq!(value[0]["name"], "coffee");
    }

    #[test]
    fn parse_http_response_reads_content_length_and_chunked_bodies() {
        let content_length = b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello";
        let r = parse_http_response(content_length).unwrap();
        assert_eq!(r.status, 200);
        assert_eq!(r.body, "hello");

        let chunked =
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n0\r\n\r\n";
        let r = parse_http_response(chunked).unwrap();
        assert_eq!(r.body, "hello");
    }

    #[test]
    fn parse_http_url_refuses_https_and_parses_port_and_path() {
        let (host, port, path) = parse_http_url("http://127.0.0.1:8080/projects/coffee").unwrap();
        assert_eq!(host, "127.0.0.1");
        assert_eq!(port, 8080);
        assert_eq!(path, "/projects/coffee");

        let (_, port, _) = parse_http_url("http://example.com").unwrap();
        assert_eq!(port, 80);

        assert!(parse_http_url("https://example.com").is_err());
    }

    #[test]
    fn pct_encodes_path_unsafe_bytes() {
        assert_eq!(pct("coffee-machine.v2"), "coffee-machine.v2");
        assert_eq!(pct("a/b"), "a%2Fb");
    }
}
