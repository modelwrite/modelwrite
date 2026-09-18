// SPDX-License-Identifier: AGPL-3.0-or-later
//! A minimal HTTP/1.1 client over std::net::TcpStream. It speaks plain HTTP on purpose:
//! TLS belongs at the reverse proxy, and the client says so rather than pretending.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::Path;

use serde_json::{json, Value};

use crate::Command;

/// One HTTP response: the status code and the raw body bytes.
struct Response {
    status: u16,
    body: Vec<u8>,
}

/// A plain-HTTP client for one server, with an optional bearer token.
struct Client {
    host: String,
    port: u16,
    token: Option<String>,
}

/// Run a command against the service at url. token, when present, is the bearer token
/// already read from the environment variable named by --token.
pub fn run(url: &str, token: Option<&str>, command: Command) -> Result<Value, String> {
    let client = Client::connect(url, token)?;
    client.execute(command)
}

impl Client {
    fn connect(url: &str, token: Option<&str>) -> Result<Client, String> {
        if url.starts_with("https://") {
            return Err(format!(
                "{} is an https URL; this client speaks plain HTTP because TLS belongs at the reverse proxy - point --server at the proxy's http:// address instead",
                url
            ));
        }
        let rest = url
            .strip_prefix("http://")
            .ok_or_else(|| format!("--server must be an http:// URL, got {}", url))?;
        let rest = rest.strip_suffix('/').unwrap_or(rest);
        if rest.contains('/') {
            return Err("--server URL must not include a path".to_string());
        }
        let (host, port) = match rest.split_once(':') {
            Some((host, port)) => {
                let port: u16 = port
                    .parse()
                    .map_err(|_| format!("invalid port in {}", url))?;
                (host.to_string(), port)
            }
            None => (rest.to_string(), 80),
        };
        if host.is_empty() {
            return Err(format!("missing host in {}", url));
        }
        Ok(Client {
            host,
            port,
            token: token.map(str::to_string),
        })
    }

    fn execute(&self, command: Command) -> Result<Value, String> {
        match command {
            Command::ProjectCreate { name } => {
                let body =
                    serde_json::to_vec(&json!({ "name": name })).expect("json body serialises");
                let resp = self.request("POST", "/projects", Some(body))?;
                self.handle(resp, 201)
            }
            Command::ProjectList => {
                let resp = self.request("GET", "/projects", None)?;
                self.handle(resp, 200)
            }
            Command::Commit {
                project,
                branch,
                message,
                file,
                holder,
            } => {
                let okf = read_json(&file)?;
                let mut body = json!({ "branch": branch, "message": message, "okf": okf });
                if let Some(h) = holder {
                    body["holder"] = Value::String(h);
                }
                let path = format!("/projects/{}/commits", project);
                let body = serde_json::to_vec(&body).expect("json body serialises");
                let resp = self.request("POST", &path, Some(body))?;
                self.handle(resp, 201)
            }
            Command::Log { project, branch } => {
                let path = format!("/projects/{}/commits?branch={}", project, branch);
                let resp = self.request("GET", &path, None)?;
                self.handle(resp, 200)
            }
            Command::BranchList { project } => {
                let path = format!("/projects/{}/branches", project);
                let resp = self.request("GET", &path, None)?;
                self.handle(resp, 200)
            }
            Command::BranchCreate {
                project,
                name,
                from,
            } => {
                let body = serde_json::to_vec(&json!({ "name": name, "from": from }))
                    .expect("json body serialises");
                let path = format!("/projects/{}/branches", project);
                let resp = self.request("POST", &path, Some(body))?;
                self.handle(resp, 201)
            }
            Command::BranchDelete { project, name } => {
                let path = format!("/projects/{}/branches/{}", project, name);
                let resp = self.request("DELETE", &path, None)?;
                self.handle(resp, 204)
            }
            Command::Merge {
                project,
                branch,
                other,
                message,
                holder,
            } => {
                let mut body = json!({ "branch": branch, "other": other, "message": message });
                if let Some(h) = holder {
                    body["holder"] = Value::String(h);
                }
                let path = format!("/projects/{}/merge", project);
                let body = serde_json::to_vec(&body).expect("json body serialises");
                let resp = self.request("POST", &path, Some(body))?;
                self.handle(resp, 201)
            }
            Command::Reset {
                project,
                branch,
                to,
                message,
                holder,
            } => {
                let mut body = json!({ "to": to, "message": message });
                if let Some(h) = holder {
                    body["holder"] = Value::String(h);
                }
                let path = format!("/projects/{}/branches/{}/reset", project, branch);
                let body = serde_json::to_vec(&body).expect("json body serialises");
                let resp = self.request("POST", &path, Some(body))?;
                self.handle(resp, 201)
            }
            Command::Gate {
                project,
                reference,
                candidate,
            } => {
                let body =
                    serde_json::to_vec(&json!({ "reference": reference, "candidate": candidate }))
                        .expect("json body serialises");
                let path = format!("/projects/{}/gate", project);
                let resp = self.request("POST", &path, Some(body))?;
                self.handle(resp, 200)
            }
            Command::LockAcquire {
                project,
                branch,
                elements,
                holder,
                ttl_seconds,
            } => {
                let body = serde_json::to_vec(&json!({
                    "branch": branch,
                    "elements": elements,
                    "holder": holder,
                    "ttlSeconds": ttl_seconds,
                }))
                .expect("json body serialises");
                let path = format!("/projects/{}/locks", project);
                let resp = self.request("POST", &path, Some(body))?;
                self.handle(resp, 201)
            }
            Command::LockRelease {
                project,
                holder,
                ids,
            } => {
                let body = serde_json::to_vec(&json!({ "holder": holder, "ids": ids }))
                    .expect("json body serialises");
                let path = format!("/projects/{}/locks/release", project);
                let resp = self.request("POST", &path, Some(body))?;
                self.handle(resp, 200)
            }
            Command::LockList { project } => {
                let path = format!("/projects/{}/locks", project);
                let resp = self.request("GET", &path, None)?;
                self.handle(resp, 200)
            }
            Command::Audit { project, limit } => {
                let path = format!("/projects/{}/audit?limit={}", project, limit);
                let resp = self.request("GET", &path, None)?;
                self.handle(resp, 200)
            }
        }
    }

    fn request(&self, method: &str, path: &str, body: Option<Vec<u8>>) -> Result<Response, String> {
        let addr = format!("{}:{}", self.host, self.port);
        let mut stream =
            TcpStream::connect(&addr).map_err(|e| format!("cannot connect to {}: {}", addr, e))?;

        let mut head = format!(
            "{} {} HTTP/1.1\r\nHost: {}\r\nAccept: application/json\r\n",
            method, path, addr
        );
        if let Some(token) = &self.token {
            head.push_str(&format!("Authorization: Bearer {}\r\n", token));
        }
        match body {
            Some(bytes) => {
                head.push_str("Content-Type: application/json\r\n");
                head.push_str(&format!("Content-Length: {}\r\n", bytes.len()));
                head.push_str("Connection: close\r\n\r\n");
                stream
                    .write_all(head.as_bytes())
                    .map_err(|e| format!("write failed: {}", e))?;
                stream
                    .write_all(&bytes)
                    .map_err(|e| format!("write failed: {}", e))?;
            }
            None => {
                head.push_str("Connection: close\r\n\r\n");
                stream
                    .write_all(head.as_bytes())
                    .map_err(|e| format!("write failed: {}", e))?;
            }
        }
        stream.flush().map_err(|e| format!("flush failed: {}", e))?;

        let mut raw = Vec::new();
        stream
            .read_to_end(&mut raw)
            .map_err(|e| format!("read failed: {}", e))?;
        parse_response(&raw)
    }

    fn handle(&self, resp: Response, ok_status: u16) -> Result<Value, String> {
        if resp.status == ok_status {
            if resp.body.is_empty() {
                return Ok(Value::Null);
            }
            return serde_json::from_slice(&resp.body)
                .map_err(|_| "the server returned a non-JSON success response".to_string());
        }
        // Never echo a token: the body comes from the server, which never returns one.
        let message = match serde_json::from_slice::<Value>(&resp.body) {
            Ok(value) => value
                .get("error")
                .and_then(|e| e.as_str())
                .map(str::to_string)
                .unwrap_or_else(|| value.to_string()),
            Err(_) => format!("the server answered status {}", resp.status),
        };
        Err(format!("{} (status {})", message, resp.status))
    }
}

/// Read a JSON document from a file, so the commit body carries the model as a value
/// rather than as a string.
fn read_json(path: &Path) -> Result<Value, String> {
    let bytes =
        std::fs::read(path).map_err(|e| format!("cannot read {}: {}", path.display(), e))?;
    serde_json::from_slice(&bytes)
        .map_err(|e| format!("{} is not valid JSON: {}", path.display(), e))
}

/// Parse a raw HTTP/1.1 response into a status and body, de-chunking when the server
/// answers with transfer-encoding: chunked.
fn parse_response(raw: &[u8]) -> Result<Response, String> {
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or("malformed response: no header terminator")?;
    let head = &raw[..split];
    let rest = &raw[split + 4..];

    let head = std::str::from_utf8(head).map_err(|_| "malformed response header")?;
    let mut lines = head.split("\r\n");
    let status_line = lines.next().ok_or("empty response")?;
    let status = parse_status(status_line)?;

    let mut content_length: Option<usize> = None;
    let mut chunked = false;
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            let name = name.trim().to_ascii_lowercase();
            let value = value.trim();
            match name.as_str() {
                "content-length" => content_length = value.parse().ok(),
                "transfer-encoding" => chunked = value.to_ascii_lowercase().contains("chunked"),
                _ => {}
            }
        }
    }

    let body = if chunked {
        dechunk(rest)?
    } else if let Some(len) = content_length {
        rest[..len.min(rest.len())].to_vec()
    } else {
        rest.to_vec()
    };
    Ok(Response { status, body })
}

fn parse_status(line: &str) -> Result<u16, String> {
    let mut parts = line.split_whitespace();
    let _version = parts.next();
    let code = parts.next().ok_or("malformed status line")?;
    code.parse::<u16>()
        .map_err(|_| format!("malformed status code {}", code))
}

/// De-chunk a transfer-encoded body into its payload bytes.
fn dechunk(mut data: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    loop {
        let line_end = data
            .windows(2)
            .position(|w| w == b"\r\n")
            .ok_or("malformed chunked body")?;
        let size_hex =
            std::str::from_utf8(&data[..line_end]).map_err(|_| "malformed chunk size")?;
        let size_hex = size_hex.split(';').next().unwrap_or(size_hex).trim();
        let size = usize::from_str_radix(size_hex, 16)
            .map_err(|_| format!("malformed chunk size {}", size_hex))?;
        data = &data[line_end + 2..];
        if size == 0 {
            break;
        }
        if data.len() < size + 2 {
            return Err("truncated chunk".to_string());
        }
        out.extend_from_slice(&data[..size]);
        data = &data[size..];
        if &data[..2] != b"\r\n" {
            return Err("malformed chunk terminator".to_string());
        }
        data = &data[2..];
    }
    Ok(out)
}
