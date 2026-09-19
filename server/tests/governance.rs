// SPDX-License-Identifier: AGPL-3.0-or-later
//! The migration rules are a property of the repository, not of one endpoint. A commit that
//! DECLARES itself imported must name a retained artifact that exists, and every blocking loss
//! in the recorded report must have been accepted by name. The check lives where the commit is
//! written (the store's `commit_model`), so it holds for the import endpoint, any direct
//! store caller, and any route that does not exist yet - all of which reach `commit_model`.
//! These tests prove it by driving the shared commit core and the store directly - never the
//! import endpoint.

use okf::types::OkfRoot;
use server::api::{commit_core, CommitCore, CommitFailure};
use server::store::{
    sqlite::SqliteStore, Commit, CommitProvenance, ImportProvenance, Store, StoreError,
};

fn store() -> (SqliteStore, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::open(&dir.path().join("mw.db")).unwrap();
    (store, dir)
}

/// A valid OKF document and its exact bytes, the same shape the plain commit endpoint accepts.
fn candidate() -> (OkfRoot, Vec<u8>) {
    let root: OkfRoot = serde_json::from_value(serde_json::json!({
        "project": "tiny",
        "exportedAt": "2026-09-17T00:00:00Z",
        "summary": {},
        "stateMachine": { "name": "tiny sm", "regions": [] },
        "graph": { "nodes": [{ "id": "b1", "kind": "block", "name": "B1" }], "edges": [] }
    }))
    .unwrap();
    let bytes = serde_json::to_vec(&root).unwrap();
    (root, bytes)
}

fn provenance(artifact_hash: &str, accepted_losses: Vec<String>) -> ImportProvenance {
    ImportProvenance {
        artifact_hash: artifact_hash.to_string(),
        binding_id: "sysml-v1-xmi".to_string(),
        binding_version: "2.4".to_string(),
        accepted_losses,
        acceptance: None,
    }
}

/// Unwrap a commit that must land, with a panic message that names which commit failed.
fn landed(result: Result<Commit, CommitFailure>, context: &str) -> Commit {
    match result {
        Ok(commit) => commit,
        Err(CommitFailure::Invalid { errors }) => {
            panic!("{} should land, but was invalid: {:?}", context, errors)
        }
        Err(CommitFailure::Store(error)) => {
            panic!("{} should land, but was refused: {:?}", context, error)
        }
    }
}

/// A loss report naming one blocking (lossy) entry, serialized exactly as the store persists it.
fn lossy_report(artifact_hash: &str) -> String {
    let report = binding::LossReport {
        binding: binding::BindingInfo {
            id: "sysml-v1-xmi".to_string(),
            version: "2.4".to_string(),
            direction: binding::Direction::ImportAndExport,
            description: "test binding".to_string(),
        },
        mappings: vec![binding::Mapping {
            subject: "uml:Model model-grinder".to_string(),
            verdict: binding::MappingVerdict::Lossy,
            note: "dropped body".to_string(),
        }],
        artifact_hash: artifact_hash.to_string(),
    };
    serde_json::to_string(&report).unwrap()
}

#[test]
fn an_imported_commit_whose_artifact_is_missing_is_refused_by_the_commit_path() {
    // The import record exists and its report is valid, but the retained artifact bytes were
    // never stored. A route that skips retention must not be able to substantiate its import
    // claim: the commit path refuses it before writing anything.
    let (store, _dir) = store();
    store.create_project("coffee", None).unwrap();
    store
        .record_import(
            "coffee",
            "missing-artifact",
            "sysml-v1-xmi",
            "2.4",
            &lossy_report("missing-artifact"),
            "{}",
        )
        .unwrap();

    let (root, bytes) = candidate();
    let provenance = provenance("missing-artifact", vec![]);
    let result = commit_core(
        &store,
        &CommitCore {
            project: "coffee",
            branch: "main",
            author: "alex",
            message: "import",
            actor: "alex",
            mechanism: "open",
            authorizer: "",
            candidate: &root,
            bytes: &bytes,
            import: Some(&provenance),
            acceptance: None,
            holder: "",
            now: 1,
            tip: None,
            reference: None,
        },
    );

    match result {
        Err(CommitFailure::Store(StoreError::NotFound(message))) => {
            assert!(message.contains("artifact"), "message: {}", message);
        }
        Err(CommitFailure::Store(other)) => panic!("expected NotFound, got {:?}", other),
        Err(CommitFailure::Invalid { errors }) => {
            panic!("expected a store refusal, got invalid: {:?}", errors)
        }
        Ok(commit) => panic!("expected refusal, got commit {}", commit.hash),
    }
    assert!(
        store.commits_on("coffee", "main").unwrap().is_empty(),
        "no commit may be written"
    );
}

#[test]
fn a_commit_with_unaccepted_losses_cannot_be_marked_imported_by_the_commit_path() {
    // The artifact is retained and the report names one blocking loss, but the provenance
    // accepts nothing. The commit path refuses rather than writing an imported commit whose
    // losses were never accepted.
    let (store, _dir) = store();
    store.create_project("coffee", None).unwrap();

    let artifact_hash = store.put_blob(b"source xmi bytes").unwrap();
    store
        .record_import(
            "coffee",
            &artifact_hash,
            "sysml-v1-xmi",
            "2.4",
            &lossy_report(&artifact_hash),
            "{}",
        )
        .unwrap();

    let (root, bytes) = candidate();
    let provenance = provenance(&artifact_hash, vec![]);
    let result = commit_core(
        &store,
        &CommitCore {
            project: "coffee",
            branch: "main",
            author: "alex",
            message: "import",
            actor: "alex",
            mechanism: "open",
            authorizer: "",
            candidate: &root,
            bytes: &bytes,
            import: Some(&provenance),
            acceptance: None,
            holder: "",
            now: 1,
            tip: None,
            reference: None,
        },
    );

    match result {
        Err(CommitFailure::Store(StoreError::Conflict(message))) => {
            assert!(message.contains("not accepted"), "message: {}", message);
        }
        Err(CommitFailure::Store(other)) => panic!("expected Conflict, got {:?}", other),
        Err(CommitFailure::Invalid { errors }) => {
            panic!("expected a store refusal, got invalid: {:?}", errors)
        }
        Ok(commit) => panic!("expected refusal, got commit {}", commit.hash),
    }
    assert!(
        store.commits_on("coffee", "main").unwrap().is_empty(),
        "no commit may be written"
    );
}

#[test]
fn the_store_refuses_an_unsubstantiated_import_so_any_commit_path_inherits_the_rule() {
    // This test drives store.commit_model directly - not the import endpoint, not commit_core,
    // and NOT the offline CLI (the CLI has no import command; its commit path writes Authored
    // commits only). The rule must hold in the store itself, so that any route reaching
    // commit_model - today's import endpoint, a future import command, or a direct caller -
    // inherits it and cannot write an imported commit with losses it never accepted.
    let (store, _dir) = store();
    store.create_project("coffee", None).unwrap();

    let artifact_hash = store.put_blob(b"source xmi bytes").unwrap();
    store
        .record_import(
            "coffee",
            &artifact_hash,
            "sysml-v1-xmi",
            "2.4",
            &lossy_report(&artifact_hash),
            "{}",
        )
        .unwrap();

    let provenance = provenance(&artifact_hash, vec![]);
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
        Err(StoreError::Conflict(message)) => {
            assert!(message.contains("not accepted"), "message: {}", message);
        }
        Err(other) => panic!("expected Conflict, got {:?}", other),
        Ok(commit) => panic!("expected refusal, got commit {}", commit.hash),
    }
    assert!(
        store.commits_on("coffee", "main").unwrap().is_empty(),
        "no commit may be written"
    );
}

#[test]
fn a_provenance_binding_that_disagrees_with_the_record_is_refused_by_the_commit_path() {
    // The import record names binding sysml-v1-xmi@2.4, but the caller's provenance names a
    // different binding. The provenance is substantiated, not asserted: the commit path must
    // read the record and refuse the mismatch rather than copying a claim the caller made up.
    let (store, _dir) = store();
    store.create_project("coffee", None).unwrap();

    let artifact_hash = store.put_blob(b"source xmi bytes").unwrap();
    store
        .record_import(
            "coffee",
            &artifact_hash,
            "sysml-v1-xmi",
            "2.4",
            &lossy_report(&artifact_hash),
            "{}",
        )
        .unwrap();

    // Accept the one blocking loss so the ONLY thing wrong is the binding claim.
    let mut provenance = provenance(
        &artifact_hash,
        vec!["uml:Model model-grinder [lossy]".to_string()],
    );
    provenance.binding_id = "some-other-binding".to_string();

    let (root, bytes) = candidate();
    let result = commit_core(
        &store,
        &CommitCore {
            project: "coffee",
            branch: "main",
            author: "alex",
            message: "import",
            actor: "alex",
            mechanism: "open",
            authorizer: "",
            candidate: &root,
            bytes: &bytes,
            import: Some(&provenance),
            acceptance: None,
            holder: "",
            now: 1,
            tip: None,
            reference: None,
        },
    );

    match result {
        Err(CommitFailure::Store(StoreError::Conflict(message))) => {
            assert!(message.contains("binding"), "message: {}", message);
            assert!(message.contains("does not match"), "message: {}", message);
        }
        Err(CommitFailure::Store(other)) => panic!("expected Conflict, got {:?}", other),
        Err(CommitFailure::Invalid { errors }) => {
            panic!("expected a store refusal, got invalid: {:?}", errors)
        }
        Ok(commit) => panic!("expected refusal, got commit {}", commit.hash),
    }
    assert!(
        store.commits_on("coffee", "main").unwrap().is_empty(),
        "no commit may be written"
    );
}

#[test]
fn an_imported_commit_with_substantiated_provenance_lands_through_the_commit_path() {
    // Accepting the one blocking loss by its entry identity is honest provenance, so the
    // imported commit lands with its provenance written atomically.
    let (store, _dir) = store();
    store.create_project("coffee", None).unwrap();

    let expected_artifact_hash = store.put_blob(b"source xmi bytes").unwrap();
    store
        .record_import(
            "coffee",
            &expected_artifact_hash,
            "sysml-v1-xmi",
            "2.4",
            &lossy_report(&expected_artifact_hash),
            "{}",
        )
        .unwrap();

    let (root, bytes) = candidate();
    let provenance = provenance(
        &expected_artifact_hash,
        vec!["uml:Model model-grinder [lossy]".to_string()],
    );
    let commit = landed(
        commit_core(
            &store,
            &CommitCore {
                project: "coffee",
                branch: "main",
                author: "alex",
                message: "import",
                actor: "alex",
                mechanism: "open",
                authorizer: "",
                candidate: &root,
                bytes: &bytes,
                import: Some(&provenance),
                acceptance: None,
                holder: "",
                now: 1,
                tip: None,
                reference: None,
            },
        ),
        "a substantiated import",
    );

    match &commit.provenance {
        CommitProvenance::Imported { artifact_hash, .. } => {
            assert_eq!(artifact_hash, &expected_artifact_hash);
        }
        other => panic!("expected imported provenance, got {:?}", other),
    }
}

#[test]
fn an_authored_commit_is_not_held_to_migration_rules() {
    // A commit that is NOT imported needs no artifact and no loss report: it must not be made
    // to satisfy migration requirements.
    let (store, _dir) = store();
    store.create_project("coffee", None).unwrap();

    let (root, bytes) = candidate();
    let commit = landed(
        commit_core(
            &store,
            &CommitCore {
                project: "coffee",
                branch: "main",
                author: "alex",
                message: "typed",
                actor: "alex",
                mechanism: "open",
                authorizer: "",
                candidate: &root,
                bytes: &bytes,
                import: None,
                acceptance: None,
                holder: "",
                now: 1,
                tip: None,
                reference: None,
            },
        ),
        "an authored commit",
    );

    assert_eq!(commit.provenance, CommitProvenance::Authored);
}
