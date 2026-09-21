// SPDX-License-Identifier: AGPL-3.0-or-later
//! The FIRST import of a content-rich, permissively-licensed VENDOR model: the
//! Thirty Meter Telescope, exported by CATIA Magic (Cameo) as a \`.mdzip\` and
//! published by Open-MBEE under Apache-2.0 (with a Caltech BSD-style COPYRIGHT).
//!
//! Every other XMI fixture here is hand-written, a Papyrus skeleton, or the
//! small coffee-machine corpus. This is a 36 MB, 255,000-line MagicDraw SysML
//! document - thousands of Classes, Associations, Activities, States, Instances
//! and Slots. It is the real test of whether the reader can migrate a model
//! somebody else built in a vendor tool.
//!
//! Like \`real_magicdraw.rs\`, this test asserts what the platform promises
//! about ANY input - a clean result or a clean error, every departure named,
//! never a panic - and PRINTS what a real migration would actually get. The
//! number is the finding, not the flattery.

use binding::{round_trip, summarize_import, Binding, MappingVerdict};
use binding_xmi::XmiBinding;
use std::collections::BTreeMap;

fn real_model() -> Vec<u8> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../sample/examples/sysml-v1/openmbee-tmt/tmt-2022x.model.xmi"
    );
    std::fs::read(path).unwrap_or_else(|e| panic!("cannot read {}: {}", path, e))
}

/// The leading construct of a loss subject: "uml:Port _..." -> "uml:Port".
fn kind_of(subject: &str) -> String {
    let head = subject.split_whitespace().next().unwrap_or(subject);
    if head.starts_with("uml:") || head.starts_with("sysml:") || head.starts_with("xmi:") {
        head.to_string()
    } else {
        format!("{} (other)", head)
    }
}

#[test]
fn a_real_tmt_vendor_export_imports_with_every_loss_named() {
    let bytes = real_model();
    let binding = XmiBinding::new();

    let (root, report) = match binding.import(&bytes) {
        Ok(ok) => ok,
        Err(e) => {
            println!("=== REAL TMT MODEL (Open-MBEE TMT.mdzip, 2022x) ===");
            println!("IMPORT REFUSED: {:?}", e);
            println!("A clean error, not a panic - but the model is outside the subset.");
            return;
        }
    };

    println!("=== REAL TMT MODEL (Open-MBEE TMT.mdzip, 2022x) ===");
    println!("structure elements : {}", root.structure.len());
    println!("requirements       : {}", root.requirements.len());
    println!("interfaces         : {}", root.interfaces.len());
    println!("signals            : {}", root.signals.len());
    println!("activities         : {}", root.activities.len());
    println!(
        "graph nodes/edges  : {}/{}",
        root.graph.as_ref().map(|g| g.nodes.len()).unwrap_or(0),
        root.graph.as_ref().map(|g| g.edges.len()).unwrap_or(0)
    );
    let declarations = report.declarations();
    let content_losses = report.content_losses();
    let lossy = report.lossy();
    println!("declarations       : {}", declarations.len());
    println!("CONTENT LOSSES     : {}", content_losses.len());
    println!("lossy id/name drops: {}", lossy.len());
    println!("blocking           : {}", report.blocking().len());
    println!("lossless           : {}", report.is_lossless());

    // The human-first summary, exactly as the workbench renders it.
    let summary = summarize_import(&root, &report);
    println!("--- IMPORT SUMMARY (read this first) ---");
    println!("{}", summary.render());

    // The loss histogram: the biggest remaining gap is a fact, not a guess.
    let mut by_kind: BTreeMap<String, usize> = BTreeMap::new();
    let mut by_verdict: BTreeMap<String, usize> = BTreeMap::new();
    for m in &content_losses {
        *by_kind.entry(kind_of(&m.subject)).or_default() += 1;
        *by_verdict.entry(format!("{:?}", m.verdict)).or_default() += 1;
    }
    println!("--- CONTENT LOSSES by verdict ---");
    for (k, v) in &by_verdict {
        println!("  {:>6}  {}", v, k);
    }
    println!("--- CONTENT LOSSES by construct, biggest first ---");
    let mut rows: Vec<_> = by_kind.into_iter().collect();
    rows.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    for (k, v) in &rows {
        println!("  {:>6}  {}", v, k);
    }
    println!("--- a sample of each of the top ten constructs ---");
    for (kind, _) in rows.iter().take(10) {
        let sample = content_losses
            .iter()
            .find(|m| &kind_of(&m.subject) == kind)
            .map(|m| m.subject.clone())
            .unwrap_or_default();
        println!("  {}  ->  {}", kind, sample);
    }
    let unmappable = content_losses
        .iter()
        .filter(|m| m.verdict == MappingVerdict::Unmappable)
        .count();
    println!(
        "=== SUMMARY: {} content losses ({} unmappable), {} lossy, {} declarations, {} elements ===",
        content_losses.len(),
        unmappable,
        lossy.len(),
        declarations.len(),
        root.structure.len()
    );

    // --- The numbers this test exists to move ---
    // These are the measured totals for the Open-MBEE TMT 2022x vendor export
    // (upstream commit b2a33b7). They pin the measurement so a reader change that
    // silently moves them is caught, and so the number is citable. They are NOT a
    // target: the reader was not tuned to reach them, and any change must move the
    // numbers in the direction of carrying MORE, named in the loss report.
    assert_eq!(root.structure.len(), 362, "362 blocks carried");
    assert_eq!(root.requirements.len(), 7, "7 requirements carried");
    let graph = root.graph.as_ref().expect("graph present");
    assert_eq!(graph.nodes.len(), 369, "369 graph nodes");
    assert_eq!(graph.edges.len(), 633, "633 graph edges");
    assert_eq!(
        content_losses.len(),
        45725,
        "content losses, every one named"
    );
    assert_eq!(lossy.len(), 2828, "lossy id/name drops");
    assert_eq!(declarations.len(), 2231, "declarations recognised");
    assert_eq!(root.requirements.len() as u64, root.summary.requirements);
    assert!(!content_losses.iter().any(|m| m.subject.is_empty()));
}

#[test]
#[ignore = "reads the 36 MB TMT fixture; run explicitly"]
fn the_real_tmt_model_validates_so_the_only_blocker_is_the_losses() {
    // The honest end state the SECOND blocker hid: the imported TMT document now
    // VALIDATES, so the only remaining blocker is the human decision on the 48,553
    // blocking losses. The two template requirements with an empty reqId (#parent,
    // #child) are accepted as warnings - NAMED, not hidden - because a missing
    // human-facing SysML id is a fact about the source, not a structural error.
    let bytes = real_model();
    let binding = XmiBinding::new();
    let (root, report) = binding.import(&bytes).expect("import must succeed");

    let validation = okf::validate::validate(&root);
    println!("=== TMT COMMIT VALIDATION (empty reqId accepted as a warning) ===");
    println!("valid                 : {}", validation.valid);
    println!("errors                : {}", validation.errors.len());
    for error in &validation.errors {
        println!("  error: {error}");
    }
    let empty_reqid: Vec<&String> = validation
        .warnings
        .iter()
        .filter(|w| w.contains("empty reqId"))
        .collect();
    println!("empty-reqId warnings  : {}", empty_reqid.len());
    for warning in &empty_reqid {
        println!("  warning: {warning}");
    }

    assert!(
        validation.valid,
        "the TMT model must validate: {:?}",
        validation.errors
    );
    assert_eq!(
        empty_reqid.len(),
        2,
        "exactly two unnamed requirements must be named: {:?}",
        empty_reqid
    );

    // The ONLY remaining blocker is the unaccepted losses: accept all 48,553 and
    // the import commits (the round-trip test above proves fidelity; this test
    // proves validation; nothing else stands between the import and the commit).
    assert_eq!(
        report.blocking().len(),
        48553,
        "48,553 blocking losses remain the sole blocker"
    );
    let verbatim: Vec<String> = root
        .requirements
        .iter()
        .filter(|r| r.req_id.is_empty())
        .map(|r| format!("{} (name={:?}, reqId={:?})", r.id, r.name, r.req_id))
        .collect();
    println!("unnamed requirements  : {verbatim:?}");
}

#[test]
fn the_real_tmt_model_round_trips_through_the_harness() {
    // GAP: an imported-then-exported real model must retain what the import
    // carries. The engine's own diff - not the binding's claim - is the arbiter
    // of what survives the OKF -> XMI -> OKF journey.
    let bytes = real_model();
    let binding = XmiBinding::new();
    let (root, _) = binding.import(&bytes).expect("import must succeed");
    let source = serde_json::to_vec(&root).expect("source must serialize");
    let outcome = round_trip(&binding, &source).expect("round trip must succeed");
    assert!(
        outcome.diff.equal,
        "engine diff over the real TMT model:\n  missing elements: {:?}\n  extra elements: {:?}\n  missing edges: {:?}\n  extra edges: {:?}\n  changed attributes: {:?}",
        outcome.diff.missing_elements,
        outcome.diff.extra_elements,
        outcome.diff.missing_edges,
        outcome.diff.extra_edges,
        outcome.diff.changed_attributes
    );
}
