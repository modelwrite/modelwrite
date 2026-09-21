// SPDX-License-Identifier: AGPL-3.0-or-later
//! The Package 3 analytics REST surface: read-only views of the mw-analytics-schema@1 tables
//! and measures, plus the operator-gated OpenMetrics endpoint.
//!
//! Endpoints:
//! - GET /analytics/schema - the schema document and version
//! - GET /analytics/{project}/tables/{table} - a table for one commit or branch, with
//!   format, limit, cursor and key-column equality filters
//! - GET /analytics/{project}/metrics - the metrics table for a commit or branch head
//! - GET /analytics/{project}/trend - one metric across a branch's commits in order
//! - GET /metrics - OpenMetrics gauges (off unless MW_ENABLE_OPENMETRICS is set)
//!
//! Large tables stream: rows are serialised one at a time through a bounded HTTP body, never
//! materialised as a whole table in memory (the TMT import_losses table is 48,553 blocking +
//! 2,231 declaration mappings). Every response carries the schema version; JSON carries it in
//! the envelope, CSV/NDJSON in the X-MW-Schema-Version header.

pub mod format;
pub mod projection;

use std::collections::HashMap;

use axum::extract::{Path, Query, State};
use axum::http::header::{HeaderName, HeaderValue, ACCEPT, CONTENT_TYPE};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use futures_util::StreamExt;
use serde_json::{json, Value};

use crate::api::{map_store_error, ApiState};
use crate::auth::{Identity, Permission};
use crate::error::ApiError;
use crate::store::{Commit, Store};

use projection::SCHEMA_VERSION;

/// The default page size for a table read, and its ceiling.
const DEFAULT_LIMIT: usize = 100;
const MAX_LIMIT: usize = 1000;

/// Reserved query parameters of the tables endpoint. Every other query parameter is an
/// equality filter on a string column of the table. 'commit' and 'branch' are RESERVED for
/// commit resolution (a 'commits' table is always one row, so filtering it by branch would be
/// meaningless); the nine tables carry no column named format, limit or cursor.
const RESERVED: &[&str] = &["commit", "branch", "format", "limit", "cursor"];

/// The response format of a table read.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Format {
    Json,
    Csv,
    Ndjson,
}

/// Resolve the commit to read: an explicit 'commit' hash wins, otherwise 'branch'
/// (default 'main') is resolved to its tip.
fn resolve_commit(
    store: &dyn Store,
    project: &str,
    commit: Option<&str>,
    branch: Option<&str>,
) -> Result<Commit, ApiError> {
    let hash = if let Some(hash) = commit {
        hash.to_string()
    } else {
        let branch = branch.unwrap_or("main");
        store
            .branch_tip(project, branch)
            .map_err(map_store_error)?
            .ok_or_else(|| ApiError::not_found(format!("branch {}", branch)))?
    };
    store
        .commit(project, &hash)
        .map_err(map_store_error)?
        .ok_or_else(|| ApiError::not_found(format!("commit {}", hash)))
}

/// 404 on a missing project, before any commit resolution, naming only what the caller asked.
fn require_project(store: &dyn Store, project: &str) -> Result<(), ApiError> {
    if store.project(project).map_err(map_store_error)?.is_none() {
        return Err(ApiError::not_found(format!("project {}", project)));
    }
    Ok(())
}

/// An equality filter on one string column: column = value.
type Filter = (String, String);

/// Whether a row satisfies every filter. Only string-scalar columns match a string filter.
fn matches_filters(row: &Value, filters: &[Filter]) -> bool {
    filters
        .iter()
        .all(|(column, value)| row.get(column).and_then(Value::as_str) == Some(value.as_str()))
}

/// Turn a serialisation error into an io::Error for the streaming body.
fn io_err(e: impl std::fmt::Display) -> std::io::Error {
    std::io::Error::other(e.to_string())
}

/// A body stream that prepends 'prefix', then each chunk, then 'suffix'. Chunks are
/// produced lazily, so a table is serialised one row at a time with the HTTP layer applying
/// backpressure - the whole table is never held in memory.
fn body_from_chunks(
    prefix: Vec<u8>,
    chunks: impl Iterator<Item = Result<Vec<u8>, std::io::Error>> + Send + 'static,
    suffix: Vec<u8>,
) -> axum::body::Body {
    let stream = futures_util::stream::once(async move { Ok::<_, std::io::Error>(prefix) })
        .chain(futures_util::stream::iter(chunks))
        .chain(futures_util::stream::once(async move {
            Ok::<_, std::io::Error>(suffix)
        }));
    axum::body::Body::from_stream(stream)
}

// ---------------------------------------------------------------------------
// GET /analytics/schema
// ---------------------------------------------------------------------------

/// The schema document: the nine tables (name + snake_case logical columns), the generated
/// metric definitions, and the schema version. This is the schema ARTEFACT, so column and
/// definition names stay in the at-rest snake_case, matching the CLI's offline schema.
pub async fn schema(
    identity: Identity,
    State(_state): State<ApiState>,
) -> Result<Json<Value>, ApiError> {
    if !identity.may(Permission::Read) {
        return Err(ApiError::forbidden("read permission required"));
    }
    let tables: Vec<Value> = projection::TABLES
        .iter()
        .map(|(name, columns)| json!({ "name": name, "columns": columns }))
        .collect();
    let metric_definitions: Vec<Value> = analytics::metrics::metric_definitions()
        .into_iter()
        .map(|d| {
            json!({
                "id": d.id,
                "name": d.name,
                "description": d.description,
                "depends_on": d.depends_on,
                "status": d.status,
            })
        })
        .collect();
    Ok(Json(json!({
        "schemaVersion": SCHEMA_VERSION,
        "tables": tables,
        "metricDefinitions": metric_definitions,
    })))
}

// ---------------------------------------------------------------------------
// GET /analytics/{project}/tables/{table}
// ---------------------------------------------------------------------------

/// Build the materialised, sorted rows of a small table for one commit. 'import_losses' is
/// NOT handled here: it is the one table that can be large, and the handler streams it.
fn materialise_table(
    store: &dyn Store,
    project: &str,
    commit: &Commit,
    root: &okf::types::OkfRoot,
    table: &str,
) -> Result<Vec<Value>, ApiError> {
    let mut rows = match table {
        "projects" => projection::projects_table(store, project).map_err(ApiError::internal)?,
        "commits" => projection::commits_table(commit),
        "elements" => projection::elements_table(project, commit, root),
        "relationships" => projection::relationships_table(project, commit, root),
        "requirements" => projection::requirements_table(project, commit, root),
        "trace_links" => projection::trace_links_table(project, commit, root),
        "metrics" => {
            let cnc = projection::constructs_not_carried(store, project, commit)
                .map_err(ApiError::internal)?;
            projection::metric_rows(project, commit, root, cnc)
        }
        "metric_definitions" => projection::metric_definitions_table(),
        other => return Err(ApiError::not_found(format!("unknown table {}", other))),
    };
    projection::sort_rows(table, &mut rows);
    Ok(rows)
}

/// Decide the response format from the 'format' parameter, then the Accept header.
fn negotiate_format(headers: &HeaderMap, format: Option<&str>) -> Result<Format, ApiError> {
    if let Some(format) = format {
        return match format {
            "json" => Ok(Format::Json),
            "csv" => Ok(Format::Csv),
            "ndjson" => Ok(Format::Ndjson),
            other => Err(ApiError::bad_request(format!(
                "format must be json, csv or ndjson, got {}",
                other
            ))),
        };
    }
    let accept = headers
        .get(ACCEPT)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if accept.contains("text/csv") {
        return Ok(Format::Csv);
    }
    if accept.contains("application/x-ndjson")
        || accept.contains("application/ndjson")
        || accept.contains("application/jsonl")
    {
        return Ok(Format::Ndjson);
    }
    Ok(Format::Json)
}

pub async fn table(
    identity: Identity,
    State(state): State<ApiState>,
    Path((project, table)): Path<(String, String)>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Response, ApiError> {
    if !identity.may(Permission::Read) {
        return Err(ApiError::forbidden("read permission required"));
    }
    if !identity.may_reach(&project) {
        return Err(ApiError::forbidden("project not in scope"));
    }
    if !projection::is_table(&table) {
        return Err(ApiError::not_found(format!("unknown table {}", table)));
    }
    require_project(state.store_for(&identity).as_ref(), &project)?;

    let format = negotiate_format(&headers, params.get("format").map(String::as_str))?;
    let limit = parse_limit(params.get("limit").map(String::as_str))?;
    let cursor = match params.get("cursor") {
        Some(raw) => raw
            .parse::<usize>()
            .map_err(|_| ApiError::bad_request("cursor must be a non-negative integer"))?,
        None => 0,
    };

    // Equality filters: every query parameter that is not reserved.
    let mut filters: Vec<Filter> = params
        .iter()
        .filter(|(k, _)| !RESERVED.contains(&k.as_str()))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    filters.sort();
    validate_filters(&table, &filters)?;

    let commit = resolve_commit(
        state.store_for(&identity).as_ref(),
        &project,
        params.get("commit").map(String::as_str),
        params.get("branch").map(String::as_str),
    )?;

    // import_losses is the one table that can be large (45,725 content-loss rows in the TMT
    // case): it streams from the loss report without materialising the rows. Every other table
    // is bounded by the model's own size and materialises to sort.
    if table == "import_losses" {
        return stream_import_losses(
            state.store_for(&identity).as_ref(),
            &project,
            &commit,
            &filters,
            cursor,
            limit,
            format,
        );
    }

    let root = projection::load(state.store_for(&identity).as_ref(), &project, &commit.hash)
        .map_err(ApiError::internal)?;
    let mut rows = materialise_table(
        state.store_for(&identity).as_ref(),
        &project,
        &commit,
        &root,
        &table,
    )?;
    rows.retain(|r| matches_filters(r, &filters));
    let total = rows.len();
    let next_cursor = if cursor + limit < total {
        Some((cursor + limit).to_string())
    } else {
        None
    };
    let page: Vec<Value> = rows.into_iter().skip(cursor).take(limit).collect();

    Ok(stream_page(
        &project,
        &commit.hash,
        &table,
        total,
        next_cursor,
        format,
        page.into_iter(),
    ))
}

fn validate_filters(table: &str, filters: &[Filter]) -> Result<(), ApiError> {
    let columns = projection::table_columns(table);
    for (column, _) in filters {
        if !columns.contains(&column.as_str()) {
            return Err(ApiError::bad_request(format!(
                "unknown filter column {} for table {}",
                column, table
            )));
        }
    }
    Ok(())
}

fn parse_limit(limit: Option<&str>) -> Result<usize, ApiError> {
    let limit = match limit {
        Some(raw) => raw
            .parse::<usize>()
            .map_err(|_| ApiError::bad_request("limit must be a positive integer"))?,
        None => DEFAULT_LIMIT,
    };
    if limit == 0 {
        return Err(ApiError::bad_request("limit must be a positive integer"));
    }
    Ok(limit.min(MAX_LIMIT))
}

/// Stream the import_losses table lazily: rows are built one at a time from the owned
/// mappings and never collected into a whole-table vector.
fn stream_import_losses(
    store: &dyn Store,
    project: &str,
    commit: &Commit,
    filters: &[Filter],
    cursor: usize,
    limit: usize,
    format: Format,
) -> Result<Response, ApiError> {
    let import =
        projection::import_losses_for(store, project, commit).map_err(ApiError::internal)?;

    let Some(import) = import else {
        // A non-import commit has an empty import_losses table.
        return Ok(stream_page(
            project,
            &commit.hash,
            "import_losses",
            0,
            None,
            format,
            std::iter::empty(),
        ));
    };

    let artifact_hash = import.artifact_hash.clone();
    let binding_id = import.binding_id.clone();
    let binding_version = import.binding_version.clone();
    let project_owned = project.to_string();

    // Count the rows matching the filters (one pass, no row materialisation).
    let total = {
        let ah = artifact_hash.clone();
        let bi = binding_id.clone();
        let bv = binding_version.clone();
        let pr = project_owned.clone();
        let f = filters.to_vec();
        import
            .report
            .mappings
            .iter()
            .filter(|m| {
                let row = projection::loss_row(&ah, &pr, &bi, &bv, m);
                matches_filters(&row, &f)
            })
            .count()
    };
    let next_cursor = if cursor + limit < total {
        Some((cursor + limit).to_string())
    } else {
        None
    };

    // The page iterator: builds each row lazily from the owned mappings.
    let ah = artifact_hash;
    let bi = binding_id;
    let bv = binding_version;
    let pr = project_owned;
    let f = filters.to_vec();
    let page = import
        .report
        .mappings
        .into_iter()
        .map(move |m| projection::loss_row(&ah, &pr, &bi, &bv, &m))
        .filter(move |row| matches_filters(row, &f))
        .skip(cursor)
        .take(limit);

    Ok(stream_page(
        project,
        &commit.hash,
        "import_losses",
        total,
        next_cursor,
        format,
        page,
    ))
}

/// Serialise a page of rows as JSON, CSV or NDJSON, streaming rather than buffering.
fn stream_page(
    project: &str,
    commit: &str,
    table: &str,
    total: usize,
    next_cursor: Option<String>,
    fmt: Format,
    rows: impl Iterator<Item = Value> + Send + 'static,
) -> Response {
    let mut headers = HeaderMap::new();
    headers.insert(
        HeaderName::from_static("x-mw-schema-version"),
        HeaderValue::from_static(SCHEMA_VERSION),
    );
    headers.insert(
        HeaderName::from_static("x-mw-total"),
        HeaderValue::from_str(&total.to_string()).expect("total is a valid header value"),
    );
    if let Some(cursor) = &next_cursor {
        headers.insert(
            HeaderName::from_static("x-mw-next-cursor"),
            HeaderValue::from_str(cursor).expect("cursor is a valid header value"),
        );
    }

    match fmt {
        Format::Json => {
            let prefix_obj = json!({
                "schemaVersion": SCHEMA_VERSION,
                "project": project,
                "commit": commit,
                "table": table,
                "total": total,
            });
            let mut prefix = serde_json::to_string(&prefix_obj).expect("envelope serialises");
            prefix.pop(); // drop the closing '}'
            prefix.push_str(",\"rows\":[");

            let suffix = match next_cursor {
                Some(cursor) => format!(
                    "],\"nextCursor\":{}}}",
                    serde_json::to_string(&cursor).expect("cursor serialises")
                ),
                None => "]}".to_string(),
            };

            let chunks = rows.enumerate().map(|(i, row)| {
                let bytes = serde_json::to_vec(&format::to_camel_value(&row)).map_err(io_err)?;
                if i == 0 {
                    Ok(bytes)
                } else {
                    let mut with_comma = Vec::with_capacity(bytes.len() + 1);
                    with_comma.push(b',');
                    with_comma.extend_from_slice(&bytes);
                    Ok(with_comma)
                }
            });
            headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
            (
                StatusCode::OK,
                headers,
                body_from_chunks(prefix.into_bytes(), chunks, suffix.into_bytes()),
            )
                .into_response()
        }
        Format::Csv => {
            let columns = projection::table_columns(table);
            let prefix = format!("{}\n", columns.join(",")).into_bytes();
            let chunks = rows.map(|row| {
                let cells: Vec<String> = columns.iter().map(|c| csv_cell(&row[c])).collect();
                Ok::<_, std::io::Error>(format!("{}\n", cells.join(",")).into_bytes())
            });
            headers.insert(CONTENT_TYPE, HeaderValue::from_static("text/csv"));
            (
                StatusCode::OK,
                headers,
                body_from_chunks(prefix, chunks, Vec::new()),
            )
                .into_response()
        }
        Format::Ndjson => {
            let chunks = rows.map(|row| {
                let mut line = serde_json::to_vec(&row).map_err(io_err)?;
                line.push(b'\n');
                Ok(line)
            });
            headers.insert(
                CONTENT_TYPE,
                HeaderValue::from_static("application/x-ndjson"),
            );
            (
                StatusCode::OK,
                headers,
                body_from_chunks(Vec::new(), chunks, Vec::new()),
            )
                .into_response()
        }
    }
}

fn csv_cell(value: &Value) -> String {
    let s = match value {
        Value::Null => String::new(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.clone(),
        other => serde_json::to_string(other).unwrap_or_default(),
    };
    if s.contains(',') || s.contains('"') || s.contains('\n') || s.contains('\r') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s
    }
}

// ---------------------------------------------------------------------------
// GET /analytics/{project}/metrics
// ---------------------------------------------------------------------------

pub async fn metrics(
    identity: Identity,
    State(state): State<ApiState>,
    Path(project): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Json<Value>, ApiError> {
    if !identity.may(Permission::Read) {
        return Err(ApiError::forbidden("read permission required"));
    }
    if !identity.may_reach(&project) {
        return Err(ApiError::forbidden("project not in scope"));
    }
    require_project(state.store_for(&identity).as_ref(), &project)?;
    let commit = resolve_commit(
        state.store_for(&identity).as_ref(),
        &project,
        params.get("commit").map(String::as_str),
        params.get("branch").map(String::as_str),
    )?;
    let root = projection::load(state.store_for(&identity).as_ref(), &project, &commit.hash)
        .map_err(ApiError::internal)?;
    let cnc =
        projection::constructs_not_carried(state.store_for(&identity).as_ref(), &project, &commit)
            .map_err(ApiError::internal)?;
    let mut rows = projection::metric_rows(&project, &commit, &root, cnc);
    projection::sort_rows("metrics", &mut rows);
    let rows: Vec<Value> = rows.iter().map(format::to_camel_value).collect();
    Ok(Json(json!({
        "schemaVersion": SCHEMA_VERSION,
        "project": project,
        "commit": commit.hash,
        "metrics": rows,
    })))
}

// ---------------------------------------------------------------------------
// GET /analytics/{project}/trend
// ---------------------------------------------------------------------------

pub async fn trend(
    identity: Identity,
    State(state): State<ApiState>,
    Path(project): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Json<Value>, ApiError> {
    if !identity.may(Permission::Read) {
        return Err(ApiError::forbidden("read permission required"));
    }
    if !identity.may_reach(&project) {
        return Err(ApiError::forbidden("project not in scope"));
    }
    let metric = params
        .get("metric")
        .filter(|m| !m.is_empty())
        .ok_or_else(|| ApiError::bad_request("metric is required"))?;
    let branch = params
        .get("branch")
        .filter(|b| !b.is_empty())
        .ok_or_else(|| ApiError::bad_request("branch is required"))?;
    if !analytics::metrics::ENGINE_METRIC_IDS
        .iter()
        .any(|id| id == metric)
    {
        return Err(ApiError::bad_request(format!("unknown metric {}", metric)));
    }
    require_project(state.store_for(&identity).as_ref(), &project)?;

    let commits = state
        .store_for(&identity)
        .commits_on(&project, branch)
        .map_err(map_store_error)?;
    let commits = slice_range(
        &commits,
        params.get("from").map(String::as_str),
        params.get("to").map(String::as_str),
    )?;

    let mut rows = Vec::new();
    for commit in &commits {
        let root = projection::load(state.store_for(&identity).as_ref(), &project, &commit.hash)
            .map_err(ApiError::internal)?;
        let cnc = projection::constructs_not_carried(
            state.store_for(&identity).as_ref(),
            &project,
            commit,
        )
        .map_err(ApiError::internal)?;
        if let Some(row) = projection::metric_rows(&project, commit, &root, cnc)
            .into_iter()
            .find(|r| r.get("metric_id").and_then(Value::as_str) == Some(metric.as_str()))
        {
            rows.push(format::to_camel_value(&row));
        }
    }
    if rows.is_empty() {
        return Err(ApiError::bad_request(format!(
            "metric {} produced no rows across branch {}",
            metric, branch
        )));
    }
    Ok(Json(json!({
        "schemaVersion": SCHEMA_VERSION,
        "project": project,
        "branch": branch,
        "metric": metric,
        "metrics": rows,
    })))
}

/// Slice a commit list to the [from, to] range, both inclusive, by hash in commit order.
fn slice_range(
    commits: &[Commit],
    from: Option<&str>,
    to: Option<&str>,
) -> Result<Vec<Commit>, ApiError> {
    if commits.is_empty() {
        return Ok(Vec::new());
    }
    let from_idx = match from {
        Some(h) => commits.iter().position(|c| c.hash == h).ok_or_else(|| {
            ApiError::bad_request(format!("commit {} not found on this branch", h))
        })?,
        None => 0,
    };
    let to_idx = match to {
        Some(h) => commits.iter().position(|c| c.hash == h).ok_or_else(|| {
            ApiError::bad_request(format!("commit {} not found on this branch", h))
        })?,
        None => commits.len() - 1,
    };
    if from_idx > to_idx {
        return Err(ApiError::bad_request(
            "the from commit is after the to commit",
        ));
    }
    Ok(commits[from_idx..=to_idx].to_vec())
}

// ---------------------------------------------------------------------------
// GET /metrics (OpenMetrics)
// ---------------------------------------------------------------------------

/// Whether the operator enabled the OpenMetrics endpoint. Off by default; MW_ENABLE_OPENMETRICS
/// with a truthy value turns it on.
pub fn openmetrics_enabled() -> bool {
    std::env::var("MW_ENABLE_OPENMETRICS")
        .ok()
        .map(|v| {
            matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "1" | "yes" | "true" | "on"
            )
        })
        .unwrap_or(false)
}

/// The gauges the endpoint exposes, each backed by a measure the schema carries today. The
/// value of mw_import_blocking_losses is the import report's blocking count (lossy +
/// unmappable) - the import_losses table's severity column - and 0 for a non-import commit.
const GAUGES: &[(&str, &str)] = &[
    (
        "mw_requirements_total",
        "Requirements in the model at the branch head.",
    ),
    (
        "mw_requirements_uncovered",
        "Requirements with no covering trace link at the branch head.",
    ),
    (
        "mw_requirements_coverage_ratio",
        "Covered requirements divided by total requirements at the branch head.",
    ),
    (
        "mw_graph_orphans",
        "Graph nodes with no incident edge at the branch head.",
    ),
    (
        "mw_import_blocking_losses",
        "Import blocking losses (lossy + unmappable) at the branch head.",
    ),
    (
        "mw_elements",
        "Graph nodes (every element as a node) at the branch head.",
    ),
];

pub async fn openmetrics(
    identity: Identity,
    State(state): State<ApiState>,
) -> Result<Response, ApiError> {
    if !identity.may(Permission::Read) {
        return Err(ApiError::forbidden("read permission required"));
    }
    if !openmetrics_enabled() {
        return Err(ApiError::service_unavailable(
            "the OpenMetrics endpoint is disabled; set MW_ENABLE_OPENMETRICS=1 to enable it",
        ));
    }

    let store = state.store_for(&identity);
    let mut projects = store.list_projects().map_err(map_store_error)?;
    projects.sort_by(|a, b| a.name.cmp(&b.name));

    let mut body = String::new();
    for (name, help) in GAUGES {
        body.push_str(&format!(
            "# HELP {} {}\n# TYPE {} gauge\n",
            name, help, name
        ));
    }

    for project in projects.iter().filter(|p| identity.may_reach(&p.name)) {
        let mut branches = store
            .list_branches(&project.name)
            .map_err(map_store_error)?;
        branches.sort_by(|a, b| a.0.cmp(&b.0));
        for (branch, tip) in branches {
            let Some(commit) = store.commit(&project.name, &tip).map_err(map_store_error)? else {
                continue;
            };
            let root = match projection::load(store.as_ref(), &project.name, &commit.hash) {
                Ok(root) => root,
                Err(e) => {
                    eprintln!("openmetrics: skipping {}@{}: {}", project.name, branch, e);
                    continue;
                }
            };
            let cnc = projection::constructs_not_carried(store.as_ref(), &project.name, &commit)
                .unwrap_or(0);
            let rows = projection::metric_rows(&project.name, &commit, &root, cnc);
            let get = |id: &str| {
                rows.iter()
                    .find(|r| r.get("metric_id").and_then(Value::as_str) == Some(id))
                    .and_then(|r| r.get("value").and_then(Value::as_i64))
                    .unwrap_or(0)
            };
            let req_total = get("coverage.total");
            let uncovered = get("coverage.uncovered");
            let covered = get("coverage.covered");
            let orphans = get("graph.isolated_count");
            let elements = get("graph.node_count");
            let ratio = if req_total > 0 {
                covered as f64 / req_total as f64
            } else {
                0.0
            };
            let blocking = projection::import_losses_for(store.as_ref(), &project.name, &commit)
                .map(|i| i.map(|l| l.report.blocking().len()).unwrap_or(0))
                .unwrap_or(0);

            let project_label = escape_label(&project.name);
            let branch_label = escape_label(&branch);
            let labels = format!("project=\"{}\",branch=\"{}\"", project_label, branch_label);
            body.push_str(&format!(
                "mw_requirements_total{{{}}} {}\n",
                labels, req_total
            ));
            body.push_str(&format!(
                "mw_requirements_uncovered{{{}}} {}\n",
                labels, uncovered
            ));
            body.push_str(&format!(
                "mw_requirements_coverage_ratio{{{}}} {}\n",
                labels, ratio
            ));
            body.push_str(&format!("mw_graph_orphans{{{}}} {}\n", labels, orphans));
            body.push_str(&format!(
                "mw_import_blocking_losses{{{}}} {}\n",
                labels, blocking
            ));
            body.push_str(&format!("mw_elements{{{}}} {}\n", labels, elements));
        }
    }

    let mut headers = HeaderMap::new();
    headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_static("text/plain; version=0.0.4; charset=utf-8"),
    );
    headers.insert(
        HeaderName::from_static("x-mw-schema-version"),
        HeaderValue::from_static(SCHEMA_VERSION),
    );
    Ok((StatusCode::OK, headers, axum::body::Body::from(body)).into_response())
}

/// Escape a label value for the OpenMetrics text exposition format.
fn escape_label(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            c => out.push(c),
        }
    }
    out
}
