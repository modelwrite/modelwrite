// SPDX-License-Identifier: AGPL-3.0-or-later
use server::store::{commit_hash, sqlite::SqliteStore, Commit, Store, StoreError};

fn store() -> (SqliteStore, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::open(&dir.path().join("mw.db")).unwrap();
    (store, dir)
}

fn commit_for(project: &str, branch: &str, okf_hash: &str) -> Commit {
    let parents: Vec<String> = vec![];
    let hash = commit_hash(project, branch, &parents, okf_hash, "alex", "first commit");
    Commit {
        hash,
        project: project.to_string(),
        branch: branch.to_string(),
        parents,
        okf_hash: okf_hash.to_string(),
        author: "alex".to_string(),
        message: "first commit".to_string(),
        created_at: "0".to_string(),
    }
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
    store.create_project("coffee").unwrap();
    let first = commit_for("coffee", "main", "okf1");
    store.append_commit(&first).unwrap();
    assert_eq!(
        store.branch_tip("coffee", "main").unwrap().unwrap(),
        first.hash
    );

    let second = commit_for("coffee", "main", "okf2");
    store.append_commit(&second).unwrap();
    assert_eq!(
        store.branch_tip("coffee", "main").unwrap().unwrap(),
        second.hash
    );
    assert_eq!(store.commits_on("coffee", "main").unwrap().len(), 2);
}

#[test]
fn duplicate_projects_and_branches_are_conflicts() {
    let (store, _dir) = store();
    store.create_project("coffee").unwrap();
    match store.create_project("coffee") {
        Err(StoreError::Conflict(_)) => {}
        other => panic!("expected a conflict, got {:?}", other),
    }
    let commit = commit_for("coffee", "main", "okf1");
    store.append_commit(&commit).unwrap();
    store
        .create_branch("coffee", "review", &commit.hash)
        .unwrap();
    match store.create_branch("coffee", "review", &commit.hash) {
        Err(StoreError::Conflict(_)) => {}
        other => panic!("expected a conflict, got {:?}", other),
    }
    match store.create_branch("coffee", "other", "missing-commit") {
        Err(StoreError::NotFound(_)) => {}
        other => panic!("expected not found, got {:?}", other),
    }
}
#[test]
fn concurrent_commits_to_one_branch_form_a_linear_chain() {
    // The store resolves the parents, hashes and appends inside one lock and transaction.
    // If that atomicity were lost, two concurrent commits would both read the same tip
    // and fork the history: both commits would land, but one would orphan the other and
    // the chain would have two roots. This test fails loudly in that case.
    let (store, _dir) = store();
    store.create_project("coffee").unwrap();
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
                )
                .expect("commit model");
        }));
    }
    for handle in handles {
        handle.join().expect("worker thread");
    }

    let history = store.commits_on("coffee", "main").unwrap();
    assert_eq!(history.len(), 8, "every commit must land");
    let roots = history.iter().filter(|c| c.parents.is_empty()).count();
    assert_eq!(roots, 1, "exactly one commit may be rootless");
    for commit in &history {
        assert!(
            commit.parents.len() <= 1,
            "no commit may have two parents in this scenario"
        );
    }
    let links: usize = history.iter().map(|c| c.parents.len()).sum();
    assert_eq!(links, 7, "seven commits must link to their predecessor");
}
