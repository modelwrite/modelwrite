// SPDX-License-Identifier: AGPL-3.0-or-later
//! Cost joins, exercised through the public surface. The whole point is that an
//! uncosted requirement is UNCOSTED and never zero and still appears; a cost is
//! attributed to its source and date; trust is resolved from the registry, never
//! self-attested; a dataset record for an unknown requirement is surfaced, never
//! silently dropped; a dataset missing a declared column is a clear error; and a
//! fused cost's trust is the weakest of its inputs.

use std::collections::BTreeSet;

use analytics::{
    cost_by_requirement, ColumnMapping, Cost, CostError, Dataset, Money, Registry, Source,
    SourceKind, TrustLevel,
};
use okf::types::{OkfRoot, Requirement, Summary};

fn source(id: &str, trust: TrustLevel) -> Source {
    Source {
        id: id.to_string(),
        kind: SourceKind::Structured,
        trust,
        description: format!("{} cost export", id),
    }
}

fn dataset(source: Source, captured_at: &str, csv: &str) -> Dataset {
    Dataset::from_csv_at(source, captured_at.to_string(), csv.as_bytes()).expect("valid CSV parses")
}

fn registry(sources: &[Source]) -> Registry {
    let mut r = Registry::new();
    for s in sources {
        r.register(s.clone()).unwrap();
    }
    r
}

fn model(ids: &[&str]) -> OkfRoot {
    OkfRoot {
        okf: String::new(),
        project: "proj".to_string(),
        exported_at: String::new(),
        summary: Summary::default(),
        structure: Vec::new(),
        interfaces: Vec::new(),
        signals: Vec::new(),
        requirements: ids
            .iter()
            .map(|id| Requirement {
                id: id.to_string(),
                name: String::new(),
                kind: String::new(),
                stereotypes: Vec::new(),
                attributes: Vec::new(),
                documentation: String::new(),
                req_id: String::new(),
                req_text: String::new(),
            })
            .collect(),
        state_machine: None,
        activities: Vec::new(),
        graph: None,
        provenance: None,
    }
}

fn mapping() -> ColumnMapping {
    ColumnMapping::new("requirement_id", "unit_cost")
}

fn set(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|s| s.to_string()).collect()
}

fn money(s: &str) -> Money {
    s.parse().expect("a valid money amount")
}

#[test]
fn an_uncosted_requirement_is_uncosted_and_not_zero_and_still_appears() {
    let m = model(&["REQ-1", "REQ-2"]);
    let erp = source("erp", TrustLevel::Reported);
    let ds = dataset(
        erp.clone(),
        "2026-09-17T00:00:00Z",
        "requirement_id,unit_cost\nREQ-1,100\n",
    );
    let reg = registry(&[erp]);
    let mapping = mapping();

    let result = cost_by_requirement(&[&m], &reg, &[(&ds, &mapping)]).unwrap();

    // REQ-2 has no record, yet it still appears, and it is UNCOSTED - not zero.
    assert_eq!(result.len(), 2);
    assert_eq!(result[1].requirement, "REQ-2");
    assert!(result[1].cost.is_uncosted());
    assert!(matches!(&result[1].cost, Cost::Uncosted));

    // And REQ-1 is costed, proving the two states coexist without a phantom zero.
    match &result[0].cost {
        Cost::Costed(fused) => assert_eq!(fused.value, money("100")),
        other => panic!("REQ-1 should be costed, got {:?}", other),
    }
}

#[test]
fn a_cost_is_attributed_to_its_source_and_date() {
    let m = model(&["REQ-1"]);
    let erp = source("erp", TrustLevel::Reported);
    let ds = dataset(
        erp.clone(),
        "2026-09-17T00:00:00Z",
        "requirement_id,unit_cost\nREQ-1,250\n",
    );
    let reg = registry(&[erp]);
    let mapping = mapping();

    let result = cost_by_requirement(&[&m], &reg, &[(&ds, &mapping)]).unwrap();
    let fused = match &result[0].cost {
        Cost::Costed(fused) => fused,
        other => panic!("expected costed, got {:?}", other),
    };

    assert_eq!(fused.value, money("250"));
    assert_eq!(fused.trust, TrustLevel::Reported);
    assert_eq!(fused.sources, set(&["erp"]));
    assert_eq!(
        fused.captured_at.get("erp").map(String::as_str),
        Some("2026-09-17T00:00:00Z")
    );
}

#[test]
fn a_dataset_missing_a_declared_column_is_a_clear_error() {
    let m = model(&["REQ-1"]);
    let erp = source("erp", TrustLevel::Reported);
    // The header says "amount", but the mapping declares the cost column "unit_cost".
    let ds = dataset(erp.clone(), "t", "requirement_id,amount\nREQ-1,100\n");
    let reg = registry(&[erp]);
    let mapping = mapping();

    match cost_by_requirement(&[&m], &reg, &[(&ds, &mapping)]) {
        Err(CostError::MissingColumn { source, column }) => {
            assert_eq!(source, "erp");
            assert_eq!(column, "unit_cost");
        }
        other => panic!("expected MissingColumn, got {:?}", other),
    }
}

#[test]
fn a_dataset_missing_the_requirement_column_is_also_a_clear_error() {
    let m = model(&["REQ-1"]);
    let erp = source("erp", TrustLevel::Reported);
    // The header names the requirement column "id", not the declared "requirement_id".
    let ds = dataset(erp.clone(), "t", "id,unit_cost\nREQ-1,100\n");
    let reg = registry(&[erp]);
    let mapping = mapping();

    match cost_by_requirement(&[&m], &reg, &[(&ds, &mapping)]) {
        Err(CostError::MissingColumn { source, column }) => {
            assert_eq!(source, "erp");
            assert_eq!(column, "requirement_id");
        }
        other => panic!("expected MissingColumn, got {:?}", other),
    }
}

#[test]
fn a_fused_costs_trust_is_the_weakest_of_its_inputs() {
    let m = model(&["REQ-1"]);
    let erp = source("erp", TrustLevel::Reported);
    let sheet = source("spreadsheet", TrustLevel::Estimated);
    let erp_ds = dataset(
        erp.clone(),
        "2026-01-01",
        "requirement_id,unit_cost\nREQ-1,100\n",
    );
    let sheet_ds = dataset(
        sheet.clone(),
        "2023-01-01",
        "requirement_id,unit_cost\nREQ-1,20\n",
    );
    let reg = registry(&[erp, sheet]);
    let mapping = mapping();

    let result =
        cost_by_requirement(&[&m], &reg, &[(&erp_ds, &mapping), (&sheet_ds, &mapping)]).unwrap();
    let fused = match &result[0].cost {
        Cost::Costed(fused) => fused,
        other => panic!("expected costed, got {:?}", other),
    };

    // One reported and one estimated record: the sum is 120, but the trust is the
    // weakest link - estimated - and both sources are named.
    assert_eq!(fused.value, money("120"));
    assert_eq!(fused.trust, TrustLevel::Estimated);
    assert!(fused.is_estimate());
    assert_eq!(fused.sources, set(&["erp", "spreadsheet"]));
}

#[test]
fn a_decimal_cost_is_read_exactly() {
    let m = model(&["REQ-1"]);
    let erp = source("erp", TrustLevel::Reported);
    let ds = dataset(erp.clone(), "t", "requirement_id,unit_cost\nREQ-1,12.50\n");
    let reg = registry(&[erp]);
    let mapping = mapping();

    let result = cost_by_requirement(&[&m], &reg, &[(&ds, &mapping)]).unwrap();
    let fused = match &result[0].cost {
        Cost::Costed(fused) => fused,
        other => panic!("expected costed, got {:?}", other),
    };
    assert_eq!(fused.value, money("12.5"));
}

#[test]
fn a_negative_cost_is_a_credit() {
    let m = model(&["REQ-1"]);
    let erp = source("erp", TrustLevel::Reported);
    let ds = dataset(erp.clone(), "t", "requirement_id,unit_cost\nREQ-1,-3.25\n");
    let reg = registry(&[erp]);
    let mapping = mapping();

    let result = cost_by_requirement(&[&m], &reg, &[(&ds, &mapping)]).unwrap();
    let fused = match &result[0].cost {
        Cost::Costed(fused) => fused,
        other => panic!("expected costed, got {:?}", other),
    };
    assert_eq!(fused.value, money("-3.25"));
}

#[test]
fn decimal_costs_fuse_exactly() {
    let m = model(&["REQ-1"]);
    let erp = source("erp", TrustLevel::Measured);
    let sheet = source("sheet", TrustLevel::Measured);
    let a = dataset(
        erp.clone(),
        "2026-01-01",
        "requirement_id,unit_cost\nREQ-1,0.1\n",
    );
    let b = dataset(
        sheet.clone(),
        "2026-01-02",
        "requirement_id,unit_cost\nREQ-1,0.2\n",
    );
    let reg = registry(&[erp, sheet]);
    let mapping = mapping();

    let result = cost_by_requirement(&[&m], &reg, &[(&a, &mapping), (&b, &mapping)]).unwrap();
    let fused = match &result[0].cost {
        Cost::Costed(fused) => fused,
        other => panic!("expected costed, got {:?}", other),
    };
    // 0.1 + 0.2 is exactly 0.3, never 0.30000000000000004.
    assert_eq!(fused.value, money("0.3"));
}

#[test]
fn a_cost_that_is_not_a_number_is_a_typed_error() {
    let m = model(&["REQ-1"]);
    let erp = source("erp", TrustLevel::Reported);
    let ds = dataset(erp.clone(), "t", "requirement_id,unit_cost\nREQ-1,free\n");
    let reg = registry(&[erp]);
    let mapping = mapping();

    match cost_by_requirement(&[&m], &reg, &[(&ds, &mapping)]) {
        Err(CostError::MalformedCost {
            source,
            requirement,
            value,
            row,
        }) => {
            assert_eq!(source, "erp");
            assert_eq!(requirement, "REQ-1");
            assert_eq!(value, "free");
            assert_eq!(row, 2);
        }
        other => panic!("expected MalformedCost, got {:?}", other),
    }
}

#[test]
fn a_requirement_in_many_models_appears_once() {
    let a = model(&["REQ-1", "REQ-2"]);
    let b = model(&["REQ-1"]);
    let erp = source("erp", TrustLevel::Reported);
    let ds = dataset(erp.clone(), "t", "requirement_id,unit_cost\nREQ-1,100\n");
    let reg = registry(&[erp]);
    let mapping = mapping();

    let result = cost_by_requirement(&[&a, &b], &reg, &[(&ds, &mapping)]).unwrap();
    assert_eq!(result.len(), 2);
    assert_eq!(result[0].requirement, "REQ-1");
    assert_eq!(result[1].requirement, "REQ-2");
}

#[test]
fn an_empty_model_list_with_no_records_is_an_empty_result() {
    let erp = source("erp", TrustLevel::Reported);
    let ds = dataset(erp.clone(), "t", "requirement_id,unit_cost\n");
    let reg = registry(&[erp]);
    let mapping = mapping();

    let result = cost_by_requirement(&[], &reg, &[(&ds, &mapping)]).unwrap();
    assert!(result.is_empty());
}

#[test]
fn uncosted_serializes_distinctly_from_a_cost() {
    let m = model(&["REQ-1", "REQ-2"]);
    let erp = source("erp", TrustLevel::Reported);
    let ds = dataset(erp.clone(), "t", "requirement_id,unit_cost\nREQ-1,100\n");
    let reg = registry(&[erp]);
    let mapping = mapping();

    let result = cost_by_requirement(&[&m], &reg, &[(&ds, &mapping)]).unwrap();
    let value = serde_json::to_value(&result).expect("serializes");
    // REQ-2 serializes as the string "Uncosted", never as a number, so a consumer
    // cannot read it as a zero.
    assert_eq!(value[1]["cost"], "Uncosted");
    // REQ-1 serializes as an object tagged "Costed".
    assert!(value[0]["cost"].get("Costed").is_some());
}

#[test]
fn the_real_corpus_joins_against_cost_data() {
    let model: OkfRoot =
        serde_json::from_str(&test_support::load_okf_expected()).expect("corpus fixture parses");
    assert_eq!(model.requirements.len(), 25);

    // Price the first two requirements by their real ids; the other 23 stay uncosted.
    let first = model.requirements[0].id.clone();
    let second = model.requirements[1].id.clone();
    let csv = format!(
        "requirement_id,unit_cost\n{},1200\n{},3400\n",
        first, second
    );
    let erp = source("erp", TrustLevel::Reported);
    let ds = dataset(erp.clone(), "2026-09-17T00:00:00Z", &csv);
    let reg = registry(&[erp]);
    let mapping = mapping();

    let result = cost_by_requirement(&[&model], &reg, &[(&ds, &mapping)]).unwrap();
    assert_eq!(result.len(), 25);

    match &result[0].cost {
        Cost::Costed(fused) => assert_eq!(fused.value, money("1200")),
        other => panic!("first corpus requirement should be costed, got {:?}", other),
    }
    match &result[1].cost {
        Cost::Costed(fused) => assert_eq!(fused.value, money("3400")),
        other => panic!(
            "second corpus requirement should be costed, got {:?}",
            other
        ),
    }

    // Every other requirement is uncosted - still present, never zero, never omitted.
    for entry in &result[2..] {
        assert!(
            entry.cost.is_uncosted(),
            "requirement {} should be uncosted",
            entry.requirement
        );
    }
}

#[test]
fn a_dataset_record_for_an_unknown_requirement_is_surfaced_not_dropped() {
    let m = model(&["REQ-1"]);
    let erp = source("erp", TrustLevel::Reported);
    // REQ-2 has a cost record but appears in no model. It must be surfaced, never
    // silently dropped while REQ-1 would otherwise read uncosted.
    let ds = dataset(
        erp.clone(),
        "t",
        "requirement_id,unit_cost\nREQ-1,100\nREQ-2,200\n",
    );
    let reg = registry(&[erp]);
    let mapping = mapping();

    match cost_by_requirement(&[&m], &reg, &[(&ds, &mapping)]) {
        Err(CostError::UnmatchedRequirements { requirements }) => {
            assert_eq!(requirements, vec!["REQ-2".to_string()]);
        }
        other => panic!("expected UnmatchedRequirements, got {:?}", other),
    }
}

#[test]
fn an_empty_model_list_with_records_is_an_error_not_a_silent_drop() {
    let erp = source("erp", TrustLevel::Reported);
    let ds = dataset(erp.clone(), "t", "requirement_id,unit_cost\nREQ-1,100\n");
    let reg = registry(&[erp]);
    let mapping = mapping();

    match cost_by_requirement(&[], &reg, &[(&ds, &mapping)]) {
        Err(CostError::UnmatchedRequirements { requirements }) => {
            assert_eq!(requirements, vec!["REQ-1".to_string()]);
        }
        other => panic!("expected UnmatchedRequirements, got {:?}", other),
    }
}

#[test]
fn a_requirement_cell_is_trimmed_before_joining() {
    let m = model(&["REQ-1"]);
    let erp = source("erp", TrustLevel::Reported);
    // A trailing space on the requirement cell is a near-miss key: it must still
    // join to the model's exact id rather than leaving REQ-1 falsely uncosted.
    let ds = dataset(erp.clone(), "t", "requirement_id,unit_cost\nREQ-1 ,100\n");
    let reg = registry(&[erp]);
    let mapping = mapping();

    let result = cost_by_requirement(&[&m], &reg, &[(&ds, &mapping)]).unwrap();
    assert_eq!(result.len(), 1);
    match &result[0].cost {
        Cost::Costed(fused) => assert_eq!(fused.value, money("100")),
        other => panic!("REQ-1 should be costed, got {:?}", other),
    }
}

#[test]
fn a_query_resolves_trust_from_the_registry_not_the_dataset() {
    let m = model(&["REQ-1"]);
    let mut reg = Registry::new();
    reg.register(source("spreadsheet", TrustLevel::Estimated))
        .unwrap();
    // A fabricated dataset claiming Measured for a source the registry knows as
    // Estimated. The query must CORRECT it to Estimated.
    let fabricated = dataset(
        source("spreadsheet", TrustLevel::Measured),
        "t",
        "requirement_id,unit_cost\nREQ-1,10\n",
    );
    let mapping = mapping();

    let result = cost_by_requirement(&[&m], &reg, &[(&fabricated, &mapping)]).unwrap();
    let fused = match &result[0].cost {
        Cost::Costed(fused) => fused,
        other => panic!("expected costed, got {:?}", other),
    };
    assert_eq!(fused.trust, TrustLevel::Estimated);
    assert!(fused.is_estimate());
}

#[test]
fn an_unregistered_source_cannot_be_queried() {
    let m = model(&["REQ-1"]);
    let ds = dataset(
        source("erp", TrustLevel::Reported),
        "t",
        "requirement_id,unit_cost\nREQ-1,100\n",
    );
    let reg = Registry::new();
    let mapping = mapping();

    match cost_by_requirement(&[&m], &reg, &[(&ds, &mapping)]) {
        Err(CostError::UnregisteredSource { source }) => assert_eq!(source, "erp"),
        other => panic!("expected UnregisteredSource, got {:?}", other),
    }
}

#[test]
fn a_dataset_with_no_read_time_is_refused() {
    let m = model(&["REQ-1"]);
    let erp = source("erp", TrustLevel::Reported);
    let ds = Dataset::from_csv_at(
        erp.clone(),
        String::new(),
        b"requirement_id,unit_cost\nREQ-1,100\n",
    )
    .unwrap();
    let reg = registry(&[erp]);
    let mapping = mapping();

    match cost_by_requirement(&[&m], &reg, &[(&ds, &mapping)]) {
        Err(CostError::UnstampedSource { source }) => assert_eq!(source, "erp"),
        other => panic!("expected UnstampedSource, got {:?}", other),
    }
}

#[test]
fn a_trailing_blank_line_is_skipped_not_an_error() {
    let m = model(&["REQ-1"]);
    let erp = source("erp", TrustLevel::Reported);
    // A trailing blank line parses to a row of empty cells; it is not a record and
    // must not abort the join.
    let ds = dataset(erp.clone(), "t", "requirement_id,unit_cost\nREQ-1,100\n\n");
    let reg = registry(&[erp]);
    let mapping = mapping();

    let result = cost_by_requirement(&[&m], &reg, &[(&ds, &mapping)]).unwrap();
    assert_eq!(result.len(), 1);
    match &result[0].cost {
        Cost::Costed(fused) => assert_eq!(fused.value, money("100")),
        other => panic!("REQ-1 should be costed, got {:?}", other),
    }
}

#[test]
fn a_row_too_short_reports_its_row_number() {
    let m = model(&["REQ-1"]);
    let erp = source("erp", TrustLevel::Reported);
    // The data row has one cell where two are declared.
    let ds = dataset(erp.clone(), "t", "requirement_id,unit_cost\nREQ-1\n");
    let reg = registry(&[erp]);
    let mapping = mapping();

    match cost_by_requirement(&[&m], &reg, &[(&ds, &mapping)]) {
        Err(CostError::MalformedCost {
            source,
            requirement,
            value,
            row,
        }) => {
            assert_eq!(source, "erp");
            assert_eq!(requirement, "REQ-1");
            assert_eq!(value, "");
            assert_eq!(row, 2);
        }
        other => panic!("expected MalformedCost, got {:?}", other),
    }
}
