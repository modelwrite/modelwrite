// SPDX-License-Identifier: AGPL-3.0-or-later
//! A minimal HTTP/1.1 client over std::net::TcpStream. It speaks plain HTTP on purpose:
//! TLS belongs at the reverse proxy, and the client says so rather than pretending.

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{json, Value};

use crate::analytics::TABLES;
use crate::Command;

/// One HTTP response: the status code, the raw body bytes, and the response headers. The
/// tables endpoint carries the row total and the page cursor in headers for the CSV and
/// NDJSON formats, where the JSON envelope's fields have no place to live.
struct Response {
    status: u16,
    body: Vec<u8>,
    headers: Vec<(String, String)>,
}

impl Response {
    /// A response header by its lower-cased wire name.
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }
}

/// The identity and row total of one table for one commit or branch: what
/// GET /analytics/{project}/tables/{table} reports in its JSON envelope.
struct TableSummary {
    commit: String,
    total: usize,
}

/// A plain-HTTP client for one server, with an optional bearer token.
struct Client {
    host: String,
    port: u16,
    token: Option<String>,
    connect_timeout: Duration,
    io_timeout: Duration,
}

/// A server that accepts the connection but never answers must not hang the CLI forever.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const IO_TIMEOUT: Duration = Duration::from_secs(30);

/// Run a command against the service at url. token, when present, is the bearer token
/// already read from the environment variable named by --token.
pub fn run(url: &str, token: Option<&str>, command: Command) -> Result<Value, String> {
    let client = Client::connect(url, token)?;
    client.execute(command)
}

/// Build a Package 3 analytics URL with the optional commit/branch query selectors.
fn analytics_path(base: &str, commit: Option<&str>, branch: Option<&str>) -> String {
    let mut params = Vec::new();
    if let Some(c) = commit {
        params.push(format!("commit={}", c));
    }
    if let Some(b) = branch {
        params.push(format!("branch={}", b));
    }
    if params.is_empty() {
        base.to_string()
    } else {
        format!("{}?{}", base, params.join("&"))
    }
}

/// The real Package 3 tables route: GET /analytics/{project}/tables/{table}. The table is a
/// PATH segment, not a query parameter: the request this client used to build
/// (/analytics/{project}/tables?commit=...) matched no route at all and was answered 404,
/// so no table row could be fetched over --server.
fn table_path(project: &str, table: &str, commit: Option<&str>, branch: Option<&str>) -> String {
    analytics_path(
        &format!("/analytics/{}/tables/{}", project, table),
        commit,
        branch,
    )
}

/// Append one query parameter to a path that may already carry a query string.
fn with_param(path: String, param: &str) -> String {
    if path.contains('?') {
        format!("{}&{}", path, param)
    } else {
        format!("{}?{}", path, param)
    }
}

/// The error text of a non-success response: the server's own "error" field when it sent
/// one, otherwise the status alone. Never echoes a token.
fn failure_message(resp: &Response) -> String {
    let message = match serde_json::from_slice::<Value>(&resp.body) {
        Ok(value) => value
            .get("error")
            .and_then(|e| e.as_str())
            .map(str::to_string)
            .unwrap_or_else(|| value.to_string()),
        Err(_) => format!("the server answered status {}", resp.status),
    };
    format!("{} (status {})", message, resp.status)
}

/// The page size the --server export pages a table at: the server's own MAX_LIMIT, so a
/// routine table is one request and only a genuinely large table walks the cursor.
const EXPORT_PAGE: usize = 1000;

/// The metrics and trend routes answer the JSON wire shape and nothing else: there is no CSV
/// envelope on the wire, and building one here would be a second serialiser to keep in step
/// with the server. Refuse a format the route cannot serve, naming the transport that can,
/// rather than quietly answering JSON to a caller who asked for CSV.
fn require_json_format(format: &str) -> Result<(), String> {
    if format == "json" {
        return Ok(());
    }
    Err(format!(
        "analytics metrics and trend over --server serve the JSON wire shape only; --format {} is not available over --server - use --db <path> for csv, or drop --format for the JSON rows",
        format
    ))
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
            connect_timeout: CONNECT_TIMEOUT,
            io_timeout: IO_TIMEOUT,
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
            Command::Artifact { .. } => Err(
                "fetching the retained artifact through the CLI is supported offline only; use --db <path>"
                    .to_string(),
            ),
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
            // The analytics group over --server reaches the real Package 3 REST surface:
            // GET /analytics/schema, GET /analytics/{project}/tables/{table} (the table is a
            // path segment), GET /analytics/{project}/metrics and GET /analytics/{project}/trend.
            Command::AnalyticsSchema => {
                let resp = self.request("GET", "/analytics/schema", None)?;
                self.handle(resp, 200)
            }
            Command::AnalyticsTables {
                project,
                commit,
                branch,
            } => {
                // The route is PER TABLE, so the summary the --db path prints is assembled
                // from one cheap page per table: the envelope's total is the row count the
                // offline projection computes for the same commit, and its commit is the
                // resolved tip, so both transports print identical bytes.
                let commit = commit.as_deref();
                let branch = branch.as_deref();
                let mut resolved: Option<String> = None;
                let mut tables = Vec::with_capacity(TABLES.len());
                for &(name, _) in TABLES {
                    let summary = self.table_summary(&project, name, commit, branch)?;
                    if resolved.is_none() {
                        resolved = Some(summary.commit);
                    }
                    tables.push(json!({ "name": name, "rowCount": summary.total }));
                }
                Ok(json!({
                    "schemaVersion": crate::analytics::SCHEMA_VERSION,
                    "project": project,
                    "commit": resolved.unwrap_or_default(),
                    "tables": tables,
                }))
            }
            Command::AnalyticsExport {
                project,
                commit,
                branch,
                all_commits,
                format,
                out,
            } => self.export(project, commit, branch, all_commits, format, out),
            Command::AnalyticsMetrics {
                project,
                commit,
                branch,
                format,
            } => {
                require_json_format(&format)?;
                let path = analytics_path(
                    &format!("/analytics/{}/metrics", project),
                    commit.as_deref(),
                    branch.as_deref(),
                );
                let resp = self.request("GET", &path, None)?;
                self.handle(resp, 200)
            }
            Command::AnalyticsTrend {
                metric,
                project,
                branch,
                from,
                to,
                format,
            } => {
                require_json_format(&format)?;
                let mut params = vec![format!("metric={}", metric), format!("branch={}", branch)];
                if let Some(f) = &from {
                    params.push(format!("from={}", f));
                }
                if let Some(t) = &to {
                    params.push(format!("to={}", t));
                }
                let path = format!("/analytics/{}/trend?{}", project, params.join("&"));
                let resp = self.request("GET", &path, None)?;
                self.handle(resp, 200)
            }
        }
    }

    /// The identity and row total of one table, one page, capped at one row: the JSON
    /// envelope reports the resolved commit and the total across every page.
    fn table_summary(
        &self,
        project: &str,
        table: &str,
        commit: Option<&str>,
        branch: Option<&str>,
    ) -> Result<TableSummary, String> {
        let path = with_param(table_path(project, table, commit, branch), "limit=1");
        let resp = self.request("GET", &path, None)?;
        let value = self.handle(resp, 200)?;
        Ok(TableSummary {
            commit: value
                .get("commit")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            total: value.get("total").and_then(Value::as_u64).unwrap_or(0) as usize,
        })
    }

    /// One NDJSON page of one table for one commit. The rows are the schema's at-rest
    /// snake_case - exactly the shape the --db NDJSON export writes - and the cursor rides in
    /// the X-MW-Next-Cursor header, because the NDJSON body carries no envelope to hold it.
    fn table_ndjson_page(
        &self,
        project: &str,
        table: &str,
        commit: &str,
        cursor: Option<&str>,
    ) -> Result<(Vec<Value>, Option<String>), String> {
        let mut path = with_param(
            table_path(project, table, Some(commit), None),
            &format!("format=ndjson&limit={}", EXPORT_PAGE),
        );
        if let Some(cursor) = cursor {
            path = with_param(path, &format!("cursor={}", cursor));
        }
        let resp = self.request("GET", &path, None)?;
        if resp.status != 200 {
            return Err(failure_message(&resp));
        }
        let next = resp.header("x-mw-next-cursor").map(str::to_string);
        let mut rows = Vec::new();
        for line in resp.body.split(|b| *b == b'\n') {
            if line.iter().all(u8::is_ascii_whitespace) {
                continue;
            }
            rows.push(
                serde_json::from_slice(line)
                    .map_err(|e| format!("the server returned a row that is not JSON: {}", e))?,
            );
        }
        Ok((rows, next))
    }

    /// Every row of one table for one commit, paging on the cursor to completion.
    fn table_rows(&self, project: &str, table: &str, commit: &str) -> Result<Vec<Value>, String> {
        let mut rows = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let (mut page, next) =
                self.table_ndjson_page(project, table, commit, cursor.as_deref())?;
            rows.append(&mut page);
            match next {
                Some(next) => cursor = Some(next),
                None => return Ok(rows),
            }
        }
    }

    /// The CLI --server ROW-EXPORT path: read every table of every selected commit over the
    /// real REST routes and write the same tree the --db path writes, through the same
    /// serialisers, so the two exports are byte-identical files and a pipeline can switch
    /// transports without changing anything else.
    fn export(
        &self,
        project: String,
        commit: Option<String>,
        branch: Option<String>,
        all_commits: bool,
        format: String,
        out: PathBuf,
    ) -> Result<Value, String> {
        if !matches!(format.as_str(), "csv" | "ndjson" | "parquet") {
            return Err("analytics export --format must be csv, ndjson or parquet".to_string());
        }
        let commits =
            self.resolve_commits(&project, commit.as_deref(), branch.as_deref(), all_commits)?;
        if commits.is_empty() {
            return Err(format!("no commits to export for project {}", project));
        }

        let mut per_commit: Vec<(String, Vec<crate::analytics::Table>)> = Vec::new();
        for hash in &commits {
            let mut tables = Vec::with_capacity(TABLES.len());
            for &(name, columns) in TABLES {
                tables.push(crate::analytics::Table {
                    name,
                    columns,
                    rows: self.table_rows(&project, name, hash)?,
                });
            }
            per_commit.push((hash.clone(), tables));
        }

        let files = crate::analytics::write_export(&format, &out, &project, &per_commit)?;
        let tables: Vec<Value> = files
            .iter()
            .map(|(name, rows)| json!({ "name": name, "rowCount": rows }))
            .collect();
        Ok(json!({
            "schemaVersion": crate::analytics::SCHEMA_VERSION,
            "project": project,
            "format": format,
            "out": out,
            "tables": tables,
        }))
    }

    /// The commits an export covers, resolved exactly as the --db path resolves them: an
    /// explicit commit is itself; --all-commits is every commit of every branch,
    /// deduplicated and sorted by hash; otherwise every commit on the branch (default
    /// "main"), in the order the server lists them (tip order).
    fn resolve_commits(
        &self,
        project: &str,
        commit: Option<&str>,
        branch: Option<&str>,
        all_commits: bool,
    ) -> Result<Vec<String>, String> {
        if let Some(hash) = commit {
            return Ok(vec![hash.to_string()]);
        }
        if all_commits {
            let mut seen = std::collections::BTreeSet::new();
            let mut out = Vec::new();
            for name in self.branch_names(project)? {
                for hash in self.commit_hashes(project, &name)? {
                    if seen.insert(hash.clone()) {
                        out.push(hash);
                    }
                }
            }
            out.sort();
            return Ok(out);
        }
        self.commit_hashes(project, branch.unwrap_or("main"))
    }

    /// The branch names of one project.
    fn branch_names(&self, project: &str) -> Result<Vec<String>, String> {
        let path = format!("/projects/{}/branches", project);
        let resp = self.request("GET", &path, None)?;
        let value = self.handle(resp, 200)?;
        Ok(value
            .as_array()
            .map(|branches| {
                branches
                    .iter()
                    .filter_map(|b| b.get("name").and_then(Value::as_str).map(str::to_string))
                    .collect()
            })
            .unwrap_or_default())
    }

    /// The commit hashes on one branch, tip first, as the commits route returns them.
    fn commit_hashes(&self, project: &str, branch: &str) -> Result<Vec<String>, String> {
        let path = format!("/projects/{}/commits?branch={}", project, branch);
        let resp = self.request("GET", &path, None)?;
        let value = self.handle(resp, 200)?;
        Ok(value
            .as_array()
            .map(|commits| {
                commits
                    .iter()
                    .filter_map(|c| c.get("hash").and_then(Value::as_str).map(str::to_string))
                    .collect()
            })
            .unwrap_or_default())
    }

    fn request(&self, method: &str, path: &str, body: Option<Vec<u8>>) -> Result<Response, String> {
        let addr = format!("{}:{}", self.host, self.port);
        let socket = addr
            .to_socket_addrs()
            .map_err(|e| format!("cannot resolve {}: {}", addr, e))?
            .next()
            .ok_or_else(|| format!("{} resolved to no address", addr))?;
        let mut stream = TcpStream::connect_timeout(&socket, self.connect_timeout)
            .map_err(|e| format!("cannot connect to {}: {}", addr, e))?;
        stream
            .set_read_timeout(Some(self.io_timeout))
            .map_err(|e| format!("cannot configure the read timeout: {}", e))?;
        stream
            .set_write_timeout(Some(self.io_timeout))
            .map_err(|e| format!("cannot configure the write timeout: {}", e))?;

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
        stream.read_to_end(&mut raw).map_err(|e| {
            if matches!(
                e.kind(),
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
            ) {
                format!(
                    "the server at {} did not answer within {} seconds",
                    addr,
                    self.io_timeout.as_secs()
                )
            } else {
                format!("read failed: {}", e)
            }
        })?;
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
        Err(failure_message(&resp))
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
    let mut headers: Vec<(String, String)> = Vec::new();
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            let name = name.trim().to_ascii_lowercase();
            let value = value.trim();
            match name.as_str() {
                "content-length" => content_length = value.parse().ok(),
                "transfer-encoding" => chunked = value.to_ascii_lowercase().contains("chunked"),
                _ => {}
            }
            headers.push((name, value.to_string()));
        }
    }

    let body = if chunked {
        dechunk(rest)?
    } else if let Some(len) = content_length {
        rest[..len.min(rest.len())].to_vec()
    } else {
        rest.to_vec()
    };
    Ok(Response {
        status,
        body,
        headers,
    })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_server_that_accepts_but_never_answers_fails_with_a_timeout() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            let _stream = listener.accept().unwrap();
            // Hold the accepted connection open without answering; the client must time out.
            std::thread::sleep(Duration::from_secs(3));
        });

        let client = Client {
            host: "127.0.0.1".to_string(),
            port: addr.port(),
            token: None,
            connect_timeout: Duration::from_secs(1),
            io_timeout: Duration::from_millis(500),
        };
        let err = match client.request("GET", "/projects", None) {
            Err(e) => e,
            Ok(_) => panic!("a silent server must not produce a response"),
        };
        assert!(
            err.contains("did not answer"),
            "expected a clear timeout message, got: {}",
            err
        );
    }

    #[test]
    fn the_tables_url_carries_the_table_as_a_path_segment() {
        assert_eq!(
            table_path("coffee", "elements", Some("abc"), None),
            "/analytics/coffee/tables/elements?commit=abc"
        );
        assert_eq!(
            table_path("coffee", "import_losses", None, Some("main")),
            "/analytics/coffee/tables/import_losses?branch=main"
        );
        assert_eq!(
            table_path("coffee", "metrics", None, None),
            "/analytics/coffee/tables/metrics"
        );
        // The defect this guards: the segment-less route matches nothing on the server.
        assert!(!table_path("coffee", "metrics", Some("abc"), None)
            .starts_with("/analytics/coffee/tables?"));
        assert_eq!(
            with_param(
                table_path("coffee", "elements", Some("abc"), None),
                "limit=1"
            ),
            "/analytics/coffee/tables/elements?commit=abc&limit=1"
        );
        assert_eq!(
            with_param("/analytics/coffee/tables/elements".to_string(), "limit=1"),
            "/analytics/coffee/tables/elements?limit=1"
        );
    }

    /// The export pages a table on the X-MW-Next-Cursor header, because an NDJSON body has no
    /// envelope to carry the cursor. A canned two-page server proves the client follows it to
    /// completion and that the request is the real per-table route.
    #[test]
    fn a_cursored_ndjson_table_is_paged_to_completion() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let pages = [(Some("1"), "{\"n\":1}\n"), (None, "{\"n\":2}\n")];
            let mut requests = Vec::new();
            for (cursor, body) in pages {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = Vec::new();
                loop {
                    let mut chunk = [0u8; 1024];
                    let n = stream.read(&mut chunk).unwrap();
                    if n == 0 {
                        break;
                    }
                    request.extend_from_slice(&chunk[..n]);
                    if request.windows(4).any(|w| w == b"\r\n\r\n") {
                        break;
                    }
                }
                requests.push(String::from_utf8_lossy(&request).to_string());
                let cursor_header = match cursor {
                    Some(cursor) => format!("X-MW-Next-Cursor: {}\r\n", cursor),
                    None => String::new(),
                };
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/x-ndjson\r\nContent-Length: {}\r\n{}Connection: close\r\n\r\n{}",
                    body.len(),
                    cursor_header,
                    body
                );
                stream.write_all(response.as_bytes()).unwrap();
                stream.flush().unwrap();
            }
            requests
        });

        let client = Client {
            host: "127.0.0.1".to_string(),
            port: addr.port(),
            token: None,
            connect_timeout: Duration::from_secs(1),
            io_timeout: Duration::from_secs(2),
        };
        let rows = client
            .table_rows("coffee", "elements", "abc")
            .expect("both pages must be read");
        assert_eq!(rows, vec![json!({ "n": 1 }), json!({ "n": 2 })]);

        let requests = server.join().unwrap();
        let lines: Vec<&str> = requests
            .iter()
            .map(|r| r.lines().next().unwrap_or_default())
            .collect();
        assert_eq!(
            lines,
            vec![
                "GET /analytics/coffee/tables/elements?commit=abc&format=ndjson&limit=1000 HTTP/1.1",
                "GET /analytics/coffee/tables/elements?commit=abc&format=ndjson&limit=1000&cursor=1 HTTP/1.1",
            ],
            "the client must address the real per-table route and follow the cursor"
        );
    }
}
