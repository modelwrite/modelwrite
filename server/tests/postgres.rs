// SPDX-License-Identifier: AGPL-3.0-or-later
#![cfg(feature = "postgres")]

use postgres::NoTls;
use server::store::postgres::PostgresStore;
use server::store::{AuditEntry, CommitGuard, GateRun, Store, StoreError};

/// Every test in this file is marked `#[ignore = "requires MW_TEST_DATABASE_URL"]`, so a
/// plain `cargo test` reports them as IGNORED (not passed) and they do no work. A developer
/// who has a database runs them with `--include-ignored` after setting MW_TEST_DATABASE_URL;
/// this helper then opens the configured database and panics - not skips - when the URL is
/// unset, empty or unreachable, because a misconfigured URL must fail loudly.
fn require_store() -> PostgresStore {
    let url = std::env::var("MW_TEST_DATABASE_URL").expect(
        "MW_TEST_DATABASE_URL must be set to run the ignored postgres tests (use --include-ignored)",
    );
    if url.trim().is_empty() {
        panic!("MW_TEST_DATABASE_URL is set but empty");
    }
    match PostgresStore::open(&url) {
        Ok(store) => store,
        Err(error) => panic!(
            "could not open the configured PostgreSQL database: {}",
            error
        ),
    }
}

/// A project name unique to this test run, so tests can share one database without
/// colliding, and a re-run starts clean.
fn unique_project() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("pgtest_{}_{}", nanos, n)
}

#[test]
#[ignore = "requires MW_TEST_DATABASE_URL"]
fn blobs_are_content_addressed() {
    let store = require_store();
    let a = store.put_blob(b"model-one").unwrap();
    let b = store.put_blob(b"model-one").unwrap();
    let c = store.put_blob(b"model-two").unwrap();
    assert_eq!(a, b, "identical bytes must share an address");
    assert_ne!(a, c);
    assert_eq!(store.blob(&a).unwrap().unwrap(), b"model-one");
}

#[test]
#[ignore = "requires MW_TEST_DATABASE_URL"]
fn commits_land_on_a_branch_and_move_its_tip() {
    let store = require_store();
    let project = unique_project();
    store.create_project(&project, None).unwrap();
    let first = store
        .commit_model(
            &project,
            "main",
            "okf1",
            "alex",
            "first commit",
            None,
            None,
            None,
        )
        .unwrap();
    assert_eq!(
        store.branch_tip(&project, "main").unwrap().unwrap(),
        first.hash
    );

    let second = store
        .commit_model(
            &project,
            "main",
            "okf2",
            "alex",
            "second commit",
            None,
            None,
            None,
        )
        .unwrap();
    assert_eq!(
        store.branch_tip(&project, "main").unwrap().unwrap(),
        second.hash
    );
    assert_eq!(store.commits_on(&project, "main").unwrap().len(), 2);
    assert!(first.parents.is_empty(), "the first commit has no parent");
    assert_eq!(second.parents, vec![first.hash.clone()]);
}

#[test]
#[ignore = "requires MW_TEST_DATABASE_URL"]
fn duplicate_projects_and_branches_are_conflicts() {
    let store = require_store();
    let project = unique_project();
    store.create_project(&project, None).unwrap();
    match store.create_project(&project, None) {
        Err(StoreError::Conflict(_)) => {}
        other => panic!("expected a conflict, got {:?}", other),
    }
    let commit = store
        .commit_model(
            &project,
            "main",
            "okf1",
            "alex",
            "first commit",
            None,
            None,
            None,
        )
        .unwrap();
    store
        .create_branch(&project, "review", &commit.hash, None)
        .unwrap();
    match store.create_branch(&project, "review", &commit.hash, None) {
        Err(StoreError::Conflict(_)) => {}
        other => panic!("expected a conflict, got {:?}", other),
    }
    match store.create_branch(&project, "other", "missing-commit", None) {
        Err(StoreError::NotFound(_)) => {}
        other => panic!("expected not found, got {:?}", other),
    }
}

#[test]
#[ignore = "requires MW_TEST_DATABASE_URL"]
fn concurrent_commits_to_one_branch_form_a_linear_chain() {
    let store = require_store();
    let project = unique_project();
    store.create_project(&project, None).unwrap();
    let store = std::sync::Arc::new(store);

    let mut handles = Vec::new();
    for i in 0..8 {
        let store = store.clone();
        let project = project.clone();
        handles.push(std::thread::spawn(move || {
            store
                .commit_model(
                    &project,
                    "main",
                    &format!("okf-{}", i),
                    "alex",
                    "concurrent",
                    None,
                    None,
                    None,
                )
                .expect("commit model");
        }));
    }
    for handle in handles {
        handle.join().expect("worker thread");
    }

    let history = store.commits_on(&project, "main").unwrap();
    assert_eq!(history.len(), 8, "every commit must land");

    let mut children: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();
    for commit in &history {
        for parent in &commit.parents {
            children
                .entry(parent.clone())
                .or_default()
                .push(commit.hash.clone());
        }
    }
    for (parent, kids) in &children {
        assert_eq!(
            kids.len(),
            1,
            "commit {} is the parent of {} commits, so the history forked",
            parent,
            kids.len()
        );
    }

    let tip = store.branch_tip(&project, "main").unwrap().unwrap();
    let mut seen: Vec<String> = Vec::new();
    let mut cursor = Some(tip);
    while let Some(hash) = cursor {
        assert!(
            !seen.contains(&hash),
            "the chain revisits {}, so it is not a chain",
            hash
        );
        let commit = history
            .iter()
            .find(|c| c.hash == hash)
            .expect("the tip and its ancestors must all be in the history");
        cursor = commit.parents.first().cloned();
        seen.push(hash);
    }
    assert_eq!(
        seen.len(),
        8,
        "walking back from the tip must reach all eight commits"
    );
}

#[test]
#[ignore = "requires MW_TEST_DATABASE_URL"]
fn a_merge_refuses_when_the_branch_moved_under_it() {
    let store = require_store();
    let project = unique_project();
    store.create_project(&project, None).unwrap();
    let root = store
        .commit_model(
            &project, "main", "okf-root", "alex", "root", None, None, None,
        )
        .unwrap();
    store
        .create_branch(&project, "feature", &root.hash, None)
        .unwrap();
    let their_side = store
        .commit_model(
            &project,
            "feature",
            "okf-feature",
            "alex",
            "feature",
            None,
            None,
            None,
        )
        .unwrap();

    let concurrent = store
        .commit_model(
            &project,
            "main",
            "okf-concurrent",
            "alex",
            "concurrent",
            None,
            None,
            None,
        )
        .unwrap();

    let refused = store.commit_merge(
        &project,
        "main",
        &[root.hash.clone(), their_side.hash.clone()],
        "okf-merged",
        "alex",
        "merge",
        None,
        None,
    );
    match refused {
        Err(StoreError::Conflict(_)) => {}
        other => panic!("expected a conflict, got {:?}", other.map(|c| c.hash)),
    }

    assert_eq!(
        store.branch_tip(&project, "main").unwrap().unwrap(),
        concurrent.hash
    );
    let merges: Vec<_> = store
        .commits_on(&project, "main")
        .unwrap()
        .into_iter()
        .filter(|c| c.parents.len() == 2)
        .collect();
    assert!(merges.is_empty(), "no merge commit may be written");
}

#[test]
#[ignore = "requires MW_TEST_DATABASE_URL"]
fn a_guarded_commit_is_refused_inside_the_transaction() {
    let store = require_store();
    let project = unique_project();
    store.create_project(&project, None).unwrap();
    let root = store
        .commit_model(
            &project, "main", "okf-root", "alex", "root", None, None, None,
        )
        .unwrap();
    let root_hash = root.hash.clone();

    store
        .acquire_locks(
            &project,
            "main",
            &["b1".to_string()],
            "alex",
            600,
            1000,
            None,
        )
        .unwrap();

    let touched = vec!["b1".to_string()];
    let refused = store.commit_model(
        &project,
        "main",
        "okf-sam",
        "sam",
        "sam edits a locked element",
        Some(CommitGuard {
            holder: "sam",
            elements: &touched,
            now: 1000,
            expected_tip: Some(&root_hash),
        }),
        None,
        None,
    );
    match refused {
        Err(StoreError::Locked {
            element, holder, ..
        }) => {
            assert_eq!(element, "b1");
            assert_eq!(holder, "alex");
        }
        other => panic!("expected a conflict, got {:?}", other.map(|c| c.hash)),
    }
    assert_eq!(store.commits_on(&project, "main").unwrap().len(), 1);

    let allowed = store.commit_model(
        &project,
        "main",
        "okf-sam",
        "sam",
        "sam edits after the lease lapsed",
        Some(CommitGuard {
            holder: "sam",
            elements: &touched,
            now: 2000,
            expected_tip: Some(&root_hash),
        }),
        None,
        None,
    );
    assert!(allowed.is_ok(), "an expired lease must not block a commit");

    let elsewhere = vec!["b3".to_string()];
    store
        .acquire_locks(
            &project,
            "main",
            &["b2".to_string()],
            "alex",
            600,
            1000,
            None,
        )
        .unwrap();
    let tip_now = store.branch_tip(&project, "main").unwrap();
    let unrelated = store.commit_model(
        &project,
        "main",
        "okf-other",
        "sam",
        "sam edits something else",
        Some(CommitGuard {
            holder: "sam",
            elements: &elsewhere,
            now: 1000,
            expected_tip: tip_now.as_deref(),
        }),
        None,
        None,
    );
    assert!(unrelated.is_ok(), "only the guarded elements are protected");
}

#[test]
#[ignore = "requires MW_TEST_DATABASE_URL"]
fn a_guard_computed_against_an_old_tip_is_refused() {
    let store = require_store();
    let project = unique_project();
    store.create_project(&project, None).unwrap();
    let root = store
        .commit_model(
            &project, "main", "okf-root", "alex", "root", None, None, None,
        )
        .unwrap();

    store
        .commit_model(
            &project,
            "main",
            "okf-other",
            "alex",
            "moved",
            None,
            None,
            None,
        )
        .unwrap();

    let touched = vec!["b1".to_string()];
    let stale = store.commit_model(
        &project,
        "main",
        "okf-sam",
        "sam",
        "sam commits against a stale view",
        Some(CommitGuard {
            holder: "sam",
            elements: &touched,
            now: 1000,
            expected_tip: Some(&root.hash),
        }),
        None,
        None,
    );
    match stale {
        Err(StoreError::Conflict(message)) => {
            assert!(
                message.contains("moved"),
                "the refusal must say the branch moved: {}",
                message
            );
        }
        other => panic!("expected a conflict, got {:?}", other.map(|c| c.hash)),
    }
    assert_eq!(
        store.commits_on(&project, "main").unwrap().len(),
        2,
        "nothing may be written"
    );
}

#[test]
#[ignore = "requires MW_TEST_DATABASE_URL"]
fn audit_rows_ride_each_mutation_transaction() {
    let store = require_store();
    let project = unique_project();
    let entry = |action: &str| AuditEntry {
        id: 0,
        project: project.clone(),
        at: 1,
        actor: "alex".to_string(),
        mechanism: "jwt".to_string(),
        authorizer: String::new(),
        action: action.to_string(),
        subject: "s".to_string(),
        detail: "d".to_string(),
    };

    store
        .create_project(&project, Some(&entry("project.create")))
        .unwrap();
    let root = store
        .commit_model(
            &project, "main", "okf-root", "alex", "root", None, None, None,
        )
        .unwrap();
    store
        .create_branch(
            &project,
            "feature",
            &root.hash,
            Some(&entry("branch.create")),
        )
        .unwrap();
    store
        .acquire_locks(
            &project,
            "main",
            &["b1".to_string()],
            "alex",
            600,
            1000,
            Some(&entry("lock.acquire")),
        )
        .unwrap();
    let ids: Vec<String> = store
        .locks(&project, 1000)
        .unwrap()
        .into_iter()
        .map(|l| l.id)
        .collect();
    store
        .release_locks(&project, "alex", &ids, Some(&entry("lock.release")))
        .unwrap();
    store
        .record_gate_run(
            &GateRun {
                project: project.clone(),
                branch: "main".to_string(),
                reference_hash: "r".to_string(),
                candidate_hash: "c".to_string(),
                passed: true,
                evidence: "{}".to_string(),
                created_at: "1".to_string(),
            },
            Some(&entry("gate.run")),
        )
        .unwrap();
    store
        .delete_branch(&project, "feature", Some(&entry("branch.delete")))
        .unwrap();

    let actions: Vec<String> = store
        .audit(&project, 1000)
        .unwrap()
        .into_iter()
        .map(|e| e.action)
        .collect();
    for action in [
        "project.create",
        "branch.create",
        "lock.acquire",
        "lock.release",
        "gate.run",
        "branch.delete",
    ] {
        assert!(
            actions.contains(&action.to_string()),
            "missing audit action {}",
            action
        );
    }
}

#[test]
#[ignore = "requires MW_TEST_DATABASE_URL"]
fn the_audit_table_refuses_update_and_delete() {
    let store = require_store();
    let project = unique_project();
    store.create_project(&project, None).unwrap();
    let id = store
        .append_audit(&AuditEntry {
            id: 0,
            project: project.clone(),
            at: 1,
            actor: "a".to_string(),
            mechanism: "open".to_string(),
            authorizer: String::new(),
            action: "x".to_string(),
            subject: "s".to_string(),
            detail: "d".to_string(),
        })
        .unwrap();

    // A second connection, raw SQL, so the trigger is tested against the database rather
    // than against the store's own (deliberately update-free) interface.
    let url = std::env::var("MW_TEST_DATABASE_URL").unwrap();
    let mut conn = postgres::Client::connect(&url, NoTls).expect("connect raw client");
    assert!(
        conn.execute(
            "UPDATE audit SET detail = 'rewritten' WHERE id = $1",
            &[&id]
        )
        .is_err(),
        "UPDATE on the audit table must be refused by the trigger"
    );
    assert!(
        conn.execute("DELETE FROM audit WHERE id = $1", &[&id])
            .is_err(),
        "DELETE on the audit table must be refused by the trigger"
    );
}

#[test]
#[ignore = "requires MW_TEST_DATABASE_URL"]
fn a_mutation_rolls_back_when_its_audit_entry_cannot_be_written() {
    let store = require_store();
    let project = unique_project();
    let unwritable = AuditEntry {
        id: 0,
        project: project.clone(),
        at: 0,
        actor: String::new(),
        mechanism: "open".to_string(),
        authorizer: String::new(),
        action: "project.create".to_string(),
        subject: project.clone(),
        detail: "an entry that names nobody is not a record".to_string(),
    };

    let refused = store.create_project(&project, Some(&unwritable));
    assert!(
        refused.is_err(),
        "an unwritable audit entry must fail the mutation it describes"
    );
    assert!(
        store.project(&project).unwrap().is_none(),
        "the project must NOT survive: the mutation rolled back with its record"
    );
}

#[test]
#[ignore = "requires MW_TEST_DATABASE_URL"]
fn an_expired_lease_stops_blocking() {
    let store = require_store();
    let project = unique_project();

    // A acquires b1 at now = 1000 with ttl 30, so the lease expires at 1030.
    store
        .acquire_locks(
            &project,
            "main",
            &["b1".to_string()],
            "alex",
            30,
            1000,
            None,
        )
        .unwrap();

    match store.acquire_locks(&project, "main", &["b1".to_string()], "bob", 30, 1000, None) {
        Err(StoreError::Locked { holder, .. }) => {
            assert_eq!(holder, "alex", "the refusal must name the holder");
        }
        other => panic!("expected a lock refusal at now=1000, got {:?}", other),
    }

    let acquired = store
        .acquire_locks(&project, "main", &["b1".to_string()], "bob", 30, 1031, None)
        .unwrap();
    assert_eq!(acquired.len(), 1);
    assert_eq!(acquired[0].holder, "bob");
}

#[test]
#[ignore = "requires MW_TEST_DATABASE_URL"]
fn deleting_a_branch_leaves_its_commits_readable() {
    let store = require_store();
    let project = unique_project();
    store.create_project(&project, None).unwrap();
    let commit = store
        .commit_model(&project, "main", "okf", "alex", "commit", None, None, None)
        .unwrap();
    store
        .create_branch(&project, "review", &commit.hash, None)
        .unwrap();

    store.delete_branch(&project, "review", None).unwrap();

    // Deleting a branch removes the pointer, never the commits it pointed at.
    let still = store.commit(&project, &commit.hash).unwrap();
    assert!(still.is_some(), "the commit must survive its branch");
    assert_eq!(still.unwrap().hash, commit.hash);
}

#[test]
#[ignore = "requires MW_TEST_DATABASE_URL"]
fn two_concurrent_acquires_by_different_holders_cannot_both_succeed() {
    let store = require_store();
    let project = unique_project();
    let store = std::sync::Arc::new(store);

    // Two holders race for the same element. The UNIQUE(project, element) constraint is
    // the arbiter: exactly one INSERT ... ON CONFLICT wins, and the loser is refused with
    // the winner's identity rather than also holding the element.
    let handles: Vec<_> = ["alex".to_string(), "bob".to_string()]
        .into_iter()
        .map(|holder| {
            let store = store.clone();
            let project = project.clone();
            std::thread::spawn(move || {
                store.acquire_locks(
                    &project,
                    "main",
                    &["b1".to_string()],
                    &holder,
                    600,
                    1000,
                    None,
                )
            })
        })
        .collect();

    let results: Vec<_> = handles
        .into_iter()
        .map(|handle| handle.join().expect("worker thread"))
        .collect();

    let mut winners = 0;
    let mut refusals: Vec<String> = Vec::new();
    for result in results {
        match result {
            Ok(_) => winners += 1,
            Err(StoreError::Locked {
                element, holder, ..
            }) => {
                assert_eq!(element, "b1", "the refusal must name the element");
                refusals.push(holder);
            }
            Err(other) => panic!("expected a lock refusal, got {:?}", other),
        }
    }
    assert_eq!(winners, 1, "exactly one holder may win the lease");
    assert_eq!(refusals.len(), 1, "the loser must be refused as Locked");

    let locks = store.locks(&project, 1000).unwrap();
    assert_eq!(locks.len(), 1, "exactly one live lock must exist");
    assert_eq!(
        locks[0].holder, refusals[0],
        "the refusal must name the holder that actually won"
    );
}
#[test]
#[ignore = "requires MW_TEST_DATABASE_URL"]
fn a_guarded_commit_racing_an_acquire_cannot_slip_through() {
    let store = require_store();
    let project = unique_project();
    store.create_project(&project, None).unwrap();

    // The other side of the race is simulated with a raw connection that holds the SAME
    // per-element advisory lock the store takes, inserts a live lease, and commits only at
    // the end â€” so the lease becomes visible exactly when the advisory lock is released,
    // which is the interleaving the bug lived in.
    let url = std::env::var("MW_TEST_DATABASE_URL").unwrap();
    let mut conn = postgres::Client::connect(&url, NoTls).expect("connect raw client");
    let mut acquire = conn.transaction().unwrap();
    acquire
        .execute(
            "SELECT pg_advisory_xact_lock($1)",
            &[&server::store::postgres::element_lock_key(&project, "b1")],
        )
        .unwrap();
    acquire
        .execute(
            "INSERT INTO locks (id, project, branch, element, holder, acquired_at, expires_at) VALUES ($1, $2, $3, $4, $5, $6, $7)",
            &[&"raw".to_string(), &project, &"main", &"b1", &"alex", &1000i64, &1600i64],
        )
        .unwrap();

    let store = std::sync::Arc::new(store);
    let touched = vec!["b1".to_string()];
    let handle = {
        let store = store.clone();
        let project = project.clone();
        std::thread::spawn(move || {
            store.commit_model(
                &project,
                "main",
                "okf-sam",
                "sam",
                "sam edits a locked element",
                Some(CommitGuard {
                    holder: "sam",
                    elements: &touched,
                    now: 1000,
                    expected_tip: None,
                }),
                None,
                None,
            )
        })
    };

    // The guarded commit must WAIT on the advisory lock, not check past the uncommitted
    // lease and land after it.
    std::thread::sleep(std::time::Duration::from_millis(200));
    assert!(
        !handle.is_finished(),
        "a guarded commit must wait for an in-progress acquire, not slip past it"
    );

    // The acquire lands: the lease is live, and the blocked guard must now see it.
    acquire.commit().unwrap();

    let raced = handle.join().expect("worker thread");
    match raced {
        Err(StoreError::Locked {
            element, holder, ..
        }) => {
            assert_eq!(element, "b1", "the refusal must name the element");
            assert_eq!(holder, "alex", "the refusal must name the holder");
        }
        other => panic!(
            "a commit racing an acquire must be refused, got {:?}",
            other.map(|c| c.hash)
        ),
    }
    assert_eq!(
        store.commits_on(&project, "main").unwrap().len(),
        0,
        "the refused commit must leave no commit behind"
    );
}

#[test]
#[ignore = "requires MW_TEST_DATABASE_URL"]
fn acquiring_waits_for_an_in_progress_lease_check() {
    let store = require_store();
    let project = unique_project();
    store.create_project(&project, None).unwrap();

    // Hold the per-element advisory lock open with a raw connection, simulating a guarded
    // commit whose enforce_guard check is in flight. An acquire must wait on it, not race it.
    let url = std::env::var("MW_TEST_DATABASE_URL").unwrap();
    let mut conn = postgres::Client::connect(&url, NoTls).expect("connect raw client");
    let mut checking = conn.transaction().unwrap();
    checking
        .execute(
            "SELECT pg_advisory_xact_lock($1)",
            &[&server::store::postgres::element_lock_key(&project, "b1")],
        )
        .unwrap();

    let store = std::sync::Arc::new(store);
    let handle = {
        let store = store.clone();
        let project = project.clone();
        std::thread::spawn(move || {
            store.acquire_locks(
                &project,
                "main",
                &["b1".to_string()],
                "alex",
                600,
                1000,
                None,
            )
        })
    };

    std::thread::sleep(std::time::Duration::from_millis(200));
    assert!(
        !handle.is_finished(),
        "an acquire must wait for an in-progress lease check, not race it"
    );

    checking.commit().unwrap();

    let acquired = handle.join().expect("worker thread").unwrap();
    assert_eq!(acquired.len(), 1);
    assert_eq!(acquired[0].element, "b1");
}

/// Derive a connection URL for a different database on the same server, preserving the host,
/// credentials and query parameters (sslmode in particular).
fn database_url_for(base: &str, database: &str) -> String {
    let (authority, query) = match base.split_once('?') {
        Some((rest, query)) => (rest, query),
        None => (base, ""),
    };
    let (scheme_and_host, _old_db) = authority
        .rsplit_once('/')
        .expect("url must name a database");
    if query.is_empty() {
        format!("{}/{}", scheme_and_host, database)
    } else {
        format!("{}/{}?{}", scheme_and_host, database, query)
    }
}

#[test]
#[ignore = "requires MW_TEST_DATABASE_URL"]
fn an_older_database_gains_the_mechanism_column_instead_of_failing() {
    let url = std::env::var("MW_TEST_DATABASE_URL").unwrap();
    let database = unique_project();
    let project = unique_project();

    // A scratch database with the OLD audit shape: the table exists, but without the
    // mechanism column. CREATE TABLE IF NOT EXISTS cannot add it, so only the migration can.
    {
        let mut conn = postgres::Client::connect(&url, NoTls).expect("connect raw client");
        conn.batch_execute(&format!("CREATE DATABASE \"{}\"", database))
            .expect("create scratch database");
    }
    let scratch = database_url_for(&url, &database);
    {
        let mut conn = postgres::Client::connect(&scratch, NoTls).expect("connect scratch");
        conn.batch_execute(
            "CREATE TABLE audit (id BIGSERIAL PRIMARY KEY, project TEXT NOT NULL, at BIGINT NOT NULL, actor TEXT NOT NULL, action TEXT NOT NULL, subject TEXT NOT NULL, detail TEXT NOT NULL)",
        )
        .expect("create old-shape audit table");
    }

    // Opening the store must bring the old database up to date, not fail.
    let store = PostgresStore::open(&scratch).expect("an older database must open, not fail");
    store.create_project(&project, None).unwrap();
    store
        .append_audit(&AuditEntry {
            id: 0,
            project: project.clone(),
            at: 1,
            actor: "alex".to_string(),
            mechanism: "static".to_string(),
            authorizer: String::new(),
            action: "commit.create".to_string(),
            subject: "main".to_string(),
            detail: "after the migration".to_string(),
        })
        .expect("the migrated table must accept a write");
    let rows = store.audit(&project, 10).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].mechanism, "static");

    drop(store);

    {
        let mut conn = postgres::Client::connect(&url, NoTls).expect("connect raw client");
        conn.batch_execute(&format!(
            "DROP DATABASE IF EXISTS \"{}\" WITH (FORCE)",
            database
        ))
        .expect("drop scratch database");
    }
}
