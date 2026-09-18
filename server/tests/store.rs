// SPDX-License-Identifier: AGPL-3.0-or-later
use server::store::{commit_hash, sqlite::SqliteStore, AuditEntry, GateRun, Store, StoreError};

fn store() -> (SqliteStore, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::open(&dir.path().join("mw.db")).unwrap();
    (store, dir)
}

#[test]
fn blobs_are_content_addressed() {
    let (store, _dir) = store();
    let a = store.put_blob(b"{\"model\":1}").unwrap();
    let b = store.put_blob(b"{\"model\":1}").unwrap();
    let c = store.put_blob(b"{\"model\":2}").unwrap();
    assert_eq!(a, b, "identical bytes must share an address");
    assert_ne!(a, c);
    assert_eq!(store.blob(&a).unwrap().unwrap(), b"{\"model\":1}");
}

#[test]
fn a_commit_hash_ignores_time_but_not_content() {
    let parents = vec!["p1".to_string()];
    let a = commit_hash("p", "main", &parents, "okf1", "alex", "m");
    let b = commit_hash("p", "main", &parents, "okf1", "alex", "m");
    let c = commit_hash("p", "main", &parents, "okf2", "alex", "m");
    assert_eq!(a, b);
    assert_ne!(a, c);
}

#[test]
fn commits_land_on_a_branch_and_move_its_tip() {
    let (store, _dir) = store();
    store.create_project("coffee", None).unwrap();
    let first = store
        .commit_model(
            "coffee",
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
        store.branch_tip("coffee", "main").unwrap().unwrap(),
        first.hash
    );

    let second = store
        .commit_model(
            "coffee",
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
        store.branch_tip("coffee", "main").unwrap().unwrap(),
        second.hash
    );
    assert_eq!(store.commits_on("coffee", "main").unwrap().len(), 2);
    // The parent link is the point of a commit graph: without it, history is a pile of
    // unrelated snapshots that no merge could ever reason about.
    assert!(first.parents.is_empty(), "the first commit has no parent");
    assert_eq!(second.parents, vec![first.hash.clone()]);
}

#[test]
fn duplicate_projects_and_branches_are_conflicts() {
    let (store, _dir) = store();
    store.create_project("coffee", None).unwrap();
    match store.create_project("coffee", None) {
        Err(StoreError::Conflict(_)) => {}
        other => panic!("expected a conflict, got {:?}", other),
    }
    let commit = store
        .commit_model(
            "coffee",
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
        .create_branch("coffee", "review", &commit.hash, None)
        .unwrap();
    match store.create_branch("coffee", "review", &commit.hash, None) {
        Err(StoreError::Conflict(_)) => {}
        other => panic!("expected a conflict, got {:?}", other),
    }
    match store.create_branch("coffee", "other", "missing-commit", None) {
        Err(StoreError::NotFound(_)) => {}
        other => panic!("expected not found, got {:?}", other),
    }
}
#[test]
fn concurrent_commits_to_one_branch_form_a_linear_chain() {
    // The store resolves the parents, hashes and appends inside one lock and transaction.
    // If that atomicity were lost, two concurrent commits would read the same tip and
    // become SIBLINGS: both land, one orphans the other. Counting roots would not see
    // that - a sibling fork still has one root - so linearity is checked the only way
    // that catches it: by counting each commit's children and by walking the parents
    // from the tip back to the root.
    let (store, _dir) = store();
    store.create_project("coffee", None).unwrap();
    let store = std::sync::Arc::new(store);

    let mut handles = Vec::new();
    for i in 0..8 {
        let store = store.clone();
        handles.push(std::thread::spawn(move || {
            store
                .commit_model(
                    "coffee",
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

    let history = store.commits_on("coffee", "main").unwrap();
    assert_eq!(history.len(), 8, "every commit must land");

    // A parent with two children is a fork, and this is the assertion that catches it.
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

    // Walking the parents from the branch tip must reach every commit exactly once.
    let tip = store.branch_tip("coffee", "main").unwrap().unwrap();
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
fn a_merge_refuses_when_the_branch_moved_under_it() {
    // A merge is computed from tips read before the write. If a commit lands in between,
    // writing the merge would move the branch off that commit and orphan it: stored, but
    // unreachable from any branch. The store must refuse instead of losing it.
    let (store, _dir) = store();
    store.create_project("coffee", None).unwrap();
    let root = store
        .commit_model(
            "coffee", "main", "okf-root", "alex", "root", None, None, None,
        )
        .unwrap();
    store
        .create_branch("coffee", "feature", &root.hash, None)
        .unwrap();
    let their_side = store
        .commit_model(
            "coffee",
            "feature",
            "okf-feature",
            "alex",
            "feature",
            None,
            None,
            None,
        )
        .unwrap();

    // The concurrent commit: main moves AFTER the merge read its tips.
    let concurrent = store
        .commit_model(
            "coffee",
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
        "coffee",
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

    // Nothing moved, and the concurrent commit is still the tip.
    assert_eq!(
        store.branch_tip("coffee", "main").unwrap().unwrap(),
        concurrent.hash
    );
    let merges: Vec<_> = store
        .commits_on("coffee", "main")
        .unwrap()
        .into_iter()
        .filter(|c| c.parents.len() == 2)
        .collect();
    assert!(merges.is_empty(), "no merge commit may be written");
}
#[test]
fn a_guarded_commit_is_refused_inside_the_transaction() {
    // The guard has to be enforced by the store, not by the caller. A check performed
    // before the write leaves a window in which another holder takes the lock and the
    // guarded commit lands anyway - a lock that looks enforced and is not. This test drives
    // the store directly, with no API layer to do the checking for it.
    use server::store::CommitGuard;

    let (store, _dir) = store();
    store.create_project("coffee", None).unwrap();
    let root = store
        .commit_model(
            "coffee", "main", "okf-root", "alex", "root", None, None, None,
        )
        .unwrap();
    let root_hash = root.hash.clone();

    // Alex holds b1.
    store
        .acquire_locks(
            "coffee",
            "main",
            &["b1".to_string()],
            "alex",
            600,
            1000,
            None,
        )
        .unwrap();

    // Sam's commit touches b1: the store must refuse it, with nothing written.
    let touched = vec!["b1".to_string()];
    let refused = store.commit_model(
        "coffee",
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
            assert_eq!(element, "b1", "the refusal must name the element");
            assert_eq!(holder, "alex", "the refusal must name the holder");
        }
        other => panic!("expected a conflict, got {:?}", other.map(|c| c.hash)),
    }
    assert_eq!(store.commits_on("coffee", "main").unwrap().len(), 1);

    // An expired lease is not a lock: the same commit succeeds once the lease lapses.
    let allowed = store.commit_model(
        "coffee",
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

    // A guard for an element nobody holds blocks nothing, even while a DIFFERENT element is
    // locked: a lock protects the elements it names, not the document around them.
    let elsewhere = vec!["b3".to_string()];
    store
        .acquire_locks(
            "coffee",
            "main",
            &["b2".to_string()],
            "alex",
            600,
            1000,
            None,
        )
        .unwrap();
    let tip_now = store.branch_tip("coffee", "main").unwrap();
    let unrelated = store.commit_model(
        "coffee",
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
fn a_guard_computed_against_an_old_tip_is_refused() {
    // The guard carries the touched element set, which was computed from a document. If the
    // branch has moved since, that document is gone and the guard describes the WRONG
    // elements: enforcing it would protect something nobody is editing while a locked
    // element slips through. The store refuses rather than answering a stale question.
    use server::store::CommitGuard;

    let (store, _dir) = store();
    store.create_project("coffee", None).unwrap();
    let root = store
        .commit_model(
            "coffee", "main", "okf-root", "alex", "root", None, None, None,
        )
        .unwrap();

    // A concurrent commit moves the branch after the caller read the tip.
    store
        .commit_model(
            "coffee",
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
        "coffee",
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
        store.commits_on("coffee", "main").unwrap().len(),
        2,
        "nothing may be written"
    );
}

#[test]
fn audit_rows_ride_each_mutation_transaction() {
    // Every mutation method takes an optional audit entry and writes it inside its OWN
    // transaction. This drives the store directly so a mutation and its row are seen to
    // land together, with no HTTP layer in between to append the row afterwards.
    let (store, _dir) = store();
    let entry = |action: &str| AuditEntry {
        id: 0,
        project: "coffee".to_string(),
        at: 1,
        actor: "alex".to_string(),
        mechanism: "jwt".to_string(),
        authorizer: String::new(),
        action: action.to_string(),
        subject: "s".to_string(),
        detail: "d".to_string(),
    };

    store
        .create_project("coffee", Some(&entry("project.create")))
        .unwrap();
    let root = store
        .commit_model(
            "coffee", "main", "okf-root", "alex", "root", None, None, None,
        )
        .unwrap();
    store
        .create_branch(
            "coffee",
            "feature",
            &root.hash,
            Some(&entry("branch.create")),
        )
        .unwrap();
    store
        .acquire_locks(
            "coffee",
            "main",
            &["b1".to_string()],
            "alex",
            600,
            1000,
            Some(&entry("lock.acquire")),
        )
        .unwrap();
    let ids: Vec<String> = store
        .locks("coffee", 1000)
        .unwrap()
        .into_iter()
        .map(|l| l.id)
        .collect();
    store
        .release_locks("coffee", "alex", &ids, Some(&entry("lock.release")))
        .unwrap();
    store
        .record_gate_run(
            &GateRun {
                project: "coffee".to_string(),
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
        .delete_branch("coffee", "feature", Some(&entry("branch.delete")))
        .unwrap();

    let actions: Vec<String> = store
        .audit("coffee", 1000)
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
fn the_audit_table_refuses_update_and_delete() {
    // Append-only is a DATABASE property now, not a trait convention: triggers make UPDATE
    // and DELETE fail outright, so a written entry can never be rewritten or removed even by
    // a caller with raw SQL access to the same file.
    let (store, dir) = store();
    store.create_project("coffee", None).unwrap();
    store
        .append_audit(&AuditEntry {
            id: 0,
            project: "coffee".to_string(),
            at: 1,
            actor: "a".to_string(),
            mechanism: "open".to_string(),
            authorizer: String::new(),
            action: "x".to_string(),
            subject: "s".to_string(),
            detail: "d".to_string(),
        })
        .unwrap();

    let conn = rusqlite::Connection::open(dir.path().join("mw.db")).unwrap();
    assert!(
        conn.execute("UPDATE audit SET detail = 'rewritten' WHERE id = 1", [])
            .is_err(),
        "UPDATE on the audit table must be refused by the trigger"
    );
    assert!(
        conn.execute("DELETE FROM audit WHERE id = 1", []).is_err(),
        "DELETE on the audit table must be refused by the trigger"
    );
}
#[test]
fn a_mutation_rolls_back_when_its_audit_entry_cannot_be_written() {
    // The audit trail promises that a mutation and its record succeed or fail together.
    // Asserting that rows EXIST does not prove it: only making the audit write FAIL does.
    // The table's CHECK rejects an entry that names nobody, which forces exactly that.
    use server::store::AuditEntry;

    let (store, _dir) = store();
    let unwritable = AuditEntry {
        id: 0,
        project: "coffee".to_string(),
        at: 0,
        actor: String::new(),
        mechanism: "open".to_string(),
        authorizer: String::new(),
        action: "project.create".to_string(),
        subject: "coffee".to_string(),
        detail: "an entry that names nobody is not a record".to_string(),
    };

    let refused = store.create_project("coffee", Some(&unwritable));
    assert!(
        refused.is_err(),
        "an unwritable audit entry must fail the mutation it describes"
    );
    assert!(
        store.project("coffee").unwrap().is_none(),
        "the project must NOT survive: the mutation rolled back with its record"
    );
}

#[test]
fn an_older_database_gains_the_mechanism_column_instead_of_failing() {
    // CREATE TABLE IF NOT EXISTS is not a migration: a table that already exists keeps its
    // old shape, so a column added to the schema never reaches an installation that
    // upgrades. Without a migration path every audit write on such a database fails and
    // every mutation becomes a 500 - a failure discovered during a customer upgrade rather
    // than in a test. This builds the OLD audit table by hand, then opens it.
    use server::store::{AuditEntry, Store};

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("old.db");
    {
        let connection = rusqlite::Connection::open(&path).unwrap();
        let old_audit = "CREATE TABLE audit (id INTEGER PRIMARY KEY AUTOINCREMENT, project TEXT NOT NULL, at INTEGER NOT NULL, actor TEXT NOT NULL, action TEXT NOT NULL, subject TEXT NOT NULL, detail TEXT NOT NULL)";
        connection.execute_batch(old_audit).unwrap();
    }

    let store = SqliteStore::open(&path).expect("an older database must open, not fail");
    store.create_project("coffee", None).unwrap();
    let entry = AuditEntry {
        id: 0,
        project: "coffee".to_string(),
        at: 1,
        actor: "alex".to_string(),
        mechanism: "static".to_string(),
        authorizer: String::new(),
        action: "commit.create".to_string(),
        subject: "main".to_string(),
        detail: "after the migration".to_string(),
    };
    store
        .append_audit(&entry)
        .expect("the migrated table must accept a write");
    let rows = store.audit("coffee", 10).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].mechanism, "static");
}

#[test]
fn an_import_commit_rolls_back_when_its_import_record_is_missing() {
    // Provenance must be atomic with the commit: if the import record it names does not exist,
    // the commit and its link must fail together. A commit that landed without its source link
    // is exactly what an import must never produce, because it can no longer be traced to the
    // artifact it migrated.
    let (store, _dir) = store();
    store.create_project("coffee", None).unwrap();

    let provenance = server::store::ImportProvenance {
        artifact_hash: "missing-artifact".to_string(),
        binding_id: "sysml-v1-xmi".to_string(),
        binding_version: "2.4".to_string(),
        accepted_losses: vec!["uml:Model m".to_string()],
    };
    let refused = store.commit_model(
        "coffee",
        "main",
        "okf-hash",
        "alex",
        "import",
        None,
        None,
        Some(&provenance),
    );
    match refused {
        Err(StoreError::NotFound(_)) => {}
        other => panic!("expected NotFound, got {:?}", other.map(|c| c.hash)),
    }
    assert!(
        store.commits_on("coffee", "main").unwrap().is_empty(),
        "the commit must roll back with its missing provenance"
    );
}
