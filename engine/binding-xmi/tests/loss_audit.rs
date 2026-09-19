// SPDX-License-Identifier: AGPL-3.0-or-later
//! An AUDIT of exactly what the real MagicDraw import still loses.
//!
//! A single headline number is not actionable: nobody can decide what to build
//! next from it. This test turns it into a histogram, so the biggest remaining
//! gap is a fact rather than a guess. (Diagram/file metadata is reclassified as
//! declarations, so it is deliberately absent from the content-loss histogram.)

use binding::Binding;
use binding_xmi::XmiBinding;
use std::collections::BTreeMap;

fn real_model() -> Vec<u8> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../real-world/mdzip/model.xmi"
    );
    std::fs::read(path).unwrap_or_else(|e| panic!("cannot read {}: {}", path, e))
}

/// The leading construct of a loss subject: "uml:Port _2026x_1_..." -> "uml:Port".
fn kind_of(subject: &str) -> String {
    let head = subject.split_whitespace().next().unwrap_or(subject);
    if head.starts_with("uml:") || head.starts_with("sysml:") || head.starts_with("xmi:") {
        head.to_string()
    } else {
        format!("{} (other)", head)
    }
}

#[test]
fn the_remaining_losses_are_counted_by_kind() {
    let bytes = real_model();
    let binding = XmiBinding::new();
    let (_root, report) = binding.import(&bytes).expect("the real model must import");

    let losses = report.content_losses();
    let mut by_kind: BTreeMap<String, usize> = BTreeMap::new();
    let mut by_verdict: BTreeMap<String, usize> = BTreeMap::new();
    for m in &losses {
        *by_kind.entry(kind_of(&m.subject)).or_default() += 1;
        *by_verdict.entry(format!("{:?}", m.verdict)).or_default() += 1;
    }

    println!("=== LOSS AUDIT: {} content losses ===", losses.len());
    println!("--- by verdict ---");
    for (k, v) in &by_verdict {
        println!("  {:>5}  {}", v, k);
    }
    println!("--- by construct, biggest first ---");
    let mut rows: Vec<_> = by_kind.into_iter().collect();
    rows.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    for (k, v) in &rows {
        println!("  {:>5}  {}", v, k);
    }
    println!("--- a sample of each of the top three ---");
    for (kind, _) in rows.iter().take(3) {
        let sample = losses
            .iter()
            .find(|m| &kind_of(&m.subject) == kind)
            .map(|m| m.subject.clone())
            .unwrap_or_default();
        println!("  {}  ->  {}", kind, sample);
    }
    println!(
        "=== TOTAL {} losses across {} distinct constructs ===",
        losses.len(),
        rows.len()
    );
}
