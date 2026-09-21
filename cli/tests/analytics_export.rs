// SPDX-License-Identifier: AGPL-3.0-or-later
//! End-to-end tests for the "mw analytics" export: determinism (byte-identical CSV/NDJSON
//! across two runs) and the DuckDB hive_partitioning row-count proof. These run the real mw
//! binary against a temporary SQLite store carrying the sample corpus.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;
use server::store::{sqlite::SqliteStore, Store};

/// A temporary store with the coffee-machine corpus committed on "main".
struct Fixture {
    _dir: tempfile::TempDir,
    db: PathBuf,
    commit: String,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("mw.db");
    let store = SqliteStore::open(&db).unwrap();
    store.create_project("coffee", None).unwrap();
    let bytes = std::fs::read(test_support::okf_expected()).unwrap();
    let okf_hash = store.put_blob(&bytes).unwrap();
    store
        .commit_model(
            "coffee", "main", &okf_hash, "alex", "initial", None, None, None,
        )
        .unwrap();
    let commit = store.branch_tip("coffee", "main").unwrap().unwrap();
    Fixture {
        _dir: dir,
        db,
        commit,
    }
}

fn run_mw(db: &Path, args: &[&str]) -> (bool, Vec<u8>, Vec<u8>) {
    let out = Command::new(env!("CARGO_BIN_EXE_mw"))
        .arg("--db")
        .arg(db)
        .args(args)
        .output()
        .expect("run mw");
    (out.status.success(), out.stdout, out.stderr)
}

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

fn read_file(path: &Path) -> Vec<u8> {
    std::fs::read(path).unwrap_or_else(|e| panic!("cannot read {}: {}", path.display(), e))
}

#[test]
fn export_csv_and_ndjson_are_byte_identical_across_two_runs() {
    let f = fixture();
    for format in ["csv", "ndjson"] {
        let out1 = f._dir.path().join(format!("out1-{}", format));
        let out2 = f._dir.path().join(format!("out2-{}", format));
        for (out, dirname) in [(out1, "run1"), (out2, "run2")] {
            let (ok, _stdout, stderr) = run_mw(
                &f.db,
                &[
                    "analytics",
                    "export",
                    "coffee",
                    "--commit",
                    &f.commit,
                    "--format",
                    format,
                    "--out",
                    out.to_str().unwrap(),
                ],
            );
            assert!(
                ok,
                "export {} to {} failed: {}",
                format,
                dirname,
                String::from_utf8_lossy(&stderr)
            );
        }

        for table in TABLE_NAMES {
            let name = format!("{}.{}", table, format);
            let a = read_file(&f._dir.path().join(format!("out1-{}", format)).join(&name));
            let b = read_file(&f._dir.path().join(format!("out2-{}", format)).join(&name));
            assert_eq!(
                a, b,
                "{} differs between two {} exports of the same commit",
                name, format
            );
            // A CSV file always carries its header row, even for an empty table (the authored
            // corpus has no import losses); an NDJSON file for an empty table is empty by design.
            if format == "csv" {
                assert!(
                    !a.is_empty(),
                    "{} is empty; the CSV export must write a header",
                    name
                );
            }
        }
    }
}

#[test]
fn duckdb_reads_the_parquet_tree_with_hive_partitioning_and_row_counts_match() {
    if !duckdb_available() {
        eprintln!("skipping: python + duckdb not available");
        return;
    }
    let f = fixture();
    let out = f._dir.path().join("pq");
    let (ok, _stdout, stderr) = run_mw(
        &f.db,
        &[
            "analytics",
            "export",
            "coffee",
            "--commit",
            &f.commit,
            "--format",
            "parquet",
            "--out",
            out.to_str().unwrap(),
        ],
    );
    assert!(
        ok,
        "parquet export failed: {}",
        String::from_utf8_lossy(&stderr)
    );

    let (ok, stdout, stderr) = run_mw(
        &f.db,
        &["analytics", "tables", "coffee", "--commit", &f.commit],
    );
    assert!(
        ok,
        "analytics tables failed: {}",
        String::from_utf8_lossy(&stderr)
    );
    let tables: Value = serde_json::from_slice(&stdout).expect("tables returns JSON");
    let expected: std::collections::HashMap<String, u64> = tables["tables"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| {
            (
                t["name"].as_str().unwrap().to_string(),
                t["rowCount"].as_u64().unwrap(),
            )
        })
        .collect();

    let duck = run_duckdb_counts(&out);
    for table in TABLE_NAMES {
        let got = *duck
            .get(*table)
            .unwrap_or_else(|| panic!("no DuckDB count for {}", table));
        let want = *expected
            .get(*table)
            .unwrap_or_else(|| panic!("no table count for {}", table));
        assert_eq!(
            got, want,
            "DuckDB counted {} rows in table {} but mw analytics tables says {}",
            got, table, want
        );
    }
}

fn duckdb_available() -> bool {
    Command::new("python")
        .args(["-c", "import duckdb"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[test]
fn analytics_commands_return_their_json_shapes_with_basis() {
    let f = fixture();

    // schema: tables, columns and metric definitions.
    let (ok, out, err) = run_mw(&f.db, &["analytics", "schema"]);
    assert!(ok, "schema failed: {}", String::from_utf8_lossy(&err));
    let v: Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(v["schemaVersion"], "mw-analytics-schema@1");
    assert_eq!(v["tables"].as_array().unwrap().len(), 9);
    assert_eq!(v["metricDefinitions"].as_array().unwrap().len(), 25);

    // metrics: 25 rows, each carrying its basis.
    let (ok, out, err) = run_mw(
        &f.db,
        &["analytics", "metrics", "coffee", "--commit", &f.commit],
    );
    assert!(ok, "metrics failed: {}", String::from_utf8_lossy(&err));
    let v: Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(v["schemaVersion"], "mw-analytics-schema@1");
    assert_eq!(v["commit"].as_str(), Some(f.commit.as_str()));
    assert_eq!(v["metrics"].as_array().unwrap().len(), 25);
    for m in v["metrics"].as_array().unwrap() {
        assert!(
            m.get("basis_element_count").is_some(),
            "metric lacks basis_element_count: {}",
            m
        );
        assert!(m.get("basis_relationship_count").is_some());
        assert!(m.get("constructs_not_carried").is_some());
        assert!(m.get("basis_note").is_some());
        assert!(m.get("engine_version").is_some());
        assert!(m.get("evidence_hash").is_some());
    }

    // trend: one metric across the branch's commits.
    let (ok, out, err) = run_mw(
        &f.db,
        &[
            "analytics",
            "trend",
            "coffee",
            "--metric",
            "coverage.covered",
            "--branch",
            "main",
        ],
    );
    assert!(ok, "trend failed: {}", String::from_utf8_lossy(&err));
    let v: Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(v["metric"].as_str(), Some("coverage.covered"));
    assert_eq!(v["metrics"].as_array().unwrap().len(), 1);

    // metrics CSV: header + 25 rows on stdout.
    let (ok, out, err) = run_mw(
        &f.db,
        &[
            "analytics",
            "metrics",
            "coffee",
            "--commit",
            &f.commit,
            "--format",
            "csv",
        ],
    );
    assert!(ok, "metrics csv failed: {}", String::from_utf8_lossy(&err));
    let text = String::from_utf8(out).unwrap();
    let first = text.lines().next().unwrap();
    assert!(
        first.starts_with("project,commit,metric_id,"),
        "csv header wrong: {}",
        first
    );
    assert_eq!(text.lines().count(), 26, "header plus 25 metric rows");
}

/// Count every table's rows in the Parquet tree using DuckDB's hive_partitioning, so the
/// project= and commit= partitions are read back from the directory layout.
fn run_duckdb_counts(out: &Path) -> std::collections::HashMap<String, u64> {
    let tables = TABLE_NAMES.join(",");
    let script = r#"
import duckdb, json, sys
out = sys.argv[1].replace(chr(92), '/')
tables = sys.argv[2].split(',')
con = duckdb.connect()
result = {}
for t in tables:
    pattern = out + '/' + t + '/*/*/*.parquet'
    result[t] = con.execute("SELECT count(*) FROM read_parquet(?, hive_partitioning=true)", [pattern]).fetchone()[0]
print(json.dumps(result))
"#;
    let output = Command::new("python")
        .args(["-c", script, out.to_str().unwrap(), &tables])
        .output()
        .expect("run python duckdb");
    assert!(
        output.status.success(),
        "python duckdb failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    let map: std::collections::HashMap<String, u64> =
        serde_json::from_str(stdout.trim()).expect("duckdb returns a JSON object");
    map
}
