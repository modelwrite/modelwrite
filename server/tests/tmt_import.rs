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

use agent::losses::entry_identity;
use server::binding_api::{accept_import_core, import_core, ImportCore, ImportOutcome};
use server::store::{CommitProvenance, Store};
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

#[test]
#[ignore]
fn the_36mb_tmt_model_commits_end_to_end_after_accepting_its_losses() {
    // THE CAPSTONE: import the real 36 MB TMT vendor model, take the refusal's 48,553 blocking
    // losses, accept them THROUGH the existing acceptance core (re-read the retained bytes by
    // hash and re-run import_core - the same path /import/{hash}/accept calls), and prove the
    // commit lands with provenance naming the artifact hash and the accepted loss count.
    let dir = tempfile::tempdir().unwrap();
    let store = server::store::sqlite::SqliteStore::open(&dir.path().join("mw.db")).unwrap();
    store.create_project("tmt", None).unwrap();

    let source = std::fs::read(tmt_path()).expect("TMT XMI must be committed");
    let expected_hash = server::store::blob_hash(&source);

    // Retain the artifact byte-for-byte before import, exactly as the streaming handler does,
    // then read it back from the blob store.
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

    // First import with nothing accepted: a Blocking refusal naming every loss.
    let import_start = Instant::now();
    let refused = import_core(
        &store,
        "tmt",
        &ImportCore {
            binding: "sysml-v1-xmi@2.4",
            branch: "main",
            author: "alex",
            message: "import the Open-MBEE TMT model",
            artifact: &source,
            accept_losses: &[],
            holder: None,
            actor: "alex",
            mechanism: "open",
            authorizer: "",
            acceptance: None,
        },
    );
    let import_elapsed = import_start.elapsed();
    let unaccepted = match refused {
        Ok(ImportOutcome::Blocking { unaccepted, .. }) => unaccepted,
        Ok(ImportOutcome::Committed { commit, .. }) => {
            panic!(
                "expected a Blocking refusal, but the import committed {}",
                commit.hash
            )
        }
        Err(error) => panic!(
            "the import errored: status {}: {}",
            error.status, error.message
        ),
    };
    let blocking_count = unaccepted.len();
    assert_eq!(
        blocking_count, 48_553,
        "the TMT model names 48,553 blocking losses"
    );

    // Name every blocking loss by its entry identity, then accept the whole set through the
    // existing acceptance core. Acceptance covers every entry with the unique identities.
    let mut identities: Vec<String> = unaccepted.iter().map(entry_identity).collect();
    identities.sort();
    identities.dedup();
    let accepted_count = identities.len();

    let accept_start = Instant::now();
    let committed = accept_import_core(
        &store,
        "tmt",
        &artifact_hash,
        "main",
        "alex",
        "accept the Open-MBEE TMT blocking losses",
        None,
        &identities,
        "alex",
        "open",
        "",
    );
    let accept_elapsed = accept_start.elapsed();

    let commit = match committed {
        Ok(ImportOutcome::Committed { commit, .. }) => commit,
        Ok(ImportOutcome::Blocking { unaccepted, .. }) => panic!(
            "the acceptance must commit, but {} losses remain blocking",
            unaccepted.len()
        ),
        Err(error) => panic!(
            "the acceptance errored: status {}: {}",
            error.status, error.message
        ),
    };

    // The provenance must name the artifact hash, the binding and the accepted loss count.
    let prov = match &commit.provenance {
        CommitProvenance::Imported {
            artifact_hash,
            binding_id,
            binding_version,
            accepted_losses,
        } => {
            assert_eq!(artifact_hash.as_str(), expected_hash);
            assert_eq!(binding_id.as_str(), "sysml-v1-xmi");
            assert_eq!(binding_version.as_str(), "2.4");
            assert_eq!(accepted_losses.len(), accepted_count);
            (
                artifact_hash.as_str(),
                binding_id.as_str(),
                binding_version.as_str(),
                accepted_losses.len(),
            )
        }
        other => panic!("the commit provenance must be Imported, got {:?}", other),
    };

    // The committed model, read back by hash.
    let root = server::api::load_model(&store, "tmt", &commit.hash).unwrap();
    let elements = root.structure.len() + root.interfaces.len() + root.signals.len();
    let relationships = root.graph.as_ref().map(|g| g.edges.len()).unwrap_or(0);
    let requirements = root.requirements.len();
    assert_eq!(
        elements, 362,
        "362 elements carried into the committed model"
    );
    assert_eq!(relationships, 633, "633 relationships carried");
    assert_eq!(requirements, 7, "7 requirements carried");

    // The completeness analytics over the committed vendor model: the coverage view and the
    // health / graph-gaps view.
    let coverage = graph::requirement_coverage(&root);
    let stats = graph::graph_stats(&root);
    let components = graph::components(&root);

    println!("=== TMT END-TO-END MIGRATION (Open-MBEE TMT.mdzip, 2022x, Apache-2.0) ===");
    println!("source bytes          : {}", source.len());
    println!("artifact hash         : {}", artifact_hash);
    println!("streamed+stored in    : {:?}", store_elapsed);
    println!("imported (refused) in : {:?}", import_elapsed);
    println!("blocking losses       : {}", blocking_count);
    println!("unique identities     : {}", accepted_count);
    println!("accepted losses       : {}", prov.3);
    println!("acceptance+commit in  : {:?}", accept_elapsed);
    println!("commit                : {}", commit.hash);
    println!(
        "provenance            : kind=imported artifactHash={} bindingId={} bindingVersion={} acceptedLosses={}",
        prov.0, prov.1, prov.2, prov.3
    );
    println!(
        "committed model       : {} elements, {} relationships, {} requirements",
        elements, relationships, requirements
    );
    println!("--- coverage view ---");
    println!(
        "requirements          : total={} covered={} uncovered={}",
        coverage.total,
        coverage.covered,
        coverage.uncovered.len()
    );
    println!(
        "coverage links        : satisfied={} refined={} verified={} allocated={}",
        coverage.satisfied, coverage.refined, coverage.verified, coverage.allocated
    );
    for id in &coverage.uncovered {
        println!("  uncovered            : {}", id);
    }
    println!("--- health / graph-gaps view ---");
    println!(
        "graph                 : nodes={} edges={} isolated={} components={}",
        stats.node_count,
        stats.edge_count,
        stats.isolated.len(),
        stats.component_count
    );
    for id in &stats.isolated {
        println!("  orphan               : {}", id);
    }
    println!("component sizes       : {:?}", stats.component_sizes);
    for (index, group) in components.groups.iter().enumerate() {
        println!("  component {}          : {} members", index, group.len());
    }

    // Byte identity survives the whole round trip: the retained artifact is unchanged.
    assert_eq!(
        store.blob(&artifact_hash).unwrap().as_deref(),
        Some(&source[..]),
        "the retained artifact must be byte-identical after the commit"
    );
}
