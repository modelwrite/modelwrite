// SPDX-License-Identifier: AGPL-3.0-or-later
//! Integration tests for the mw command line: argument parsing, exit codes, the offline
//! commit/log round trip, and the HTTP path driven against an in-process router bound to
//! an ephemeral port.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::Arc;

use serde_json::Value;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_mw")
}

/// Run the mw binary with the given args and extra environment, returning its output.
fn run_mw(args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut cmd = Command::new(bin());
    cmd.args(args);
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd.output().expect("failed to run mw")
}

/// Like run_mw, but also removes the named environment variables from the child so a test
/// can prove the "unset variable" path is honest.
fn run_mw_removing(args: &[&str], env: &[(&str, &str)], remove: &[&str]) -> Output {
    let mut cmd = Command::new(bin());
    cmd.args(args);
    for (k, v) in env {
        cmd.env(k, v);
    }
    for k in remove {
        cmd.env_remove(k);
    }
    cmd.output().expect("failed to run mw")
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).to_string()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).to_string()
}

/// A minimal but VALID OKF document: project, a state machine, and a one-node graph.
fn okf_json(project: &str) -> Value {
    serde_json::json!({
        "okf": "1.0",
        "project": project,
        "summary": {
            "blocks": 0,
            "requirements": 0,
            "interfaces": 0,
            "signals": 0,
            "activities": 0,
            "graphNodes": 1,
            "graphEdges": 0
        },
        "stateMachine": { "name": "sm", "regions": [] },
        "graph": { "nodes": [ { "id": "n1", "kind": "block" } ], "edges": [] }
    })
}

fn write_file(dir: &Path, name: &str, contents: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, contents).unwrap();
    path
}

#[test]
fn offline_commit_then_log_round_trips() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("mw.db");
    let db = db.to_str().unwrap();
    let model = write_file(dir.path(), "model.json", &okf_json("coffee").to_string());

    let out = run_mw(&["--db", db, "project", "create", "coffee"], &[]);
    assert!(
        out.status.success(),
        "project create failed: {}",
        stderr(&out)
    );

    let out = run_mw(
        &[
            "--db",
            db,
            "commit",
            "coffee",
            "--branch",
            "main",
            "--message",
            "first commit",
            "--file",
            model.to_str().unwrap(),
        ],
        &[],
    );
    assert!(out.status.success(), "commit failed: {}", stderr(&out));
    let commit: Value = serde_json::from_str(&stdout(&out)).unwrap();
    let hash = commit["hash"].as_str().unwrap().to_string();
    assert_eq!(commit["message"], "first commit");
    assert_eq!(commit["branch"], "main");
    assert_eq!(commit["parents"], serde_json::json!([]));

    let out = run_mw(&["--db", db, "log", "coffee", "--branch", "main"], &[]);
    assert!(out.status.success(), "log failed: {}", stderr(&out));
    let commits: Value = serde_json::from_str(&stdout(&out)).unwrap();
    let commits = commits.as_array().unwrap();
    assert_eq!(commits.len(), 1);
    assert_eq!(commits[0]["hash"], hash.as_str());
    assert_eq!(commits[0]["message"], "first commit");
}

#[test]
fn a_failed_command_exits_non_zero_with_a_message() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("mw.db");
    let db = db.to_str().unwrap();
    let model = write_file(dir.path(), "model.json", &okf_json("coffee").to_string());

    // Committing into a project that does not exist must fail and say why.
    let out = run_mw(
        &[
            "--db",
            db,
            "commit",
            "coffee",
            "--branch",
            "main",
            "--message",
            "m",
            "--file",
            model.to_str().unwrap(),
        ],
        &[],
    );
    assert!(
        !out.status.success(),
        "a commit into a missing project must fail"
    );
    assert!(
        !stderr(&out).trim().is_empty(),
        "the failure must carry a message a person can act on"
    );
}

#[test]
fn an_unknown_flag_is_a_clear_error() {
    let out = run_mw(
        &[
            "--db", "x.db", "commit", "coffee", "--branch", "main", "--bogus",
        ],
        &[],
    );
    assert!(!out.status.success());
    let err = stderr(&out);
    assert!(err.contains("unknown flag"), "stderr was: {}", err);
    assert!(err.contains("--bogus"), "stderr was: {}", err);
}

#[test]
fn no_arguments_is_a_clear_error() {
    let out = run_mw(&[], &[]);
    assert!(!out.status.success());
    assert!(!stderr(&out).trim().is_empty());
}

#[test]
fn an_https_url_is_refused_with_a_message_about_the_proxy() {
    let out = run_mw(&["--server", "https://example.com", "project", "list"], &[]);
    assert!(!out.status.success());
    let err = stderr(&out);
    assert!(err.contains("https"), "stderr was: {}", err);
    assert!(err.contains("proxy"), "stderr was: {}", err);
}

// ---------------------------------------------------------------------------
// The HTTP path, exercised against the in-process router bound to an ephemeral port.
// ---------------------------------------------------------------------------

fn app_state(dir: &Path, auth: server::auth::AuthConfig) -> server::AppState {
    let store = server::store::sqlite::SqliteStore::open(&dir.join("mw.db")).unwrap();
    server::AppState {
        store: Arc::new(store),
        evidence_dir: dir.to_path_buf(),
        auth,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn http_path_talks_to_an_in_process_router() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(app_state(dir.path(), server::auth::AuthConfig::Open));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let serve = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let url = format!("http://{}", addr);

    let out = run_mw(&["--server", &url, "project", "create", "coffee"], &[]);
    assert!(
        out.status.success(),
        "project create failed: {}",
        stderr(&out)
    );

    let out = run_mw(&["--server", &url, "project", "list"], &[]);
    assert!(
        out.status.success(),
        "project list failed: {}",
        stderr(&out)
    );
    let projects: Value = serde_json::from_str(&stdout(&out)).unwrap();
    assert!(
        projects
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["name"] == "coffee"),
        "the created project must be listed"
    );

    let model = write_file(dir.path(), "model.json", &okf_json("coffee").to_string());
    let out = run_mw(
        &[
            "--server",
            &url,
            "commit",
            "coffee",
            "--branch",
            "main",
            "--message",
            "hello over http",
            "--file",
            model.to_str().unwrap(),
        ],
        &[],
    );
    assert!(out.status.success(), "commit failed: {}", stderr(&out));
    let commit: Value = serde_json::from_str(&stdout(&out)).unwrap();
    let hash = commit["hash"].as_str().unwrap().to_string();

    let out = run_mw(
        &["--server", &url, "log", "coffee", "--branch", "main"],
        &[],
    );
    assert!(out.status.success(), "log failed: {}", stderr(&out));
    let commits: Value = serde_json::from_str(&stdout(&out)).unwrap();
    let commits = commits.as_array().unwrap();
    assert_eq!(commits.len(), 1);
    assert_eq!(commits[0]["hash"], hash.as_str());
    assert_eq!(commits[0]["message"], "hello over http");

    serve.abort();
    let _ = serve.await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_token_is_read_from_the_named_environment_variable_and_never_echoed() {
    const VAR: &str = "MW_CLI_TEST_TOKEN";
    let secret = "s3cret-token";

    let dir = tempfile::tempdir().unwrap();
    let router = server::app(app_state(
        dir.path(),
        server::auth::AuthConfig::static_token(secret),
    ));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let serve = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let url = format!("http://{}", addr);

    // The correct token, read from the variable named by --token, is accepted.
    let out = run_mw(
        &["--server", &url, "--token", VAR, "project", "list"],
        &[(VAR, secret)],
    );
    assert!(
        out.status.success(),
        "the right token must work: {}",
        stderr(&out)
    );

    // A wrong token is refused, and the token value never appears in any output.
    let wrong = "super-secret-wrong-token";
    let out = run_mw(
        &["--server", &url, "--token", VAR, "project", "list"],
        &[(VAR, wrong)],
    );
    assert!(!out.status.success(), "a wrong token must be refused");
    let combined = format!("{}{}", stdout(&out), stderr(&out));
    assert!(
        !combined.contains(wrong),
        "a token must never be echoed, got: {}",
        combined
    );

    // An unset variable is a clear error that names the variable, never a secret.
    let out = run_mw_removing(
        &["--server", &url, "--token", VAR, "project", "list"],
        &[],
        &[VAR],
    );
    assert!(!out.status.success());
    let err = stderr(&out);
    assert!(
        err.contains(VAR),
        "the error must name the variable: {}",
        err
    );
    assert!(
        !err.contains(secret),
        "the error must never contain a token: {}",
        err
    );

    serve.abort();
    let _ = serve.await;
}
