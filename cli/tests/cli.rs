// SPDX-License-Identifier: AGPL-3.0-or-later
//! Integration tests for the mw command line: argument parsing, exit codes, the offline
//! commit/log round trip, and the HTTP path driven against an in-process router bound to
//! an ephemeral port.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::Arc;

use serde_json::Value;

use server::store::Store;

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

/// Run mw with the given args and assert success, returning the parsed JSON output (or
/// Value::Null for commands that print nothing).
fn run_ok(args: &[&str]) -> Value {
    let out = run_mw(args, &[]);
    assert!(out.status.success(), "mw failed: {}", stderr(&out));
    let text = stdout(&out);
    if text.trim().is_empty() {
        return Value::Null;
    }
    serde_json::from_str(&text).expect("mw stdout was not JSON")
}

/// A valid OKF document whose structure (and matching graph nodes) are the given blocks.
fn okf_with_blocks(project: &str, blocks: &[(&str, &str)]) -> Value {
    let structure: Vec<Value> = blocks
        .iter()
        .map(|(id, name)| serde_json::json!({ "id": id, "name": name, "kind": "block" }))
        .collect();
    let nodes: Vec<Value> = blocks
        .iter()
        .map(|(id, _)| serde_json::json!({ "id": id, "kind": "block" }))
        .collect();
    serde_json::json!({
        "okf": "1.0",
        "project": project,
        "summary": {
            "blocks": structure.len(),
            "requirements": 0,
            "interfaces": 0,
            "signals": 0,
            "activities": 0,
            "graphNodes": nodes.len(),
            "graphEdges": 0
        },
        "stateMachine": { "name": "sm", "regions": [] },
        "structure": structure,
        "graph": { "nodes": nodes, "edges": [] }
    })
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
fn offline_commit_derives_a_true_summary() {
    // The offline CLI reaches the store directly, bypassing the HTTP commit core, so this is
    // the convergence test the shared-core refactor exists for: a document whose summary
    // under-counts its graph must still be stored with a true summary, because the store's
    // commit path derives it. Before the derivation moved into the store, this committed the
    // false self-count verbatim.
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("mw.db");
    let db = db.to_str().unwrap();

    run_ok(&["--db", db, "project", "create", "coffee"]);
    let mut model = okf_json("coffee");
    // One graph node, but the summary claims zero: a stale, false self-count.
    model["summary"] = serde_json::json!({ "blocks": 0, "requirements": 0, "interfaces": 0, "signals": 0, "activities": 0, "graphNodes": 0, "graphEdges": 0 });
    let path = write_file(dir.path(), "stale.json", &model.to_string());
    run_ok(&[
        "--db",
        db,
        "commit",
        "coffee",
        "--branch",
        "main",
        "--message",
        "stale summary",
        "--file",
        path.to_str().unwrap(),
    ]);

    let store = server::store::sqlite::SqliteStore::open(&dir.path().join("mw.db")).unwrap();
    let tip = store.branch_tip("coffee", "main").unwrap().unwrap();
    let commit = store.commit("coffee", &tip).unwrap().unwrap();
    let blob = store.blob(&commit.okf_hash).unwrap().unwrap();
    let stored: Value = serde_json::from_slice(&blob).unwrap();
    assert_eq!(
        stored["summary"]["graphNodes"], 1,
        "the offline commit must store a summary that counts its one graph node"
    );
    assert_eq!(stored["summary"]["blocks"], 0);
}

#[test]
fn offline_artifact_fetches_the_retained_source_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("mw.db");
    let db = db.to_str().unwrap();

    // Seed the store directly: an import record whose retained artifact is the source bytes.
    // The offline CLI has no import command, so the store is prepared through its own API.
    let artifact_hash = {
        let store = server::store::sqlite::SqliteStore::open(&dir.path().join("mw.db")).unwrap();
        store.create_project("coffee", None).unwrap();
        let hash = store.put_blob(b"source xmi bytes").unwrap();
        store
            .record_import("coffee", &hash, "sysml-v1-xmi", "2.4", "{}", "{}")
            .unwrap();
        hash
    };

    let out = run_mw(
        &["--db", db, "artifact", "coffee", "--hash", &artifact_hash],
        &[],
    );
    assert!(
        out.status.success(),
        "artifact fetch failed: {}",
        stderr(&out)
    );
    let body: Value = serde_json::from_str(&stdout(&out)).unwrap();
    assert_eq!(body["artifactHash"], artifact_hash.as_str());
    assert_eq!(body["artifact"], "source xmi bytes");
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

#[test]
fn offline_commit_of_an_invalid_document_is_refused_with_the_validators_errors() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("mw.db");
    let db = db.to_str().unwrap();

    let valid = write_file(
        dir.path(),
        "valid.json",
        &okf_with_blocks("coffee", &[("block1", "Block One")]).to_string(),
    );
    run_ok(&["--db", db, "project", "create", "coffee"]);
    run_ok(&[
        "--db",
        db,
        "commit",
        "coffee",
        "--branch",
        "main",
        "--message",
        "base",
        "--file",
        valid.to_str().unwrap(),
    ]);

    // Valid JSON, but the OKF validator must refuse it: the graph section is missing.
    let invalid = serde_json::json!({
        "okf": "1.0",
        "project": "coffee",
        "summary": { "blocks": 1, "requirements": 0, "interfaces": 0, "signals": 0, "activities": 0, "graphNodes": 0, "graphEdges": 0 },
        "stateMachine": { "name": "sm", "regions": [] },
        "structure": [ { "id": "block1", "name": "Block One", "kind": "block" } ]
    });
    let invalid = write_file(dir.path(), "invalid.json", &invalid.to_string());

    let out = run_mw(
        &[
            "--db",
            db,
            "commit",
            "coffee",
            "--branch",
            "main",
            "--message",
            "bad",
            "--file",
            invalid.to_str().unwrap(),
        ],
        &[],
    );
    assert!(!out.status.success(), "an invalid document must be refused");
    let err = stderr(&out);
    assert!(
        err.contains("the model failed validation"),
        "stderr was: {}",
        err
    );
    assert!(
        err.contains("graph section is missing"),
        "the validator's error must be reported, stderr was: {}",
        err
    );

    // The refused commit must not have moved the branch tip.
    let log = run_ok(&["--db", db, "log", "coffee", "--branch", "main"]);
    assert_eq!(log.as_array().unwrap().len(), 1);
}

#[test]
fn offline_commit_changing_a_locked_element_is_refused_naming_the_holder() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("mw.db");
    let db = db.to_str().unwrap();

    let base = write_file(
        dir.path(),
        "base.json",
        &okf_with_blocks("coffee", &[("block1", "Block One")]).to_string(),
    );
    run_ok(&["--db", db, "project", "create", "coffee"]);
    run_ok(&[
        "--db",
        db,
        "commit",
        "coffee",
        "--branch",
        "main",
        "--message",
        "base",
        "--file",
        base.to_str().unwrap(),
    ]);

    // Alice holds a live lease on block1.
    run_ok(&[
        "--db",
        db,
        "lock",
        "acquire",
        "coffee",
        "--branch",
        "main",
        "--elements",
        "block1",
        "--holder",
        "alice",
        "--ttl",
        "300",
    ]);

    // Bob changes block1, so his commit must be refused for Alice's lease.
    let changed = write_file(
        dir.path(),
        "changed.json",
        &okf_with_blocks("coffee", &[("block1", "Block One Renamed")]).to_string(),
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
            "change",
            "--file",
            changed.to_str().unwrap(),
            "--holder",
            "bob",
        ],
        &[],
    );
    assert!(
        !out.status.success(),
        "a commit changing a locked element must be refused"
    );
    let err = stderr(&out);
    assert!(
        err.contains("alice"),
        "must name the holder, stderr was: {}",
        err
    );
    assert!(
        err.contains("block1"),
        "must name the element, stderr was: {}",
        err
    );
    assert!(err.contains("locked"), "stderr was: {}", err);

    // The refused commit must not have moved the branch tip.
    let log = run_ok(&["--db", db, "log", "coffee", "--branch", "main"]);
    assert_eq!(log.as_array().unwrap().len(), 1);
}

#[test]
fn offline_merge_writes_a_two_parent_commit_and_a_conflict_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("mw.db");
    let db = db.to_str().unwrap();

    run_ok(&["--db", db, "project", "create", "coffee"]);

    let base = write_file(
        dir.path(),
        "base.json",
        &okf_with_blocks("coffee", &[("a", "A"), ("b", "B")]).to_string(),
    );
    let c1 = run_ok(&[
        "--db",
        db,
        "commit",
        "coffee",
        "--branch",
        "main",
        "--message",
        "base",
        "--file",
        base.to_str().unwrap(),
    ]);
    let c1 = c1["hash"].as_str().unwrap().to_string();

    run_ok(&[
        "--db", db, "branch", "create", "coffee", "--name", "feature", "--from", &c1,
    ]);

    // main changes a, feature changes b: a clean, non-conflicting divergence.
    let main_change = write_file(
        dir.path(),
        "main.json",
        &okf_with_blocks("coffee", &[("a", "A2"), ("b", "B")]).to_string(),
    );
    let c2 = run_ok(&[
        "--db",
        db,
        "commit",
        "coffee",
        "--branch",
        "main",
        "--message",
        "main change",
        "--file",
        main_change.to_str().unwrap(),
    ]);
    let c2 = c2["hash"].as_str().unwrap().to_string();

    let feature_change = write_file(
        dir.path(),
        "feature.json",
        &okf_with_blocks("coffee", &[("a", "A"), ("b", "B2")]).to_string(),
    );
    let c3 = run_ok(&[
        "--db",
        db,
        "commit",
        "coffee",
        "--branch",
        "feature",
        "--message",
        "feature change",
        "--file",
        feature_change.to_str().unwrap(),
    ]);
    let c3 = c3["hash"].as_str().unwrap().to_string();

    let merged = run_ok(&[
        "--db",
        db,
        "merge",
        "coffee",
        "--branch",
        "main",
        "--other",
        "feature",
        "--message",
        "merge feature",
    ]);
    let parents: Vec<String> = merged["commit"]["parents"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p.as_str().unwrap().to_string())
        .collect();
    assert_eq!(
        parents,
        vec![c2.clone(), c3.clone()],
        "a merge is a two-parent commit"
    );

    // Now a genuine conflict: both branches change the same element differently.
    let merged_hash = merged["commit"]["hash"].as_str().unwrap().to_string();
    run_ok(&[
        "--db",
        db,
        "branch",
        "create",
        "coffee",
        "--name",
        "fa",
        "--from",
        &merged_hash,
    ]);
    run_ok(&[
        "--db",
        db,
        "branch",
        "create",
        "coffee",
        "--name",
        "fb",
        "--from",
        &merged_hash,
    ]);

    let fa_change = write_file(
        dir.path(),
        "fa.json",
        &okf_with_blocks("coffee", &[("a", "X"), ("b", "B2")]).to_string(),
    );
    let fa_tip = run_ok(&[
        "--db",
        db,
        "commit",
        "coffee",
        "--branch",
        "fa",
        "--message",
        "fa change",
        "--file",
        fa_change.to_str().unwrap(),
    ]);
    let fa_tip = fa_tip["hash"].as_str().unwrap().to_string();

    let fb_change = write_file(
        dir.path(),
        "fb.json",
        &okf_with_blocks("coffee", &[("a", "Y"), ("b", "B2")]).to_string(),
    );
    run_ok(&[
        "--db",
        db,
        "commit",
        "coffee",
        "--branch",
        "fb",
        "--message",
        "fb change",
        "--file",
        fb_change.to_str().unwrap(),
    ]);

    let out = run_mw(
        &[
            "--db",
            db,
            "merge",
            "coffee",
            "--branch",
            "fa",
            "--other",
            "fb",
            "--message",
            "conflicting merge",
        ],
        &[],
    );
    assert!(!out.status.success(), "a conflicting merge must be refused");
    assert!(
        stderr(&out).contains("merge conflict"),
        "stderr was: {}",
        stderr(&out)
    );

    // Nothing was written: fa's tip is unchanged.
    let branches = run_ok(&["--db", db, "branch", "list", "coffee"]);
    let fa_tip_after: String = branches
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["name"] == "fa")
        .unwrap()["tip"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(
        fa_tip_after, fa_tip,
        "a refused merge must not move the tip"
    );
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn offline_gate_matches_the_server_verdict_on_the_corpus_pair() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("mw.db");
    let db = db.to_str().unwrap();

    // Commit the corpus pair offline: the expected model on main, the broken copy on a
    // branch that descends from it, exactly as the server's corpus test does.
    run_ok(&["--db", db, "project", "create", "coffee"]);
    let expected = write_file(
        dir.path(),
        "expected.json",
        &test_support::load_okf_expected(),
    );
    let broken = write_file(dir.path(), "broken.json", &test_support::load_okf_broken());

    let expected_commit = run_ok(&[
        "--db",
        db,
        "commit",
        "coffee",
        "--branch",
        "main",
        "--message",
        "expected",
        "--file",
        expected.to_str().unwrap(),
    ]);
    let expected_hash = expected_commit["hash"].as_str().unwrap().to_string();
    run_ok(&[
        "--db",
        db,
        "branch",
        "create",
        "coffee",
        "--name",
        "corrupted",
        "--from",
        &expected_hash,
    ]);
    let broken_commit = run_ok(&[
        "--db",
        db,
        "commit",
        "coffee",
        "--branch",
        "corrupted",
        "--message",
        "broken",
        "--file",
        broken.to_str().unwrap(),
    ]);
    let broken_hash = broken_commit["hash"].as_str().unwrap().to_string();

    let out = run_mw(
        &[
            "--db",
            db,
            "gate",
            "coffee",
            "--reference",
            &expected_hash,
            "--candidate",
            &broken_hash,
        ],
        &[],
    );
    assert!(
        out.status.success(),
        "offline gate failed: {}",
        stderr(&out)
    );
    let offline_evidence: Value = serde_json::from_str(&stdout(&out)).unwrap();

    // The server opens the SAME store file, so it sees the same commits and must return
    // the same evidence for the same pair.
    let router = server::app(app_state(dir.path(), server::auth::AuthConfig::Open));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let serve = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let url = format!("http://{}", addr);

    let out = run_mw(
        &[
            "--server",
            &url,
            "gate",
            "coffee",
            "--reference",
            &expected_hash,
            "--candidate",
            &broken_hash,
        ],
        &[],
    );
    assert!(out.status.success(), "server gate failed: {}", stderr(&out));
    let server_evidence: Value = serde_json::from_str(&stdout(&out)).unwrap();

    serve.abort();
    let _ = serve.await;

    assert_eq!(
        offline_evidence["passed"], false,
        "the broken pair must fail"
    );
    assert_eq!(
        server_evidence, offline_evidence,
        "offline and server must return the same verdict and evidence"
    );
}

#[test]
fn offline_lock_acquire_deduplicates_elements_and_requires_the_project() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("mw.db");
    let db = db.to_str().unwrap();

    // Acquiring against a project that does not exist must fail, not write an orphaned row.
    let out = run_mw(
        &[
            "--db",
            db,
            "lock",
            "acquire",
            "ghost",
            "--branch",
            "main",
            "--elements",
            "a",
            "--holder",
            "alex",
            "--ttl",
            "300",
        ],
        &[],
    );
    assert!(
        !out.status.success(),
        "a lock on a missing project must fail"
    );
    assert!(
        stderr(&out).contains("not found"),
        "must say the project is not found, got: {}",
        stderr(&out)
    );

    // Releasing against a missing project must also fail.
    let out = run_mw(
        &[
            "--db", db, "lock", "release", "ghost", "--holder", "alex", "--ids", "x",
        ],
        &[],
    );
    assert!(
        !out.status.success(),
        "a release on a missing project must fail"
    );

    run_ok(&["--db", db, "project", "create", "coffee"]);

    // --elements a,a must yield ONE lease, not two.
    let acquired = run_ok(&[
        "--db",
        db,
        "lock",
        "acquire",
        "coffee",
        "--branch",
        "main",
        "--elements",
        "a,a",
        "--holder",
        "alex",
        "--ttl",
        "300",
    ]);
    assert_eq!(
        acquired.as_array().unwrap().len(),
        1,
        "a repeated element must yield one lease"
    );
}
// ---------------------------------------------------------------------------
// Cross-path content addressing: one document, one hash, every write path.
// ---------------------------------------------------------------------------

/// A small document the XMI binding round-trips losslessly (the exact shape its own
/// round-trip harness proves) with a CORRECT derived summary, so the summary re-derivation
/// is a no-op and what this test measures is the SERIALISATION ORDER of each path.
fn cross_path_document() -> okf::types::OkfRoot {
    use okf::types::{
        Attribute, Element, Graph, GraphEdge, GraphNode, OkfRoot, StateMachine, Summary,
    };
    OkfRoot {
        okf: "1.0".to_string(),
        project: "Grinder".to_string(),
        exported_at: String::new(),
        summary: Summary {
            blocks: 2,
            requirements: 0,
            interfaces: 0,
            signals: 0,
            activities: 0,
            graph_nodes: 2,
            graph_edges: 1,
        },
        structure: vec![
            Element {
                id: "block-grinder".to_string(),
                name: "Grinder".to_string(),
                kind: "block".to_string(),
                stereotypes: vec!["Block".to_string()],
                attributes: vec![
                    Attribute {
                        name: "motor".to_string(),
                        attr_type: "Motor".to_string(),
                        aggregation: "composite".to_string(),
                        default: String::new(),
                    },
                    Attribute {
                        name: "capacity".to_string(),
                        attr_type: "Integer".to_string(),
                        aggregation: "none".to_string(),
                        default: "1".to_string(),
                    },
                ],
                documentation: "Grinds coffee beans.".to_string(),
            },
            Element {
                id: "block-motor".to_string(),
                name: "Motor".to_string(),
                kind: "block".to_string(),
                stereotypes: vec!["Block".to_string()],
                attributes: Vec::new(),
                documentation: String::new(),
            },
        ],
        interfaces: Vec::new(),
        signals: Vec::new(),
        requirements: Vec::new(),
        state_machine: Some(StateMachine {
            name: "stateMachine".to_string(),
            regions: Vec::new(),
        }),
        activities: Vec::new(),
        graph: Some(Graph {
            nodes: vec![
                GraphNode {
                    id: "block-grinder".to_string(),
                    kind: "block".to_string(),
                    name: "Grinder".to_string(),
                    stereotypes: vec!["Block".to_string()],
                },
                GraphNode {
                    id: "block-motor".to_string(),
                    kind: "block".to_string(),
                    name: "Motor".to_string(),
                    stereotypes: vec!["Block".to_string()],
                },
            ],
            edges: vec![GraphEdge {
                source: "block-grinder".to_string(),
                target: "block-motor".to_string(),
                kind: "dependency".to_string(),
                label: "Satisfy".to_string(),
            }],
        }),
        provenance: None,
        references: Vec::new(),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_same_document_commits_to_one_hash_through_every_path() {
    let root = cross_path_document();
    let expected = okf::hash::canonical_hash(&root);

    // Path 1: the store commit — put_blob of struct-order bytes + commit_model.
    let store_dir = tempfile::tempdir().unwrap();
    let store = server::store::sqlite::SqliteStore::open(&store_dir.path().join("mw.db")).unwrap();
    store.create_project("cross-path", None).unwrap();
    let store_hash = {
        let bytes = serde_json::to_vec(&root).unwrap();
        let okf_hash = store.put_blob(&bytes).unwrap();
        store
            .commit_model(
                "cross-path",
                "main",
                &okf_hash,
                "alex",
                "store",
                None,
                None,
                None,
            )
            .unwrap()
            .okf_hash
    };

    // Path 2: the capture/replay path — the HTTP JSON commit endpoint, which takes the
    // document as a serde_json::Value and re-serialises it.
    let http_dir = tempfile::tempdir().unwrap();
    let router = server::app(server::AppState {
        store: Arc::new(
            server::store::sqlite::SqliteStore::open(&http_dir.path().join("mw.db")).unwrap(),
        ),
        evidence_dir: http_dir.path().to_path_buf(),
        auth: server::auth::AuthConfig::Open,
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let serve = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let url = format!("http://{}", addr);
    let http_file = write_file(
        http_dir.path(),
        "http.json",
        &serde_json::to_string(&root).unwrap(),
    );
    run_ok(&["--server", &url, "project", "create", "cross-path"]);
    let http_commit = run_ok(&[
        "--server",
        &url,
        "commit",
        "cross-path",
        "--branch",
        "main",
        "--message",
        "http",
        "--file",
        http_file.to_str().unwrap(),
    ]);
    let http_hash = http_commit["okfHash"].as_str().unwrap().to_string();
    serve.abort();
    let _ = serve.await;

    // Path 3: the offline CLI — commits the document read from a file.
    let offline_dir = tempfile::tempdir().unwrap();
    let offline_db = offline_dir.path().join("mw.db");
    let offline_db = offline_db.to_str().unwrap();
    let offline_file = write_file(
        offline_dir.path(),
        "offline.json",
        &serde_json::to_string(&root).unwrap(),
    );
    run_ok(&["--db", offline_db, "project", "create", "cross-path"]);
    let offline_commit = run_ok(&[
        "--db",
        offline_db,
        "commit",
        "cross-path",
        "--branch",
        "main",
        "--message",
        "offline",
        "--file",
        offline_file.to_str().unwrap(),
    ]);
    let offline_hash = offline_commit["okfHash"].as_str().unwrap().to_string();

    // Path 4: a round trip through the binding — export to XMI and import back, then
    // commit the re-imported document. The round trip is lossless, so the re-imported
    // document is the same content and must hash identically.
    let binding = server::binding_registry::resolve("sysml-v1-xmi", "2.4").unwrap();
    let xmi = binding.export(&root).expect("the document must export");
    let (round_tripped, _) = binding.import(&xmi).expect("the export must re-import");
    assert!(
        okf::diff::diff(&root, &round_tripped).equal,
        "the binding must round-trip the document losslessly"
    );
    let binding_dir = tempfile::tempdir().unwrap();
    let binding_store =
        server::store::sqlite::SqliteStore::open(&binding_dir.path().join("mw.db")).unwrap();
    binding_store.create_project("cross-path", None).unwrap();
    let binding_hash = {
        let bytes = serde_json::to_vec(&round_tripped).unwrap();
        let okf_hash = binding_store.put_blob(&bytes).unwrap();
        binding_store
            .commit_model(
                "cross-path",
                "main",
                &okf_hash,
                "alex",
                "binding",
                None,
                None,
                None,
            )
            .unwrap()
            .okf_hash
    };

    assert_eq!(store_hash, expected, "the store commit drifted");
    assert_eq!(
        http_hash, expected,
        "the capture/replay (HTTP) commit drifted"
    );
    assert_eq!(offline_hash, expected, "the offline CLI commit drifted");
    assert_eq!(
        binding_hash, expected,
        "the binding round-trip commit drifted"
    );
    assert_eq!(http_hash, store_hash);
    assert_eq!(offline_hash, store_hash);
    assert_eq!(binding_hash, store_hash);
}
