// SPDX-License-Identifier: AGPL-3.0-or-later
//! Generate `spec/analytics/metric_definitions.json` from the engine's metric
//! catalog. The published table is never hand-written: run
//! `cargo run -p mw-analytics --example gen_metric_definitions` and commit the
//! result. The tests in `tests/metric_definitions.rs` fail if a definition is
//! missing or invented, so the artifact can only drift if the engine drifts.

fn main() {
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../spec/analytics/metric_definitions.json");
    let rows = analytics::metrics::metric_definitions();
    let value = serde_json::to_value(rows).expect("metric definitions serialize");
    let text = serde_json::to_string_pretty(&value).expect("metric definitions serialize");
    std::fs::write(&out, format!("{}\n", text)).expect("write metric_definitions.json");
    println!("wrote {}", out.display());
}
