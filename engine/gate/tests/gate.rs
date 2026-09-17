// SPDX-License-Identifier: AGPL-3.0-or-later
use okf::types::OkfRoot;

fn expected() -> OkfRoot {
    serde_json::from_str(&test_support::load_okf_expected()).expect("corpus fixture parses")
}

fn okf(text: &str) -> OkfRoot {
    serde_json::from_str(text).expect("test OKF parses")
}

const TINY_GATE: &str = r#"{
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
      {"source": "p1", "target": "r1", "kind": "dependency", "label": "Satisfy"},
      {"source": "p1", "target": "r2", "kind": "reference", "label": ""}
    ]
  }
}"#;

#[test]
fn self_roundtrip_passes() {
    let r = expected();
    let outcome = gate::run(&r, &r, false);
    assert!(
        outcome.passed,
        "unexpected failures: {:?}",
        outcome.failures
    );
    assert_eq!(outcome.evidence["passed"], true);
    assert_eq!(
        outcome.evidence["referenceHash"],
        outcome.evidence["candidateHash"]
    );
}

#[test]
fn corrupted_corpus_fails() {
    let reference = expected();
    let candidate: OkfRoot =
        serde_json::from_str(&test_support::load_okf_broken()).expect("broken fixture parses");
    let outcome = gate::run(&reference, &candidate, false);
    assert!(!outcome.passed);
    assert!(outcome
        .failures
        .iter()
        .any(|f| f.contains("missing elements")));
    assert!(outcome.failures.iter().any(|f| f.contains("isolated")));
}

#[test]
fn strict_coverage_fails_on_uncovered() {
    let r = okf(TINY_GATE);
    let lenient = gate::run(&r, &r, false);
    assert!(
        lenient.passed,
        "unexpected failures: {:?}",
        lenient.failures
    );
    let strict = gate::run(&r, &r, true);
    assert!(!strict.passed);
    assert!(strict
        .failures
        .iter()
        .any(|f| f.contains("uncovered requirements")));
}
