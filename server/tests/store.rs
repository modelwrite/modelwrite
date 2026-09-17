// SPDX-License-Identifier: AGPL-3.0-or-later
use server::store::{commit_hash, sqlite::SqliteStore, Store, StoreError};

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
    store.create_project("coffee").unwrap();
    let first = store
        .commit_model("coffee", "main", "okf1", "alex", "first commit")
        .unwrap();
    assert_eq!(
        store.branch_tip("coffee", "main").unwrap().unwrap(),
        first.hash
    );

    let second = store
        .commit_model("coffee", "main", "okf2", "alex", "second commit")
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
    store.create_project("coffee").unwrap();
    match store.create_project("coffee") {
        Err(StoreError::Conflict(_)) => {}
        other => panic!("expected a conflict, got {:?}", other),
    }
    let commit = store
        .commit_model("coffee", "main", "okf1", "alex", "first commit")
        .unwrap();
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
    // If that atomicity were lost, two concurrent commits would read the same tip and
    // become SIBLINGS: both land, one orphans the other. Counting roots would not see
    // that - a sibling fork still has one root - so linearity is checked the only way
    // that catches it: by counting each commit's children and by walking the parents
    // from the tip back to the root.
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
    store.create_project("coffee").unwrap();
    let root = store
        .commit_model("coffee", "main", "okf-root", "alex", "root")
        .unwrap();
    store
        .create_branch("coffee", "feature", &root.hash)
        .unwrap();
    let their_side = store
        .commit_model("coffee", "feature", "okf-feature", "alex", "feature")
        .unwrap();

    // The concurrent commit: main moves AFTER the merge read its tips.
    let concurrent = store
        .commit_model("coffee", "main", "okf-concurrent", "alex", "concurrent")
        .unwrap();

    let refused = store.commit_merge(
        "coffee",
        "main",
        &[root.hash.clone(), their_side.hash.clone()],
        "okf-merged",
        "alex",
        "merge",
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
