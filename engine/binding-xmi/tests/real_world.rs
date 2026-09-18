// SPDX-License-Identifier: AGPL-3.0-or-later
//! The FIRST import of a REAL SysML model.
//!
//! Every other fixture in this crate is hand-written. This one is not: it is
//! LandingGear.uml, a Papyrus SysML model committed to a public repository, and
//! it is here to answer a question a synthetic fixture cannot - WHAT DOES THE
//! READER ACTUALLY DO WITH A MODEL SOMEBODY ELSE MADE?
//!
//! The first real import exposed a usability defect no hand-written fixture
//! could: the reader reported the file's profile applications and package
//! imports as Unmappable losses, burying the fact that the document is a
//! Papyrus project SKELETON with zero model content. A report that names every
//! declaration as a loss is unusable. This test pins the fix: declarations are
//! classified separately and never render as content losses, the summary is
//! what a human reads first, and the no-content statement is one line.

use binding::{summarize_import, Binding, ImportVerdict, MappingVerdict};
use binding_xmi::XmiBinding;

fn real_model() -> Vec<u8> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../real-world/LandingGear.uml"
    );
    std::fs::read(path).unwrap_or_else(|e| panic!("cannot read {}: {}", path, e))
}

#[test]
fn a_real_papyrus_skeleton_reports_zero_content_losses_and_names_its_declarations() {
    let bytes = real_model();
    let binding = XmiBinding::new();

    // The one thing that must hold for ANY input: a clean result or a clean
    // error, never a panic.
    let (root, report) = binding.import(&bytes).expect("import must succeed");

    let summary = summarize_import(&root, &report);

    // The summary is the FIRST thing a human reads; the detailed list follows.
    println!("=== IMPORT SUMMARY (read this first) ===");
    println!("{}", summary.render());
    println!("--- detailed mappings ---");
    for m in &report.mappings {
        println!("  [{:?}] {} :: {}", m.verdict, m.subject, m.note);
    }

    // THE FINDING: a Papyrus skeleton has no model content. The report must say
    // zero CONTENT losses, count the declarations separately, state plainly that
    // there is no content, and name the pathmap:// references.
    assert_eq!(root.structure.len(), 0);
    assert_eq!(summary.elements_imported, 0);
    assert_eq!(
        summary.content_losses, 0,
        "a skeleton has zero content losses; declarations are not losses"
    );
    assert!(summary.no_model_content);
    assert!(
        summary.statement().contains("contains no elements"),
        "the no-content statement is the first thing a human reads, got: {}",
        summary.statement()
    );
    assert_eq!(summary.verdict, ImportVerdict::Lossless);

    // Declarations are counted separately from content losses: 10 profile
    // applications, 2 package imports, and 1 root-metadata attribute.
    assert_eq!(summary.declarations_recognised, 13);
    assert_eq!(report.declarations().len(), 13);

    // The only remaining non-declaration entry is the model id drop, which is a
    // Lossy fidelity note, NOT a content loss, and does not bury anything.
    assert_eq!(summary.lossy, 1);
    assert_eq!(report.blocking().len(), 1);
    assert_eq!(report.mappings.len(), 14);

    // Every declaration is Exact (never a loss) and every Exact entry is a
    // declaration - the two classifications are the same set, at a glance.
    assert!(report
        .declarations()
        .iter()
        .all(|m| m.verdict == MappingVerdict::Exact));
    assert_eq!(
        report
            .mappings
            .iter()
            .filter(|m| m.verdict == MappingVerdict::Exact)
            .count(),
        report.declarations().len()
    );

    // The declarations break down into the 10 profile applications and 2 package
    // imports the file actually contains, each naming its Eclipse-internal
    // pathmap:// reference and saying it cannot resolve outside the IDE.
    let profiles = report
        .mappings
        .iter()
        .filter(|m| m.subject.starts_with("uml:ProfileApplication"))
        .collect::<Vec<_>>();
    let imports = report
        .mappings
        .iter()
        .filter(|m| m.subject.starts_with("uml:PackageImport"))
        .collect::<Vec<_>>();
    assert_eq!(profiles.len(), 10);
    assert_eq!(imports.len(), 2);
    assert!(profiles.iter().all(|m| {
        m.note.contains("pathmap://") && m.note.contains("cannot resolve outside the IDE")
    }));
    assert!(imports.iter().all(|m| m.note.contains("pathmap://")));
}
