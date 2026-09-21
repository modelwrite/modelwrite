// SPDX-License-Identifier: AGPL-3.0-or-later
//! Package 7: cross-transport equivalence evidence.
//!
//! "Four transports return the same rows" is a CLAIM. This test makes it checkable.
//! For the committed sample corpus and for one large-but-manageable real import it
//! reads EVERY mw-analytics-schema@1 table through
//!
//!   - REST        GET /analytics/{project}/tables/{table}   (paged to completion)
//!   - CLI --db    mw --db <db> analytics export ... --format ndjson
//!   - MCP         repo.table paged to completion, over the real HTTP transport
//!   - Python      modelwrite.analytics.tables(), a thin REST client
//!
//! and asserts the rows are identical. It then writes a DETERMINISTIC record to the
//! path in MW_EQUIVALENCE_EVIDENCE (docs/evidence/analytics-transport-equivalence.json
//! in CI), in the same regenerate-and-diff style as the mw-gate evidence records.
//!
//! Honesty notes recorded in the record itself:
//!   - MCP and Python are NOT independent implementations: both call the same REST
//!     endpoints. Only REST and CLI --db are separate Rust projections of the model.
//!   - The CLI in --server mode cannot fetch arbitrary table rows at all: its
//!     analytics tables route omits the /{table} segment (a 404 against the real
//!     route), and analytics export refuses --server by construction. This test
//!     probes and RECORDS that divergence rather than papering over it.
//!   - The 36 MB TMT import is deliberately not read through all four transports: it
//!     is ~190x larger than any other fixture AND, as server/tests/analytics_tmt.rs
//!     records, it does not pass the commit validator, so it cannot be committed and
//!     therefore cannot be read through the committed-model transports at all.
//!
//! The record is only produced when MW_EQUIVALENCE_EVIDENCE is set; without it the
//! test prints a loud SKIP and returns, so cargo test --workspace on a machine
//! without the sibling mw binary or Python stays green.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use serde_json::{json, Value};
use sha2::Digest;

use server::analytics::format::camel_case;
use server::analytics::projection::TABLES;
use server::auth::AuthConfig;
use server::store::sqlite::SqliteStore;
use server::store::Store;
use server::{app, AppState};

/// The page size used for REST and MCP. Deliberately small so the cursor path is
/// exercised on every table, not only on the large ones.
const PAGE: usize = 37;

/// The transports a table row can be read through. MCP and Python are REST clients;
/// the record says so.
const TRANSPORTS: &[&str] = &["rest", "cliDb", "mcp", "python"];

const TABLE_NAMES: &[&str] = &[
    "projects",
    "commits",
    "elements",
    "relationships",
    "requirements",
    "trace_links",
    "metrics",
    "metric_definitions",
    "import_losses",
];

fn tmp(prefix: &str) -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix(prefix)
        .tempdir()
        .expect("create a scratch directory")
}

// ---------------------------------------------------------------------------
// Normalisation: the wire is camelCase, the schema is snake_case.
// ---------------------------------------------------------------------------

/// camelCase -> snake_case. basisElementCount -> basis_element_count; a single-word
/// key passes through unchanged. This is the exact inverse of the server format::camel_case
/// for every schema column (asserted below).
fn snake_key(key: &str) -> String {
    let mut out = String::with_capacity(key.len() + 2);
    for (i, c) in key.chars().enumerate() {
        if c.is_ascii_uppercase() {
            if i > 0 {
                out.push('_');
            }
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

/// Recursively map every object key of a row to snake_case.
fn to_snake(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut out = serde_json::Map::new();
            for (k, v) in map {
                out.insert(snake_key(k), to_snake(v));
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(items.iter().map(to_snake).collect()),
        other => other.clone(),
    }
}

/// Collapse integral JSON numbers to integers so an Arrow float64 15.0 compares
/// equal to a REST integer 15. The Python bindings type metrics.value as float64
/// (bindings/python/_schema.py COLUMN_TYPES); that is a typed representation
/// difference, recorded under representationDifferences - not a value difference.
fn normalise_numbers(value: &Value) -> Value {
    match value {
        Value::Number(n) => {
            if let Some(f) = n.as_f64() {
                if f.is_finite() && f.fract() == 0.0 && f.abs() < 9.0e15 {
                    return json!(f as i64);
                }
            }
            value.clone()
        }
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, v)| (k.clone(), normalise_numbers(v)))
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(normalise_numbers).collect()),
        other => other.clone(),
    }
}

/// A row's canonical form for comparison: snake_case keys, integral numbers
/// collapsed.
fn canonical_row(row: &Value) -> String {
    serde_json::to_string(&normalise_numbers(&to_snake(row))).expect("a row serialises")
}

/// A row's canonical form with the two wall-clock columns blanked, for the
/// reproducible record digest.
fn digest_row(row: &Value) -> String {
    let mut value = normalise_numbers(&to_snake(row));
    blank_volatile(&mut value);
    serde_json::to_string(&value).expect("a row serialises")
}

/// Blank the two wall-clock columns so the record digests reproduce across runs.
/// The rows themselves (all transports, one commit) carry identical values and are
/// compared in full apart from the number normalisation above; only the RECORD needs
/// to survive a different run time.
fn blank_volatile(row: &mut Value) {
    if let Some(obj) = row.as_object_mut() {
        for key in ["created_at", "committed_at"] {
            if obj.get(key).map(Value::is_string).unwrap_or(false) {
                obj.insert(key.to_string(), json!("<time>"));
            }
        }
    }
}

fn canonical_strings(rows: &[Value], blank: bool) -> Vec<String> {
    let mut out: Vec<String> = rows
        .iter()
        .map(|row| {
            if blank {
                digest_row(row)
            } else {
                canonical_row(row)
            }
        })
        .collect();
    out.sort();
    out
}

fn digest(rows: &[Value]) -> String {
    let bytes =
        serde_json::to_vec(&canonical_strings(rows, true)).expect("canonical rows serialise");
    hex::encode(sha2::Sha256::digest(&bytes))
}

/// Whether two transports returned the same multiset of rows, in full (timestamps
/// included).
fn rows_equal(a: &[Value], b: &[Value]) -> bool {
    canonical_strings(a, false) == canonical_strings(b, false)
}

/// Whether a transport preserved the order REST returned rows in.
fn order_matches(a: &[Value], b: &[Value]) -> bool {
    let sa: Vec<String> = a.iter().map(canonical_row).collect();
    let sb: Vec<String> = b.iter().map(canonical_row).collect();
    sa == sb
}

/// The number of rows present on exactly one side, counted as a multiset symmetric
/// difference. One altered row therefore reports 2.
fn symmetric_difference(a: &[Value], b: &[Value]) -> usize {
    let mut counts: BTreeMap<String, i64> = BTreeMap::new();
    for row in a {
        *counts.entry(canonical_row(row)).or_default() += 1;
    }
    for row in b {
        *counts.entry(canonical_row(row)).or_default() -= 1;
    }
    counts.values().map(|c| c.unsigned_abs() as usize).sum()
}

// ---------------------------------------------------------------------------
// REST
// ---------------------------------------------------------------------------

fn http_json(url: &str) -> Result<Value, String> {
    let body = ureq::get(url)
        .call()
        .map_err(|e| format!("{url}: {e}"))?
        .into_string()
        .map_err(|e| format!("{url}: {e}"))?;
    serde_json::from_str(&body).map_err(|e| format!("{url}: {e}"))
}

/// Read one table through REST, following nextCursor to completion. Verifies the
/// envelope total against the number of rows actually read.
fn rest_table(base: &str, project: &str, table: &str, commit: &str) -> Result<Vec<Value>, String> {
    let mut rows: Vec<Value> = Vec::new();
    let mut cursor: Option<usize> = None;
    let mut total: Option<u64> = None;
    loop {
        let mut url =
            format!("{base}/analytics/{project}/tables/{table}?commit={commit}&limit={PAGE}");
        if let Some(cursor) = cursor {
            url.push_str(&format!("&cursor={cursor}"));
        }
        let page = http_json(&url)?;
        if let Some(t) = page.get("total").and_then(Value::as_u64) {
            total = Some(t);
        }
        rows.extend(
            page.get("rows")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default(),
        );
        match page.get("nextCursor").and_then(Value::as_str) {
            Some(next) => {
                cursor = Some(
                    next.parse()
                        .map_err(|e| format!("bad cursor {next}: {e}"))?,
                )
            }
            None => break,
        }
    }
    if let Some(total) = total {
        if total as usize != rows.len() {
            return Err(format!(
                "REST {table}: envelope total {total} but read {} rows",
                rows.len()
            ));
        }
    }
    Ok(rows)
}

// ---------------------------------------------------------------------------
// CLI
// ---------------------------------------------------------------------------

fn find_mw() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("MW_CLI_BIN") {
        return Some(PathBuf::from(path));
    }
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?.parent()?;
    for name in ["mw.exe", "mw"] {
        let candidate = dir.join(name);
        if candidate.exists() {
            return Some(candidate);
        }
    }
    None
}

/// Run mw --db <db> analytics export ... --format ndjson for one commit.
fn cli_export(
    mw: &Path,
    db: &Path,
    project: &str,
    commit: &str,
    out_dir: &Path,
) -> Result<(), String> {
    std::fs::create_dir_all(out_dir).map_err(|e| e.to_string())?;
    let output = Command::new(mw)
        .arg("--db")
        .arg(db)
        .args([
            "analytics",
            "export",
            project,
            "--commit",
            commit,
            "--format",
            "ndjson",
            "--out",
        ])
        .arg(out_dir)
        .output()
        .map_err(|e| format!("run mw: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "mw analytics export failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(())
}

fn read_ndjson(path: &Path) -> Result<Vec<Value>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut rows = Vec::new();
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        rows.push(serde_json::from_str(line).map_err(|e| format!("{}: {e}", path.display()))?);
    }
    Ok(rows)
}

/// Run mw and return (success, stdout, stderr).
fn run_mw(mw: &Path, args: &[String]) -> (bool, String, String) {
    let output = Command::new(mw).args(args).output().expect("run mw");
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
    )
}

// ---------------------------------------------------------------------------
// MCP (the real tool dispatch, over the real HTTP transport)
// ---------------------------------------------------------------------------

fn mcp_call(
    repo: &mcp::repository::Repository,
    tool: &str,
    arguments: Value,
) -> Result<Value, String> {
    let request = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": { "name": tool, "arguments": arguments }
    });
    let response = mcp::handle_request_with(&request.to_string(), repo);
    let message: Value =
        serde_json::from_str(&response).map_err(|e| format!("mcp response: {e}"))?;
    if let Some(error) = message.get("error") {
        return Err(format!("mcp error: {error}"));
    }
    let result = message.get("result").cloned().unwrap_or(Value::Null);
    if result
        .get("isError")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return Err(format!(
            "mcp tool {tool} failed: {}",
            result["content"][0]["text"]
                .as_str()
                .unwrap_or("(no detail)")
        ));
    }
    let text = result["content"][0]["text"]
        .as_str()
        .ok_or_else(|| format!("mcp tool {tool} returned no text"))?;
    serde_json::from_str(text).map_err(|e| format!("mcp tool {tool} text is not JSON: {e}"))
}

/// Read one table through repo.table, paging on nextCursor to completion.
fn mcp_table(
    repo: &mcp::repository::Repository,
    project: &str,
    table: &str,
    commit: &str,
) -> Result<Vec<Value>, String> {
    let mut rows: Vec<Value> = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let mut arguments = json!({
            "project": project,
            "table": table,
            "commit": commit,
            "limit": PAGE,
        });
        if let Some(cursor) = &cursor {
            arguments["cursor"] = json!(cursor);
        }
        let page = mcp_call(repo, "repo.table", arguments)?;
        rows.extend(
            page.get("rows")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default(),
        );
        match page.get("nextCursor").and_then(Value::as_str) {
            Some(next) => cursor = Some(next.to_string()),
            None => break,
        }
    }
    Ok(rows)
}

// ---------------------------------------------------------------------------
// Python (a thin REST client; not an independent implementation)
// ---------------------------------------------------------------------------

fn find_python() -> PathBuf {
    std::env::var("MW_PYTHON")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("python"))
}

fn python_tables(
    python: &Path,
    driver: &Path,
    package_path: &Path,
    base: &str,
    project: &str,
    commit: &str,
) -> Result<BTreeMap<String, Vec<Value>>, String> {
    let output = Command::new(python)
        .arg(driver)
        .env("MW_EQUIV_PYTHON_PATH", package_path)
        .env("MW_EQUIV_BASE_URL", base)
        .env("MW_EQUIV_PROJECT", project)
        .env("MW_EQUIV_COMMIT", commit)
        .output()
        .map_err(|e| format!("run python: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "python driver failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let parsed: Value =
        serde_json::from_slice(&output.stdout).map_err(|e| format!("python driver output: {e}"))?;
    let mut tables = BTreeMap::new();
    for &name in TABLE_NAMES {
        let rows = parsed["tables"][name]
            .as_array()
            .cloned()
            .unwrap_or_default();
        tables.insert(name.to_string(), rows);
    }
    Ok(tables)
}

fn snake_columns_match_schema() {
    for (name, columns) in TABLES {
        for &column in *columns {
            let camel = camel_case(column);
            assert_eq!(
                snake_key(&camel),
                column,
                "normalisation must invert the server naming for {name}.{column}"
            );
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rest_cli_mcp_and_python_return_the_same_rows() {
    let Ok(evidence_path) = std::env::var("MW_EQUIVALENCE_EVIDENCE") else {
        eprintln!(
            "equivalence: SKIPPED - set MW_EQUIVALENCE_EVIDENCE to a path to run the \
             cross-transport evidence test"
        );
        return;
    };

    snake_columns_match_schema();

    let Some(mw) = find_mw() else {
        panic!(
            "the mw CLI binary was not found; set MW_CLI_BIN, or build it with \
             cargo build -p mw-cli before running this test"
        );
    };
    let python = find_python();

    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let driver = root.join("tests").join("python_equivalence.py");
    let package_path = root.join("../bindings/python");

    // ---- the scratch store and the two committed cases ---------------------

    let dir = tmp("mw-equivalence-");
    let db = dir.path().join("mw.db");
    let store = Arc::new(SqliteStore::open(&db).expect("open the scratch store"));

    // Case 1: the committed sample corpus, authored.
    store.create_project("coffee-sample", None).unwrap();
    let corpus = test_support::load_okf_expected().into_bytes();
    let okf_hash = store.put_blob(&corpus).unwrap();
    let sample_commit = store
        .commit_model(
            "coffee-sample",
            "main",
            &okf_hash,
            "alex",
            "the sample corpus",
            None,
            None,
            None,
        )
        .unwrap();
    assert_eq!(sample_commit.hash.len(), 64);

    // Case 2: one large-but-manageable real import. The MagicDraw coffee-machine
    // XMI is the largest committable import in the repository; it carries a real
    // provenance record and a non-empty import_losses table.
    store.create_project("coffee-import", None).unwrap();
    let xmi_path = root.join("../real-world/mdzip/model.xmi");
    let xmi = std::fs::read(&xmi_path).expect("the MagicDraw coffee-machine XMI is committed");
    let run_import = |accept: &[String]| -> Result<server::binding_api::ImportOutcome, String> {
        server::binding_api::import_core(
            store.as_ref(),
            "coffee-import",
            &server::binding_api::ImportCore {
                binding: "sysml-v1-xmi@2.4",
                branch: "main",
                author: "alex",
                message: "import the MagicDraw coffee-machine XMI",
                artifact: &xmi,
                accept_losses: accept,
                holder: None,
                actor: "alex",
                mechanism: "equivalence-test",
                authorizer: "",
                acceptance: None,
            },
        )
        .map_err(|e| format!("import_core: {e:?}"))
    };
    let blocking = run_import(&[]).expect("the first import attempt returns the blocking losses");
    let identities: Vec<String> = match blocking {
        server::binding_api::ImportOutcome::Blocking { unaccepted, .. } => unaccepted
            .iter()
            .map(agent::losses::entry_identity)
            .collect(),
        server::binding_api::ImportOutcome::Committed { .. } => {
            panic!("the MagicDraw import committed with nothing accepted")
        }
    };
    assert!(
        !identities.is_empty(),
        "the import must have blocking losses to accept"
    );
    let imported = run_import(&identities).expect("accepting every blocking loss commits");
    let import_commit = match imported {
        server::binding_api::ImportOutcome::Committed { commit, .. } => commit,
        server::binding_api::ImportOutcome::Blocking { unaccepted, .. } => {
            panic!("import still blocking on {} losses", unaccepted.len())
        }
    };

    // ---- a real HTTP server over the same store ----------------------------

    let router = app(AppState {
        store: store.clone(),
        evidence_dir: dir.path().join("evidence"),
        auth: AuthConfig::Open,
    });
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("bind a scratch port");
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    let mut up = false;
    for _ in 0..100 {
        if http_json(&format!("{base}/health")).is_ok() {
            up = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    assert!(up, "the scratch server did not come up at {base}");

    // MCP is the real tool dispatch over the real HTTP transport.
    let repo = mcp::repository::Repository::configured(&base, "equivalence-test-token");

    // ---- capture every table through every transport -----------------------

    struct Case {
        name: &'static str,
        project: &'static str,
        commit: String,
        provenance: &'static str,
        source: String,
        /// table -> transport -> rows
        rows: BTreeMap<String, BTreeMap<String, Vec<Value>>>,
    }

    let cases: Vec<(&'static str, &'static str, String, &'static str, String)> = vec![
        (
            "sample-corpus",
            "coffee-sample",
            sample_commit.hash.clone(),
            "authored",
            format!(
                "sample/corpus/coffee-machine/okf/expected/coffee_machine_model.json ({} bytes)",
                corpus.len()
            ),
        ),
        (
            "large-import",
            "coffee-import",
            import_commit.hash.clone(),
            "imported(sysml-v1-xmi@2.4)",
            format!(
                "real-world/mdzip/model.xmi ({} bytes, {} accepted losses)",
                xmi.len(),
                identities.len()
            ),
        ),
    ];

    let mut captures: Vec<Case> = Vec::new();
    for (name, project, commit, provenance, source) in &cases {
        let name = *name;
        let project = *project;
        let provenance = *provenance;
        let export_dir = dir.path().join(format!("export-{name}"));
        cli_export(&mw, &db, project, commit, &export_dir)
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        let python_rows = python_tables(&python, &driver, &package_path, &base, project, commit)
            .unwrap_or_else(|e| panic!("{name}: {e}"));

        let mut table_rows: BTreeMap<String, BTreeMap<String, Vec<Value>>> = BTreeMap::new();
        for &table in TABLE_NAMES {
            let rest = rest_table(&base, project, table, commit)
                .unwrap_or_else(|e| panic!("{name}/{table}: {e}"));
            let cli = read_ndjson(&export_dir.join(format!("{table}.ndjson")))
                .unwrap_or_else(|e| panic!("{name}/{table}: {e}"));
            let mcp = mcp_table(&repo, project, table, commit)
                .unwrap_or_else(|e| panic!("{name}/{table}: {e}"));
            let py = python_rows
                .get(table)
                .cloned()
                .unwrap_or_else(|| panic!("{name}/{table}: python returned no table"));

            let mut by_transport = BTreeMap::new();
            by_transport.insert("rest".to_string(), rest);
            by_transport.insert("cliDb".to_string(), cli);
            by_transport.insert("mcp".to_string(), mcp);
            by_transport.insert("python".to_string(), py);
            table_rows.insert(table.to_string(), by_transport);
        }
        captures.push(Case {
            name,
            project,
            commit: commit.clone(),
            provenance,
            source: source.clone(),
            rows: table_rows,
        });
    }

    // ---- assert equivalence and build the record ---------------------------

    let mut case_records = Vec::new();
    let mut all_identical = true;
    let mut deliberate = Value::Null;
    for case in &captures {
        let mut tables = serde_json::Map::new();
        let mut case_identical = true;
        for &table in TABLE_NAMES {
            let by_transport = &case.rows[table];
            let rest = &by_transport["rest"];
            let mut identical = true;
            let mut transports = serde_json::Map::new();
            let mut order = serde_json::Map::new();
            for &transport in TRANSPORTS {
                let rows = &by_transport[transport];
                let same = rows_equal(rest, rows);
                identical &= same;
                order.insert(transport.to_string(), json!(order_matches(rest, rows)));
                transports.insert(
                    transport.to_string(),
                    json!({ "rows": rows.len(), "digest": digest(rows), "identicalToRest": same }),
                );
            }
            if !identical {
                case_identical = false;
                all_identical = false;
                eprintln!(
                    "DIVERGENCE in {} / {}: {}",
                    case.name,
                    table,
                    serde_json::to_string(&transports).unwrap()
                );
            }
            tables.insert(
                table.to_string(),
                json!({
                    "rowCount": rest.len(),
                    "digest": digest(rest),
                    "identical": identical,
                    "byTransport": transports,
                    "orderMatchesRest": order,
                }),
            );
        }
        assert!(
            case_identical,
            "{}: the four transports did not return identical rows",
            case.name
        );
        case_records.push(json!({
            "name": case.name,
            "project": case.project,
            "commit": case.commit,
            "provenance": case.provenance,
            "source": case.source,
            "tables": tables,
            "everyTableIdentical": case_identical,
        }));

        // The deliberate-failure case, run on the FIRST case only: take the real
        // REST rows for elements, corrupt the CLI --db capture of one row, and run
        // the SAME comparator again. This is the analogue of the gate corrupted
        // fixture run: it records that the checker can fail.
        if deliberate.is_null() {
            let rest = &case.rows["elements"]["rest"];
            let mut mutated = case.rows["elements"]["cliDb"].clone();
            assert!(
                !mutated.is_empty(),
                "the deliberate failure needs a non-empty table"
            );
            mutated[0]["name"] = json!("EQUIVALENCE DELIBERATE FAILURE (injected)");
            let detected = !rows_equal(rest, &mutated);
            let differing = symmetric_difference(rest, &mutated);
            deliberate = json!({
                "method": "one elements row of the real cli-db capture had its name replaced with a sentinel before re-running the same comparator against rest",
                "case": case.name,
                "table": "elements",
                "transportsCompared": ["rest", "cliDb"],
                "detected": detected,
                "symmetricDifferenceRows": differing,
            });
            assert!(
                detected,
                "the comparator must detect a deliberately corrupted capture"
            );
            assert!(differing > 0);
        }
    }
    assert!(
        all_identical,
        "every table must be identical across all four transports"
    );

    // ---- the CLI --server route check --------------------------------------

    let server = base.as_str();
    let (schema_ok, schema_out, schema_err) = run_mw(
        &mw,
        &[
            "--server".into(),
            server.into(),
            "analytics".into(),
            "schema".into(),
        ],
    );
    assert!(schema_ok, "analytics schema over --server: {schema_err}");
    let schema_json: Value = serde_json::from_str(&schema_out).expect("schema JSON");
    assert_eq!(schema_json["tables"].as_array().unwrap().len(), 9);

    let (metrics_ok, metrics_out, metrics_err) = run_mw(
        &mw,
        &[
            "--server".into(),
            server.into(),
            "analytics".into(),
            "metrics".into(),
            "coffee-sample".into(),
            "--commit".into(),
            sample_commit.hash.clone(),
        ],
    );
    assert!(metrics_ok, "analytics metrics over --server: {metrics_err}");
    let metrics_json: Value = serde_json::from_str(&metrics_out).expect("metrics JSON");
    let cli_server_metrics = metrics_json["metrics"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let rest_metrics = &captures[0].rows["metrics"]["rest"];
    let cli_server_metrics_match = rows_equal(rest_metrics, &cli_server_metrics);

    let (trend_ok, _trend_out, trend_err) = run_mw(
        &mw,
        &[
            "--server".into(),
            server.into(),
            "analytics".into(),
            "trend".into(),
            "coffee-sample".into(),
            "--metric".into(),
            "coverage.covered".into(),
            "--branch".into(),
            "main".into(),
        ],
    );
    assert!(trend_ok, "analytics trend over --server: {trend_err}");

    let (tables_ok, _tables_out, tables_err) = run_mw(
        &mw,
        &[
            "--server".into(),
            server.into(),
            "analytics".into(),
            "tables".into(),
            "coffee-sample".into(),
            "--commit".into(),
            sample_commit.hash.clone(),
        ],
    );
    let (export_ok, _export_out, export_err) = run_mw(
        &mw,
        &[
            "--server".into(),
            server.into(),
            "analytics".into(),
            "export".into(),
            "coffee-sample".into(),
            "--commit".into(),
            sample_commit.hash.clone(),
            "--format".into(),
            "ndjson".into(),
            "--out".into(),
            dir.path()
                .join("server-export")
                .to_string_lossy()
                .to_string(),
        ],
    );

    let cli_server = json!({
        "note": "The CLI in --server mode is a set of planned routes. schema/metrics/trend match the real REST routes and return rows; there is no working route for arbitrary table rows. This is recorded, not fixed.",
        "routes": [
            {
                "command": "mw --server <url> analytics schema",
                "cliUrl": "/analytics/schema",
                "serverRoute": "GET /analytics/schema",
                "exitCode": if schema_ok { 0 } else { 1 },
                "outcome": if schema_ok { "rows" } else { "error" },
                "matches": true,
            },
            {
                "command": "mw --server <url> analytics metrics <project> --commit <hash>",
                "cliUrl": format!("/analytics/coffee-sample/metrics?commit={}", sample_commit.hash),
                "serverRoute": "GET /analytics/:project/metrics",
                "exitCode": if metrics_ok { 0 } else { 1 },
                "outcome": if metrics_ok { "rows" } else { "error" },
                "matches": true,
                "rowsIdenticalToRestMetrics": cli_server_metrics_match,
                "rowsCompared": cli_server_metrics.len(),
            },
            {
                "command": "mw --server <url> analytics trend <project> --metric coverage.covered --branch main",
                "cliUrl": "/analytics/coffee-sample/trend?metric=coverage.covered&branch=main",
                "serverRoute": "GET /analytics/:project/trend",
                "exitCode": if trend_ok { 0 } else { 1 },
                "outcome": if trend_ok { "rows" } else { "error" },
                "matches": true,
            },
            {
                "command": "mw --server <url> analytics tables <project> --commit <hash>",
                "cliUrl": format!("/analytics/coffee-sample/tables?commit={}", sample_commit.hash),
                "serverRoute": "GET /analytics/:project/tables/:table",
                "exitCode": if tables_ok { 0 } else { 1 },
                "outcome": if tables_ok { "rows" } else { "error" },
                "matches": false,
                "divergence": "the CLI omits the /{table} segment (cli/src/http.rs:246), so the request is a 404 against the real route (server/src/lib.rs:310) and no table rows can be fetched in --server mode",
                "stderr": tables_err.lines().next().unwrap_or(""),
            },
            {
                "command": "mw --server <url> analytics export <project> --commit <hash> --format ndjson --out <dir>",
                "cliUrl": "(no request: refused before the network)",
                "serverRoute": "GET /analytics/:project/tables/:table (one per table)",
                "exitCode": if export_ok { 0 } else { 1 },
                "outcome": if export_ok { "rows" } else { "refused" },
                "matches": false,
                "divergence": "the CLI refuses analytics export over --server by construction (cli/src/http.rs:250-253); --db is the CLI row-export path",
                "stderr": export_err.lines().next().unwrap_or(""),
            }
        ]
    });
    assert!(
        cli_server_metrics_match,
        "the CLI --server metrics route returns rows, so they must match REST"
    );

    // ---- the record --------------------------------------------------------

    let record = json!({
        "schemaVersion": "mw-analytics-schema@1",
        "recordType": "analytics cross-transport equivalence",
        "recordVersion": 1,
        "generator": "server/tests/equivalence.rs (analytics interfaces brief, package 7)",
        "normalisation": "REST JSON and MCP wire rows are camelCase; the schema logical columns and the CLI --db NDJSON export are snake_case. Every transport rows are converted to snake_case with the exact inverse of the server analytics::format::camel_case (asserted against analytics::projection::TABLES) before comparison. Row order is compared both as a multiset and as the REST natural order.",
        "numberNormalisation": "Integral JSON numbers are collapsed to integers before comparison, so the Python bindings Arrow float64 representation of metrics.value (15.0) equals the REST integer (15). Non-integral numbers are compared as JSON numbers. The ROWS are otherwise compared in full, wall-clock columns included.",
        "representationDifferences": [
            {
                "transport": "python",
                "column": "metrics.value",
                "detail": "bindings/python types metrics.value as float64 (its Arrow mirror of the schema), so an integral metric value is delivered as 15.0 where the REST JSON delivers 15. The comparison normalises integral numbers; the underlying value is identical. This is the ONLY representation difference found; every other table compared byte-for-byte after key normalisation."
            }
        ],
        "pagination": format!("REST limit={PAGE} and MCP limit={PAGE}, each followed on nextCursor to completion; the Python client pages internally at its own 1000-row ceiling. The page size is small on purpose so the cursor path is exercised on every table."),
        "transports": {
            "rest": { "implementation": "server/src/analytics (Rust projection), HTTP JSON", "independentProjection": true },
            "cliDb": { "implementation": "cli/src/analytics.rs (a separate Rust projection that mirrors the server one), NDJSON export", "independentProjection": true },
            "mcp": { "implementation": "engine/mcp repo.table, a pass-through of the REST JSON over the real HTTP transport", "independentProjection": false },
            "python": { "implementation": "bindings/python modelwrite.analytics.tables(), a thin REST client (requests). It computes nothing.", "independentProjection": false }
        },
        "independence": "Only REST and CLI --db are independent implementations; MCP and Python both read the same REST endpoints, so their agreement is a weaker claim (transport and naming equivalence, not independent computation). This record states that rather than implying four independent implementations.",
        "cases": case_records,
        "cliServer": cli_server,
        "deliberateFailure": deliberate,
        "notRun": [
            {
                "item": "the 36 MB Open-MBEE TMT XMI through the four transports",
                "reason": "It is ~190x larger than the fixtures used here, AND server/tests/analytics_tmt.rs already records that the imported model does not pass the commit validator (two requirements with an empty reqId), so it cannot be committed and therefore cannot be read through any committed-model transport."
            },
            {
                "item": "CLI --server table rows",
                "reason": "there is no working CLI --server route for arbitrary table rows: analytics tables is a 404 (missing /{table} segment) and analytics export refuses --server. The CLI row transport exercised here is --db. The route divergence is recorded under cliServer rather than papered over."
            }
        ],
        "passed": all_identical && cli_server_metrics_match && deliberate["detected"] == json!(true),
    });

    let mut text = serde_json::to_string_pretty(&record).expect("the record serialises");
    text.push('\n');
    let evidence_path = PathBuf::from(evidence_path);
    if let Some(parent) = evidence_path.parent() {
        std::fs::create_dir_all(parent).expect("create the evidence directory");
    }
    std::fs::write(&evidence_path, text).expect("write the evidence record");

    println!("=== ANALYTICS CROSS-TRANSPORT EQUIVALENCE ===");
    for case in &captures {
        let mut counts: Vec<String> = TABLE_NAMES
            .iter()
            .map(|t| format!("{}={}", t, case.rows[*t]["rest"].len()))
            .collect();
        counts.sort();
        println!("{}: {} identical", case.name, counts.join(" "));
    }
    println!("record written to {}", evidence_path.display());
}
