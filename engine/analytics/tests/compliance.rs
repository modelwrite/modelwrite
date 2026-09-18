// SPDX-License-Identifier: AGPL-3.0-or-later
//! Portfolio compliance, exercised through the public surface. The whole point is
//! three states, not two: a requirement present and covered is COVERED; present
//! and uncovered is UNCOVERED; absent from the model is UNKNOWN and must appear
//! as UNKNOWN, never as a blank and never omitted. The counts in a report sum to
//! the specification's size, coverage agrees with the graph engine's own function
//! on the real corpus, and each row distinguishes the model's version.

use analytics::{classify, portfolio_report, Compliance, Report};
use graph::requirement_coverage;
use okf::hash::canonical_hash;
use okf::types::OkfRoot;

fn okf(text: &str) -> OkfRoot {
    serde_json::from_str(text).expect("test OKF parses")
}

fn expected() -> OkfRoot {
    serde_json::from_str(&test_support::load_okf_expected()).expect("corpus fixture parses")
}

/// Two requirements: r1 is covered by a Satisfy edge, r2 is present but has no
/// covering edge, so it is Uncovered rather than Unknown.
const TINY: &str = r#"{
  "project": "tiny",
  "exportedAt": "2026-09-17T00:00:00Z",
  "summary": {},
  "stateMachine": {"name": "tiny sm", "regions": []},
  "requirements": [
    {"id": "r1", "name": "R1", "kind": "requirement", "stereotypes": ["Requirement"], "attributes": [], "documentation": "", "reqId": "1.1", "reqText": "covered"},
    {"id": "r2", "name": "R2", "kind": "requirement", "stereotypes": ["Requirement"], "attributes": [], "documentation": "", "reqId": "1.2", "reqText": "uncovered"}
  ],
  "graph": {
    "nodes": [
      {"id": "p1", "kind": "block", "name": "P1"},
      {"id": "r1", "kind": "requirement", "name": "R1"},
      {"id": "r2", "kind": "requirement", "name": "R2"}
    ],
    "edges": [
      {"source": "p1", "target": "r1", "kind": "dependency", "label": "Satisfy"}
    ]
  }
}"#;

/// A second product: r2 is covered here, r3 is present but uncovered, and r1 is
/// absent entirely - so a portfolio spanning both models shows r1 Covered in one
/// product and Unknown in the other.
const SECOND: &str = r#"{
  "project": "second",
  "exportedAt": "2026-09-17T00:00:00Z",
  "summary": {},
  "stateMachine": {"name": "second sm", "regions": []},
  "requirements": [
    {"id": "r2", "name": "R2", "kind": "requirement", "stereotypes": ["Requirement"], "attributes": [], "documentation": "", "reqId": "1.2", "reqText": "covered here"},
    {"id": "r3", "name": "R3", "kind": "requirement", "stereotypes": ["Requirement"], "attributes": [], "documentation": "", "reqId": "1.3", "reqText": "uncovered here"}
  ],
  "graph": {
    "nodes": [
      {"id": "p2", "kind": "block", "name": "P2"},
      {"id": "r2", "kind": "requirement", "name": "R2"},
      {"id": "r3", "kind": "requirement", "name": "R3"}
    ],
    "edges": [
      {"source": "p2", "target": "r2", "kind": "dependency", "label": "Satisfy"}
    ]
  }
}"#;

/// A model whose graph section is gone. Classifying must not panic: a present
/// requirement with no graph is Uncovered, the same reading the gate records.
const NO_GRAPH: &str = r#"{
  "project": "no graph",
  "exportedAt": "2026-09-17T00:00:00Z",
  "summary": {},
  "stateMachine": {"name": "sm", "regions": []},
  "requirements": [
    {"id": "r1", "name": "R1", "kind": "requirement", "stereotypes": ["Requirement"], "attributes": [], "documentation": "", "reqId": "1.1", "reqText": "no graph to trace through"}
  ],
  "graph": null
}"#;

#[test]
fn a_requirement_present_and_covered_is_covered() {
    let model = okf(TINY);
    assert_eq!(classify("r1", &model), Compliance::Covered);
}

#[test]
fn a_requirement_present_but_uncovered_is_uncovered() {
    let model = okf(TINY);
    // r2 has no covering edge, so it is Uncovered - and that is distinct from
    // both Covered and Unknown.
    assert_eq!(classify("r2", &model), Compliance::Uncovered);
    assert_ne!(Compliance::Uncovered, Compliance::Covered);
    assert_ne!(Compliance::Uncovered, Compliance::Unknown);
}

#[test]
fn a_requirement_absent_from_a_model_is_unknown_never_covered_and_never_omitted() {
    let model = okf(TINY);
    assert_eq!(classify("r3", &model), Compliance::Unknown);
    assert_ne!(Compliance::Unknown, Compliance::Covered);
    assert_ne!(Compliance::Unknown, Compliance::Uncovered);
}

#[test]
fn the_three_states_are_distinct() {
    assert_ne!(Compliance::Covered, Compliance::Uncovered);
    assert_ne!(Compliance::Covered, Compliance::Unknown);
    assert_ne!(Compliance::Uncovered, Compliance::Unknown);
}

#[test]
fn a_model_without_a_graph_does_not_panic() {
    let model = okf(NO_GRAPH);
    // Present but no graph to prove coverage: Uncovered, not Unknown (the model
    // does contain the requirement) and not Covered (nothing covers it).
    assert_eq!(classify("r1", &model), Compliance::Uncovered);
    assert_eq!(classify("r2", &model), Compliance::Unknown);
}

#[test]
fn counts_sum_to_the_specification_size() {
    let model = okf(TINY);
    // Five requirements in the specification, of which r4 and r5 are absent from
    // the model. If any requirement were dropped from the report, the total would
    // fall short of five and this test would fail.
    let report = portfolio_report(&["r1", "r2", "r3", "r4", "r5"], &[&model]);
    assert_eq!(report.specification_size, 5);

    let m = &report.models[0];
    assert_eq!(m.total(), 5);
    assert_eq!(
        m.covered_count() + m.uncovered_count() + m.unknown_count(),
        5
    );
    // Every requirement appears in exactly one bucket, in specification order.
    assert_eq!(m.covered, vec!["r1"]);
    assert_eq!(m.uncovered, vec!["r2"]);
    assert_eq!(m.unknown, vec!["r3", "r4", "r5"]);
}

#[test]
fn a_requirement_absent_from_every_model_still_appears_as_unknown() {
    let tiny = okf(TINY);
    let second = okf(SECOND);
    // r9 is absent from both products. It must appear in BOTH rows as Unknown,
    // never omitted, so a missing requirement cannot hide behind a blank.
    let report = portfolio_report(&["r1", "r9"], &[&tiny, &second]);
    assert_eq!(report.specification_size, 2);

    for model in &report.models {
        assert!(
            model.unknown.contains(&"r9".to_string()),
            "r9 must appear as Unknown, never omitted: {:?}",
            model.unknown
        );
        assert_eq!(model.total(), 2);
    }

    // tiny covers r1, so only r9 is Unknown there; second knows neither r1 nor
    // r9, so both are Unknown there.
    assert_eq!(report.models[0].unknown, vec!["r9"]);
    assert_eq!(report.models[1].unknown, vec!["r1", "r9"]);
}

#[test]
fn a_portfolio_spans_products_and_keeps_each_state() {
    let tiny = okf(TINY);
    let second = okf(SECOND);
    let report = portfolio_report(&["r1", "r2", "r3"], &[&tiny, &second]);

    assert_eq!(report.specification_size, 3);
    assert_eq!(report.models.len(), 2);

    // tiny covers r1, holds r2 uncovered, and does not know r3.
    let tiny = &report.models[0];
    assert_eq!(tiny.model, "tiny");
    assert_eq!(tiny.covered, vec!["r1"]);
    assert_eq!(tiny.uncovered, vec!["r2"]);
    assert_eq!(tiny.unknown, vec!["r3"]);
    assert_eq!(tiny.total(), 3);

    // second covers r2, holds r3 uncovered, and does not know r1.
    let second = &report.models[1];
    assert_eq!(second.model, "second");
    assert_eq!(second.covered, vec!["r2"]);
    assert_eq!(second.uncovered, vec!["r3"]);
    assert_eq!(second.unknown, vec!["r1"]);
    assert_eq!(second.total(), 3);
}

#[test]
fn an_empty_requirement_set_is_an_explicit_empty_report_not_a_panic() {
    let model = okf(TINY);
    let report = portfolio_report(&[], &[&model]);
    assert_eq!(report.specification_size, 0);
    let m = &report.models[0];
    assert_eq!(m.total(), 0);
    assert!(m.covered.is_empty());
    assert!(m.uncovered.is_empty());
    assert!(m.unknown.is_empty());
}

#[test]
fn an_empty_model_list_is_an_empty_report_not_a_panic() {
    let report = portfolio_report(&["r1"], &[]);
    assert_eq!(report.specification_size, 1);
    assert!(report.models.is_empty());
}

#[test]
fn a_model_compliance_row_carries_the_models_hash_and_export_time() {
    let model = okf(TINY);
    let report = portfolio_report(&["r1"], &[&model]);
    let m = &report.models[0];
    assert_eq!(m.model, "tiny");
    assert_eq!(m.exported_at, "2026-09-17T00:00:00Z");
    assert_eq!(m.model_hash, canonical_hash(&model));
    assert!(!m.model_hash.is_empty());
}

#[test]
fn two_versions_of_a_project_are_distinguishable_by_hash() {
    let v1 = okf(TINY);
    // Same project, same requirements, but a later export time: a different version.
    let v2 = okf(&TINY.replace("2026-09-17T00:00:00Z", "2026-09-18T00:00:00Z"));
    let report = portfolio_report(&["r1"], &[&v1, &v2]);
    assert_eq!(report.models[0].model, "tiny");
    assert_eq!(report.models[1].model, "tiny");
    assert_eq!(report.models[0].exported_at, "2026-09-17T00:00:00Z");
    assert_eq!(report.models[1].exported_at, "2026-09-18T00:00:00Z");
    assert_ne!(report.models[0].model_hash, report.models[1].model_hash);
}

/// The reference corpus carries 25 requirements, 15 covered and 10 uncovered. The
/// classification must agree with the graph engine's own coverage on every one of
/// them, rather than re-deriving coverage from a second implementation.
#[test]
fn coverage_agrees_with_the_engine_on_the_real_corpus() {
    let model = expected();
    let coverage = requirement_coverage(&model);
    assert_eq!(coverage.total, 25);
    assert_eq!(coverage.covered, 15);
    assert_eq!(coverage.uncovered.len(), 10);

    for requirement in &model.requirements {
        let expected_state = if coverage.uncovered.contains(&requirement.id) {
            Compliance::Uncovered
        } else {
            Compliance::Covered
        };
        assert_eq!(
            classify(&requirement.id, &model),
            expected_state,
            "requirement {} disagreed with the engine's coverage",
            requirement.id
        );
    }

    // A portfolio report over the model's own requirements groups the same way.
    let spec: Vec<&str> = model.requirements.iter().map(|r| r.id.as_str()).collect();
    let report = portfolio_report(&spec, &[&model]);
    assert_eq!(report.specification_size, 25);
    let m = &report.models[0];
    assert_eq!(m.covered.len(), 15);
    assert_eq!(m.uncovered.len(), 10);
    assert_eq!(m.unknown.len(), 0);
    assert_eq!(m.total(), 25);
}

#[test]
fn report_serializes_with_camel_case_keys() {
    let model = okf(TINY);
    let report: Report = portfolio_report(&["r1", "r2"], &[&model]);
    let value = serde_json::to_value(&report).expect("report serializes");
    let obj = value.as_object().unwrap();
    assert!(obj.contains_key("specificationSize"));
    assert!(!obj.contains_key("specification_size"));

    let row = report.models[0].clone();
    let row_value = serde_json::to_value(&row).expect("row serializes");
    let row_obj = row_value.as_object().unwrap();
    assert!(row_obj.contains_key("modelHash"));
    assert!(row_obj.contains_key("exportedAt"));
    assert!(row_obj.contains_key("covered"));
    assert!(row_obj.contains_key("uncovered"));
    assert!(row_obj.contains_key("unknown"));
}
