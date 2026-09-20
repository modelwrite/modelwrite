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

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::time::Duration;

use rustls::pki_types::ServerName;
use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned};
use serde_json::{json, Value};

/// Environment variable holding the running modelwrite service's base URL, e.g.
/// http://127.0.0.1:8080 for a local service or https://trial.modelwrite.org for a deployed
/// one behind TLS. Repository mode is enabled only when this AND [TOKEN_VAR] are both set
/// and non-empty.
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

    /// Dispatch the one repository tool whose answer is raw bytes rather than JSON. It is kept
    /// separate from [Self::run_tool] because its result must reach the agent verbatim - byte
    /// for byte, never re-serialised through a JSON formatter - so the tool text IS the
    /// retained artifact.
    pub(crate) fn run_raw_tool(&self, name: &str, args: &Value) -> Result<String, String> {
        match &self.client {
            Some(client) => run_raw_tool(name, args, client),
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

/// The URL scheme the repository client may speak.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Scheme {
    Http,
    Https,
}

/// The parts of a parsed service URL: scheme, host, port and path.
struct UrlParts {
    scheme: Scheme,
    host: String,
    port: u16,
    path: String,
}

/// The real transport: a hand-written HTTP/1.1 client over std::net::TcpStream, upgraded to
/// TLS by the pure-Rust rustls stack (ring provider, bundled webpki roots) for https:// URLs.
/// It needs no HTTP crate and no OpenSSL, so the engine's declared 1.75 floor stays reachable.
/// Certificate verification is ALWAYS on and there is no knob anywhere to disable it: a peer
/// that does not chain to a trusted root fails the handshake with a clear error, never a
/// silent downgrade and never a retry.
pub struct TcpTransport {
    connect_timeout: Duration,
    read_timeout: Duration,
    tls: Arc<ClientConfig>,
}

impl Default for TcpTransport {
    fn default() -> Self {
        TcpTransport {
            connect_timeout: Duration::from_secs(5),
            read_timeout: Duration::from_secs(30),
            tls: Arc::new(tls_client_config(bundled_roots())),
        }
    }
}

impl TcpTransport {
    /// Build a transport whose TLS trust anchors are the given roots. This is the hook for a
    /// private CA and the test seam for a loopback TLS listener: the roots are ADDED to
    /// verification, never a replacement for it, so a peer must still chain to one of them.
    /// It is NOT a way to disable verification - there is none.
    pub fn with_roots(roots: RootCertStore) -> TcpTransport {
        TcpTransport {
            connect_timeout: Duration::from_secs(5),
            read_timeout: Duration::from_secs(30),
            tls: Arc::new(tls_client_config(roots)),
        }
    }

    /// Resolve the host and connect to the first address that accepts, applying the read and
    /// write timeouts. A hostname (trial.modelwrite.org) resolves through the system resolver
    /// exactly as an IP literal does, so a TLS deployment behind a reverse proxy is reachable.
    fn connect(&self, parts: &UrlParts) -> Result<TcpStream, String> {
        let addrs = (parts.host.as_str(), parts.port)
            .to_socket_addrs()
            .map_err(|e| format!("cannot resolve the service host {}: {}", parts.host, e))?;
        let mut last: Option<std::io::Error> = None;
        for addr in addrs {
            match TcpStream::connect_timeout(&addr, self.connect_timeout) {
                Ok(stream) => return self.with_timeouts(stream, &parts.host),
                Err(e) => last = Some(e),
            }
        }
        Err(format!(
            "cannot reach the modelwrite service at {}:{}: {}",
            parts.host,
            parts.port,
            last.map(|e| e.to_string())
                .unwrap_or_else(|| "no addresses resolved".to_string())
        ))
    }

    fn with_timeouts(&self, stream: TcpStream, host: &str) -> Result<TcpStream, String> {
        stream
            .set_read_timeout(Some(self.read_timeout))
            .map_err(|e| format!("cannot configure the read timeout to {}: {}", host, e))?;
        stream
            .set_write_timeout(Some(self.read_timeout))
            .map_err(|e| format!("cannot configure the write timeout to {}: {}", host, e))?;
        Ok(stream)
    }

    /// Wrap a connected TCP stream in a TLS session and complete the handshake, so a
    /// certificate or protocol failure is surfaced here - named - before any HTTP byte is
    /// sent. With verification always on, this is where an unknown CA, a name mismatch, an
    /// expired certificate or a protocol error fails.
    fn tls_connect(
        &self,
        stream: TcpStream,
        parts: &UrlParts,
    ) -> Result<StreamOwned<ClientConnection, TcpStream>, String> {
        let name = ServerName::try_from(parts.host.as_str())
            .map_err(|e| format!("invalid TLS server name {}: {}", parts.host, e))?
            .to_owned();
        let conn = ClientConnection::new(self.tls.clone(), name)
            .map_err(|e| format!("cannot start a TLS session with {}: {}", parts.host, e))?;
        let mut tls = StreamOwned::new(conn, stream);
        while tls.conn.is_handshaking() {
            tls.conn
                .complete_io(&mut tls.sock)
                .map_err(|e| format!("TLS handshake with {} failed: {}", parts.host, e))?;
        }
        Ok(tls)
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
        let parts = parse_url(url)?;
        let request = build_request(method, &parts, authorization, body);
        let response = match parts.scheme {
            Scheme::Http => {
                let mut stream = self.connect(&parts)?;
                stream
                    .write_all(request.as_bytes())
                    .map_err(|e| format!("cannot send the request to {}: {}", url, e))?;
                let mut bytes = Vec::new();
                stream
                    .read_to_end(&mut bytes)
                    .map_err(|e| format!("cannot read the response from {}: {}", url, e))?;
                bytes
            }
            Scheme::Https => {
                let stream = self.connect(&parts)?;
                let mut tls = self.tls_connect(stream, &parts)?;
                tls.write_all(request.as_bytes())
                    .map_err(|e| format!("cannot send the request to {}: {}", url, e))?;
                let mut bytes = Vec::new();
                tls.read_to_end(&mut bytes)
                    .map_err(|e| format!("cannot read the response from {}: {}", url, e))?;
                bytes
            }
        };
        parse_http_response(&response)
    }
}

/// Assemble the raw HTTP/1.1 request line and headers for the given URL parts. The request
/// body (when present) is the propose payload and carries an explicit Content-Length.
fn build_request(
    method: &str,
    parts: &UrlParts,
    authorization: Option<&str>,
    body: Option<&str>,
) -> String {
    let mut request = format!(
        "{} {} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\nAccept: application/json\r\n",
        method, parts.path, parts.host
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
    request
}

/// The bundled public trust anchors (Mozilla's root program, shipped in webpki-roots).
fn bundled_roots() -> RootCertStore {
    let mut roots = RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    roots
}

/// Build a client config over the pure-Rust ring provider with the given trust anchors.
fn tls_client_config(roots: RootCertStore) -> ClientConfig {
    ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth()
}

/// Split a service URL into its scheme, host, port and path. Both http:// (a local service,
/// default port 80) and https:// (any TLS deployment, default port 443) are accepted; a
/// scheme that is neither is refused loudly rather than guessed at.
fn parse_url(url: &str) -> Result<UrlParts, String> {
    let (scheme, rest) = if let Some(r) = url.strip_prefix("https://") {
        (Scheme::Https, r)
    } else if let Some(r) = url.strip_prefix("http://") {
        (Scheme::Http, r)
    } else {
        return Err(format!(
            "the service URL must be http:// or https://, got {}",
            url
        ));
    };
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let default_port = match scheme {
        Scheme::Http => 80,
        Scheme::Https => 443,
    };
    let (host, port) = split_authority(authority, default_port, url)?;
    if host.is_empty() {
        return Err(format!("invalid service URL {}", url));
    }
    Ok(UrlParts {
        scheme,
        host,
        port,
        path: path.to_string(),
    })
}

/// Split the authority (host[:port]) of a URL into its host and port. An IPv6 literal keeps
/// its brackets removed and its port parsed after the closing bracket; a bare host gets the
/// scheme's default port.
fn split_authority(authority: &str, default_port: u16, url: &str) -> Result<(String, u16), String> {
    if let Some(rest) = authority.strip_prefix('[') {
        let end = rest
            .find(']')
            .ok_or_else(|| format!("invalid service URL {}", url))?;
        let host = rest[..end].to_string();
        let after = &rest[end + 1..];
        let port = match after.strip_prefix(':') {
            Some(p) => p
                .parse::<u16>()
                .map_err(|e| format!("invalid port in {}: {}", url, e))?,
            None if after.is_empty() => default_port,
            None => return Err(format!("invalid service URL {}", url)),
        };
        return Ok((host, port));
    }
    match authority.rsplit_once(':') {
        Some((h, p)) if !h.is_empty() => Ok((
            h.to_string(),
            p.parse::<u16>()
                .map_err(|e| format!("invalid port in {}: {}", url, e))?,
        )),
        _ => Ok((authority.to_string(), default_port)),
    }
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
        let detail = refusal_detail(&response.body);
        Err(format!(
            "the modelwrite service refused {} {} with HTTP {}: {}",
            method, path, response.status, detail
        ))
    }

    /// Send a request and demand a 2xx answer, returning the body as raw text (not parsed as
    /// JSON). This is the read for the retained-artifact tool: the retained source artifact IS
    /// the body, byte for byte, and must not be re-serialised or filtered through a JSON
    /// parser. A non-2xx answer is a refusal, surfaced exactly as [Self::json] surfaces it,
    /// and exactly once: never a retry.
    fn raw(&self, method: &str, path: &str, body: Option<&str>) -> Result<String, String> {
        let response = self.request(method, path, body)?;
        if (200..300).contains(&response.status) {
            return Ok(response.body);
        }
        let detail = refusal_detail(&response.body);
        Err(format!(
            "the modelwrite service refused {} {} with HTTP {}: {}",
            method, path, response.status, detail
        ))
    }
}

/// The human-readable detail of a non-2xx response: the service's own "error" field when it
/// is present, otherwise the raw body. Shared by [RepoClient::json] and [RepoClient::raw] so
/// the two refusals can never drift apart.
fn refusal_detail(body: &str) -> String {
    serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| v.get("error").and_then(|e| e.as_str()).map(str::to_string))
        .unwrap_or_else(|| body.to_string())
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
        "repo.find" => {
            let project = require_str(args, "project")?;
            let hash = require_str(args, "hash")?;
            let query = require_str(args, "query")?;
            let root = load_document(client, project, hash)?;
            let needle = query.to_lowercase();
            let mut matches: Vec<Value> = Vec::new();
            push_matches(&mut matches, "structure", &root.structure, &needle);
            push_matches(&mut matches, "interfaces", &root.interfaces, &needle);
            push_matches(&mut matches, "signals", &root.signals, &needle);
            Ok(json!({
                "project": project,
                "hash": hash,
                "query": query,
                "matches": matches,
            }))
        }
        "repo.element" => {
            let project = require_str(args, "project")?;
            let hash = require_str(args, "hash")?;
            let id = require_str(args, "id")?;
            let root = load_document(client, project, hash)?;
            let (section, element) = find_element(&root, id)
                .ok_or_else(|| format!("element {} not found in commit {}", id, hash))?;
            let edges: Vec<&okf::types::GraphEdge> = root
                .graph
                .as_ref()
                .map(|g| {
                    g.edges
                        .iter()
                        .filter(|e| e.source == id || e.target == id)
                        .collect()
                })
                .unwrap_or_default();
            Ok(json!({
                "project": project,
                "hash": hash,
                "id": id,
                "section": section,
                "element": element,
                "edges": edges,
            }))
        }
        "repo.coverage" => {
            let project = require_str(args, "project")?;
            let hash = require_str(args, "hash")?;
            let root = load_document(client, project, hash)?;
            if root.graph.is_none() {
                return Err(format!(
                    "commit {} has no graph section, so no coverage can be computed",
                    hash
                ));
            }
            // The engine's own coverage report, never a reimplementation: the same function
            // the gate and the model page call.
            let coverage = graph::requirement_coverage(&root);
            serde_json::to_value(coverage)
                .map_err(|e| format!("coverage does not serialize: {}", e))
        }
        "repo.references" => {
            let project = require_str(args, "project")?;
            let hash = require_str(args, "hash")?;
            let resolve = args
                .get("resolve")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            // The SAME resolution the API and the UI use: with the resolve flag it is the
            // references/resolve route; without it, the raw typed references.
            let path = if resolve {
                format!(
                    "/projects/{}/commits/{}/references/resolve",
                    pct(project),
                    pct(hash)
                )
            } else {
                format!(
                    "/projects/{}/commits/{}/references",
                    pct(project),
                    pct(hash)
                )
            };
            client.json("GET", &path, None)
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
        "repo.lossSummary" => {
            let project = require_str(args, "project")?;
            let artifact_hash = require_str(args, "artifactHash")?;
            let offset = args
                .get("offset")
                .and_then(Value::as_i64)
                .unwrap_or(0)
                .max(0) as usize;
            let limit = args
                .get("limit")
                .and_then(Value::as_i64)
                .unwrap_or(0)
                .max(0) as usize;
            let report = client.json(
                "GET",
                &format!(
                    "/projects/{}/import/{}/report",
                    pct(project),
                    pct(artifact_hash)
                ),
                None,
            )?;
            let mappings = report
                .get("lossReport")
                .and_then(|lr| lr.get("mappings"))
                .and_then(Value::as_array)
                .ok_or_else(|| "the import report has no loss-report mappings".to_string())?;
            // Aggregation is a VIEW over what the binding reported: it groups the entries it
            // was handed and never drops one. The full list stays available through the page.
            let mut by_verdict: BTreeMap<&str, usize> = BTreeMap::new();
            let mut by_construct: BTreeMap<&str, (usize, BTreeMap<&str, usize>)> = BTreeMap::new();
            for mapping in mappings {
                let verdict = mapping
                    .get("verdict")
                    .and_then(Value::as_str)
                    .unwrap_or("Unknown");
                let subject = mapping.get("subject").and_then(Value::as_str).unwrap_or("");
                let construct = construct_of(subject);
                *by_verdict.entry(verdict).or_default() += 1;
                let bucket = by_construct.entry(construct).or_default();
                bucket.0 += 1;
                *bucket.1.entry(verdict).or_default() += 1;
            }

            let mut verdict_rows: Vec<Value> = by_verdict
                .iter()
                .map(|(verdict, count)| json!({ "verdict": verdict, "count": count }))
                .collect();
            verdict_rows.sort_by(|a, b| {
                b["count"]
                    .as_u64()
                    .cmp(&a["count"].as_u64())
                    .then_with(|| a["verdict"].as_str().cmp(&b["verdict"].as_str()))
            });

            let mut construct_rows: Vec<Value> = by_construct
                .iter()
                .map(|(construct, (total, verdicts))| {
                    json!({
                        "construct": construct,
                        "count": total,
                        "unmappable": verdicts.get("Unmappable").copied().unwrap_or(0),
                        "lossy": verdicts.get("Lossy").copied().unwrap_or(0),
                        "exact": verdicts.get("Exact").copied().unwrap_or(0),
                    })
                })
                .collect();
            construct_rows.sort_by(|a, b| {
                b["count"]
                    .as_u64()
                    .cmp(&a["count"].as_u64())
                    .then_with(|| a["construct"].as_str().cmp(&b["construct"].as_str()))
            });

            let total = mappings.len();
            let page: Vec<Value> = mappings.iter().skip(offset).take(limit).cloned().collect();
            Ok(json!({
                "artifactHash": report.get("artifactHash"),
                "bindingId": report.get("bindingId"),
                "bindingVersion": report.get("bindingVersion"),
                "total": total,
                "byVerdict": verdict_rows,
                "byConstruct": construct_rows,
                "page": {
                    "offset": offset,
                    "limit": limit,
                    "total": total,
                    "entries": page,
                },
            }))
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
        "repo.proposals" => {
            let project = require_str(args, "project")?;
            client.json(
                "GET",
                &format!("/projects/{}/proposals", pct(project)),
                None,
            )
        }
        "repo.analytics" => {
            let project = require_str(args, "project")?;
            let requirements = require_str(args, "requirements")?;
            let mut path = format!(
                "/projects/{}/analytics?requirements={}",
                pct(project),
                pct(requirements)
            );
            if let Some(branch) = arg_str(args, "branch") {
                path.push_str(&format!("&branch={}", pct(branch)));
            }
            if let Some(commit) = arg_str(args, "commit") {
                path.push_str(&format!("&commit={}", pct(commit)));
            }
            client.json("GET", &path, None)
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

/// Dispatch the one raw-bytes tool (repo.artifact) against the configured client. It is the
/// retained source artifact, returned verbatim - never re-serialised and never parsed as JSON.
fn run_raw_tool(name: &str, args: &Value, client: &RepoClient) -> Result<String, String> {
    match name {
        "repo.artifact" => {
            let project = require_str(args, "project")?;
            let artifact_hash = require_str(args, "artifactHash")?;
            client.raw(
                "GET",
                &format!(
                    "/projects/{}/import/{}/artifact",
                    pct(project),
                    pct(artifact_hash)
                ),
                None,
            )
        }
        _ => Err(format!("unknown repository tool: {}", name)),
    }
}

/// Read and deserialise the OKF document behind a commit, so the read-only model tools
/// (find, element, coverage) share one read and one error, never a second fetch.
fn load_document(
    client: &RepoClient,
    project: &str,
    hash: &str,
) -> Result<okf::types::OkfRoot, String> {
    let document = client.json(
        "GET",
        &format!("/projects/{}/commits/{}", pct(project), pct(hash)),
        None,
    )?;
    serde_json::from_value(document)
        .map_err(|e| format!("commit {} is not an OKF document: {}", hash, e))
}

/// Whether an element matches a search fragment: a case-insensitive substring of its id, name,
/// or any stereotype.
fn element_matches(element: &okf::types::Element, needle: &str) -> bool {
    element.name.to_lowercase().contains(needle)
        || element.id.to_lowercase().contains(needle)
        || element
            .stereotypes
            .iter()
            .any(|s| s.to_lowercase().contains(needle))
}

/// Append the elements of one section that match the fragment, each with its id, name and kind
/// plus the section that disambiguates it.
fn push_matches(
    out: &mut Vec<Value>,
    section: &str,
    elements: &[okf::types::Element],
    needle: &str,
) {
    for element in elements {
        if element_matches(element, needle) {
            out.push(json!({
                "id": element.id,
                "name": element.name,
                "kind": element.kind,
                "section": section,
            }));
        }
    }
}

/// Find an element by id across the three element sections, returning the section name so a
/// caller can tell where the element lives.
fn find_element<'a>(
    root: &'a okf::types::OkfRoot,
    id: &str,
) -> Option<(&'static str, &'a okf::types::Element)> {
    if let Some(e) = root.structure.iter().find(|e| e.id == id) {
        return Some(("structure", e));
    }
    if let Some(e) = root.interfaces.iter().find(|e| e.id == id) {
        return Some(("interfaces", e));
    }
    if let Some(e) = root.signals.iter().find(|e| e.id == id) {
        return Some(("signals", e));
    }
    None
}

/// The leading construct of a loss-report subject: "uml:Port _2026x_1_..." -> "uml:Port". This
/// is the shared key a loss entry carries to its binding table row.
fn construct_of(subject: &str) -> &str {
    subject.split_whitespace().next().unwrap_or(subject)
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
    fn parse_url_parses_http_and_https_ports_and_paths() {
        let parts = parse_url("http://127.0.0.1:8080/projects/coffee").unwrap();
        assert_eq!(parts.scheme, Scheme::Http);
        assert_eq!(parts.host, "127.0.0.1");
        assert_eq!(parts.port, 8080);
        assert_eq!(parts.path, "/projects/coffee");

        let parts = parse_url("http://example.com").unwrap();
        assert_eq!(parts.scheme, Scheme::Http);
        assert_eq!(parts.port, 80);

        // https is a first-class scheme now, with the TLS default port.
        let parts = parse_url("https://example.com").unwrap();
        assert_eq!(parts.scheme, Scheme::Https);
        assert_eq!(parts.port, 443);
        assert_eq!(parts.path, "/");

        let parts = parse_url("https://example.com:8443/projects").unwrap();
        assert_eq!(parts.scheme, Scheme::Https);
        assert_eq!(parts.port, 8443);
        assert_eq!(parts.path, "/projects");

        assert!(parse_url("ftp://example.com").is_err());
        assert!(parse_url("not a url").is_err());
    }

    #[test]
    fn pct_encodes_path_unsafe_bytes() {
        assert_eq!(pct("coffee-machine.v2"), "coffee-machine.v2");
        assert_eq!(pct("a/b"), "a%2Fb");
    }
}
