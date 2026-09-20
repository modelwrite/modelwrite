// SPDX-License-Identifier: AGPL-3.0-or-later
//! The real-model proof for the STREAMING artifact path: the Open-MBEE Thirty Meter
//! Telescope XMI (a 36 MB Cameo vendor export, Apache-2.0) is staged to a file and moved
//! into the content-addressed blob store through `put_blob_file`, then imported through the
//! SAME core the workbench and the JSON/streaming endpoints call. The import is lossy - the
//! binding names 45,725 content losses - so without acceptance the import is refused as
//! `Blocking`; the artifact is still retained byte-for-byte under its hash first.
//!
//! This test is `#[ignore]`d because it reads a 36 MB fixture and runs a full import with
//! the engine's fidelity round trip; it is run explicitly to record the verbatim outcome.

use server::binding_api::{import_core, ImportCore, ImportOutcome};
use server::store::Store;
use std::time::Instant;

fn tmt_path() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../sample/examples/sysml-v1/openmbee-tmt/tmt-2022x.model.xmi")
}

#[test]
#[ignore]
fn the_36mb_tmt_model_streams_into_the_blob_store_and_reaches_the_importer() {
    let dir = tempfile::tempdir().unwrap();
    let store = server::store::sqlite::SqliteStore::open(&dir.path().join("mw.db")).unwrap();
    store.create_project("tmt", None).unwrap();

    let source = std::fs::read(tmt_path()).expect("TMT XMI must be committed");
    let expected_hash = server::store::blob_hash(&source);

    // Stage the committed file to a temporary path, exactly as the streaming handler does,
    // then move it into the blob store (the store removes the staged file on success).
    let staged = dir.path().join("tmt-staged.xmi");
    std::fs::write(&staged, &source).unwrap();
    let store_start = Instant::now();
    let artifact_hash = store.put_blob_file(&staged).unwrap();
    let store_elapsed = store_start.elapsed();
    assert_eq!(
        artifact_hash, expected_hash,
        "the stored hash must match the bytes"
    );
    assert!(
        !staged.exists(),
        "the staged file must be removed after storing"
    );

    let artifact = store
        .blob(&artifact_hash)
        .unwrap()
        .expect("the retained artifact must be readable back");
    assert_eq!(
        artifact, source,
        "the retained artifact must be byte-for-byte"
    );

    let import_start = Instant::now();
    let outcome = import_core(
        &store,
        "tmt",
        &ImportCore {
            binding: "sysml-v1-xmi@2.4",
            branch: "main",
            author: "alex",
            message: "import the Open-MBEE TMT model",
            artifact: &artifact,
            accept_losses: &[],
            holder: None,
            actor: "alex",
            mechanism: "open",
            authorizer: "",
            acceptance: None,
        },
    );
    let import_elapsed = import_start.elapsed();

    println!("=== TMT STREAMING IMPORT (Open-MBEE TMT.mdzip, 2022x, Apache-2.0) ===");
    println!("source bytes         : {}", source.len());
    println!("artifact hash        : {}", artifact_hash);
    println!("streamed+stored in   : {:?}", store_elapsed);
    println!("imported in          : {:?}", import_elapsed);
    match &outcome {
        Ok(ImportOutcome::Committed {
            commit,
            binding_id,
            binding_version,
            ..
        }) => {
            println!("outcome              : Committed");
            println!("binding              : {}@{}", binding_id, binding_version);
            println!("commit               : {}", commit.hash);
        }
        Ok(ImportOutcome::Blocking {
            binding_id,
            binding_version,
            unaccepted,
            ..
        }) => {
            println!(
                "outcome              : Blocking (refused: {} blocking losses not accepted)",
                unaccepted.len()
            );
            println!("binding              : {}@{}", binding_id, binding_version);
        }
        Err(error) => {
            println!(
                "outcome              : Error (status {}): {}",
                error.status, error.message
            );
        }
    }

    // The founding rule: whatever the import verdict, the artifact was retained BEFORE the
    // import was attempted, under a hash that matches the bytes.
    assert_eq!(
        store.blob(&artifact_hash).unwrap().as_deref(),
        Some(&source[..]),
        "the artifact must be retained byte-for-byte regardless of the import verdict"
    );
}
