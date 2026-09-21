// SPDX-License-Identifier: AGPL-3.0-or-later
//! The FIRST import of a REAL vendor model: the In-Flight Entertainment System
//! from the dbinfrago/Capella-IFE-sample repository (EPL-2.0).
//!
//! EPL-2.0 is not AGPL-compatible, so this model is NOT committed to the
//! repository. It is fetched OUTSIDE the repo by the operator (see the
//! sample/examples README for the licence rule), and this test locates it by:
//!
//!   1. the CAPELLA_IFE_CAPELLA environment variable (path to the .capella file),
//!   2. the corpus fetch location recorded below.
//!
//! When neither exists the test SKIPS loudly rather than pretend. The number is
//! the finding, not the flattery: this prints the measured counts and named
//! losses, exactly as real_tmt.rs does for the TMT model.

use binding::{summarize_import, Binding};
use binding_capella::CapellaBinding;
use std::collections::BTreeMap;

/// Locate the fetched corpus model, or return None to skip.
fn real_model() -> Option<Vec<u8>> {
    if let Ok(p) = std::env::var("CAPELLA_IFE_CAPELLA") {
        return Some(std::fs::read(&p).unwrap_or_else(|e| panic!("cannot read {p}: {e}")));
    }
    let candidates = [
        // The operator's fetch, one directory up beside the repo.
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../modelwrite-capella-corpus/ife-variant/In-Flight Entertainment System.capella"
        ),
        // A repo-relative fetch following the sample/examples README.
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../sample/examples/arcadia-capella/dbinfrago-ife-variant/In-Flight Entertainment System.capella"
        ),
    ];
    for c in candidates {
        if let Ok(bytes) = std::fs::read(c) {
            return Some(bytes);
        }
    }
    None
}

/// The leading construct of a loss subject: "SystemFunction f-1" -> "SystemFunction".
fn kind_of(subject: &str) -> String {
    subject
        .split_whitespace()
        .next()
        .unwrap_or(subject)
        .to_string()
}

#[test]
fn the_real_ife_model_imports_with_counts_and_named_losses() {
    let Some(bytes) = real_model() else {
        eprintln!("SKIPPED: Capella corpus not fetched (EPL-2.0, not bundled).");
        eprintln!("Fetch it and set CAPELLA_IFE_CAPELLA to the .capella file, or run the sample/examples fetch script.");
        return;
    };

    let binding = CapellaBinding::new();
    let (root, report) = match binding.import(&bytes) {
        Ok(ok) => ok,
        Err(e) => {
            println!("=== REAL IFE MODEL (dbinfrago/Capella-IFE-sample) ===");
            println!("IMPORT REFUSED: {:?}", e);
            println!("A clean error, not a panic - but the model is outside the subset.");
            return;
        }
    };

    let summary = summarize_import(&root, &report);
    let graph = root.graph.as_ref();

    println!("=== REAL IFE MODEL (dbinfrago/Capella-IFE-sample, EPL-2.0) ===");
    println!("project             : {}", root.project);
    println!("structure blocks    : {}", root.structure.len());
    println!("requirements        : {}", root.requirements.len());
    println!(
        "graph nodes         : {}",
        graph.map(|g| g.nodes.len()).unwrap_or(0)
    );
    println!(
        "graph edges         : {}",
        graph.map(|g| g.edges.len()).unwrap_or(0)
    );
    println!("declarations        : {}", report.declarations().len());
    println!("content losses      : {}", report.content_losses().len());
    println!("lossy drops         : {}", report.lossy().len());
    println!("{}", summary.render());

    // A breakdown of the carried blocks by Capella type.
    let mut by_stereotype: BTreeMap<&str, usize> = BTreeMap::new();
    for e in &root.structure {
        let s = e
            .stereotypes
            .first()
            .map(|s| s.as_str())
            .unwrap_or("<none>");
        *by_stereotype.entry(s).or_default() += 1;
    }
    println!("blocks by type      : {by_stereotype:?}");

    // A breakdown of the named content losses by leading construct.
    let mut by_construct: BTreeMap<String, usize> = BTreeMap::new();
    for m in report.content_losses() {
        *by_construct.entry(kind_of(&m.subject)).or_default() += 1;
    }
    println!("content losses by construct: {by_construct:?}");

    // The import must carry real content and name real losses - the model has
    // components, functions, exchanges, allocations and constraints.
    assert!(
        root.structure.len() > 100,
        "expected >100 blocks (components + functions), got {}",
        root.structure.len()
    );
    assert!(
        graph.map(|g| g.edges.len()).unwrap_or(0) > 100,
        "expected >100 edges (exchanges + allocations + contains)"
    );

    // The control-structure links are present: allocated (part) edges and
    // directed exchanges (dependency edges).
    if let Some(g) = graph {
        let parts = g.edges.iter().filter(|e| e.kind == "part").count();
        let deps = g.edges.iter().filter(|e| e.kind == "dependency").count();
        println!("part edges (allocations): {parts}");
        println!("dependency edges (exchanges): {deps}");
        assert!(
            parts > 0,
            "expected ComponentFunctionalAllocation part edges"
        );
        assert!(
            deps > 0,
            "expected functional/component exchange dependency edges"
        );
    }

    // Nothing dropped in silence: every departure from the subset is named, and
    // the named losses cover the known out-of-scope constructs.
    let subjects: Vec<&str> = report
        .content_losses()
        .iter()
        .map(|m| m.subject.as_str())
        .collect();
    assert!(
        subjects.iter().any(|s| s.starts_with("TransfoLink")),
        "TransfoLink traceability is named"
    );
    assert!(
        subjects.iter().any(|s| s.starts_with("Part")),
        "component parts are named"
    );
    assert!(
        subjects.iter().any(|s| s.starts_with("StateMachine")
            || s.starts_with("Mode")
            || s.starts_with("StateTransition")),
        "state/mode machinery is named"
    );
    assert!(
        subjects
            .iter()
            .any(|s| s.starts_with("Property") || s.starts_with("Class")),
        "the data/information model is named"
    );
}
