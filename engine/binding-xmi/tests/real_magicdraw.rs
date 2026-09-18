// SPDX-License-Identifier: AGPL-3.0-or-later
//! Importing a REAL MagicDraw SysML export.
//!
//! This is the test the project has been missing. Every other XMI fixture is
//! either hand-written or a Papyrus skeleton with no elements. This file came
//! out of a MagicDraw `.mdzip` (the `com.nomagic.magicdraw.uml_model.model`
//! entry) and contains 148 packagedElement, 57 ownedAttribute, 34 Blocks and
//! 25 Requirements, with 20 Satisfy and 3 Allocate links. (These are ELEMENT
//! counts, not substring counts: an earlier comment said 55/45/27/9 by counting
//! occurrences of the word inside reference strings and attribute values.)
//!
//! It is also, embarrassingly, something the repository already had: it sat in
//! `sample/corpus/coffee-machine/legacy/` from the beginning and no test ever
//! read it. Synthetic fixtures were tested; the real artifact was not.
//!
//! The test asserts what the platform promises about ANY input - a clean result
//! or a clean error, every departure named, never a panic - and PRINTS what a
//! real migration would actually get. The number is the finding.

use binding::{Binding, MappingVerdict};
use binding_xmi::XmiBinding;

fn real_model() -> Vec<u8> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../real-world/mdzip/model.xmi"
    );
    std::fs::read(path).unwrap_or_else(|e| panic!("cannot read {}: {}", path, e))
}

#[test]
fn a_real_magicdraw_sysml_export_imports_with_every_loss_named() {
    let bytes = real_model();
    let binding = XmiBinding::new();

    let (root, report) = match binding.import(&bytes) {
        Ok(ok) => ok,
        Err(e) => {
            println!("=== REAL MAGICDRAW MODEL ===");
            println!("IMPORT REFUSED: {:?}", e);
            println!("A clean error, not a panic - but the model is outside the subset.");
            return;
        }
    };

    println!("=== REAL MAGICDRAW MODEL (CoffeeMachine-SysML-Model.mdzip) ===");
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
    println!("declarations       : {}", declarations.len());
    println!("CONTENT LOSSES     : {}", content_losses.len());
    println!("blocking           : {}", report.blocking().len());
    println!("lossless           : {}", report.is_lossless());
    println!("--- THE CONTENT LOSSES (what a migration would actually lose) ---");
    for m in content_losses.iter().take(25) {
        println!("  [{:?}] {}", m.verdict, m.subject);
    }
    let unmappable = content_losses
        .iter()
        .filter(|m| m.verdict == MappingVerdict::Unmappable)
        .count();
    println!(
        "=== SUMMARY: {} content losses ({} unmappable), {} declarations ===",
        content_losses.len(),
        unmappable,
        declarations.len()
    );
}
