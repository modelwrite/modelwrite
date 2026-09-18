// SPDX-License-Identifier: AGPL-3.0-or-later
//! The fusion rule, exercised through the public surface: a fused value carries
//! the weakest trust among its inputs, names every contributing source, refuses to
//! fuse nothing, and keeps each source's read time.

use std::collections::BTreeMap;
use std::collections::BTreeSet;

use analytics::confidence::{fused_trust, max, min, sum, FusedValue, FusionError, Value};
use analytics::TrustLevel;

fn measured(v: i64, source: &str, at: &str) -> Value<i64> {
    Value::new(v, source, TrustLevel::Measured, at)
}

fn estimated(v: i64, source: &str, at: &str) -> Value<i64> {
    Value::new(v, source, TrustLevel::Estimated, at)
}

fn set(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|s| s.to_string()).collect()
}

#[test]
fn a_sum_of_one_measured_and_one_estimated_value_is_estimated() {
    let a = measured(10, "contract", "2026-01-01");
    let b = estimated(20, "spreadsheet", "2023-01-01");
    let fused = sum(&[a, b]).unwrap();
    assert_eq!(fused.value, 30);
    assert_eq!(fused.trust, TrustLevel::Estimated);
    assert!(fused.is_estimate());
}

#[test]
fn a_sum_of_measured_values_is_measured() {
    let a = measured(10, "contract", "2026-01-01");
    let b = measured(20, "erp", "2026-01-02");
    let fused = sum(&[a, b]).unwrap();
    assert_eq!(fused.value, 30);
    assert_eq!(fused.trust, TrustLevel::Measured);
    assert!(!fused.is_estimate());
}

#[test]
fn reported_is_weaker_than_measured() {
    let a = measured(1, "contract", "2026-01-01");
    let b = Value::new(2, "erp", TrustLevel::Reported, "2026-01-02");
    let fused = sum(&[a, b]).unwrap();
    assert_eq!(fused.trust, TrustLevel::Reported);
    assert!(!fused.is_estimate());
}

#[test]
fn a_majority_of_measured_values_does_not_mask_one_estimate() {
    let values = [
        measured(10, "contract", "2026-01-01"),
        measured(10, "erp", "2026-01-02"),
        measured(10, "plm", "2026-01-03"),
        estimated(1, "spreadsheet", "2023-01-01"),
    ];
    let fused = sum(&values).unwrap();
    assert_eq!(fused.trust, TrustLevel::Estimated);
    assert!(fused.is_estimate());
}

#[test]
fn fused_trust_is_the_weakest_of_its_inputs() {
    let values = [
        measured(1, "a", "t"),
        Value::new(2, "b", TrustLevel::Reported, "t"),
        estimated(3, "c", "t"),
    ];
    assert_eq!(fused_trust(&values), Some(TrustLevel::Estimated));
    assert_eq!(fused_trust(&values[..2]), Some(TrustLevel::Reported));
    assert_eq!(fused_trust(&values[..1]), Some(TrustLevel::Measured));
    assert_eq!(fused_trust(&values[..0]), None);
}

#[test]
fn every_fused_value_names_all_its_sources() {
    let a = measured(10, "contract", "2026-01-01");
    let b = estimated(20, "spreadsheet", "2023-01-01");
    let c = Value::new(5, "erp", TrustLevel::Reported, "2025-06-01");
    let fused = sum(&[a, b, c]).unwrap();
    assert_eq!(fused.sources, set(&["contract", "erp", "spreadsheet"]));
    assert_eq!(fused.sources.len(), 3);
}

#[test]
fn a_fused_value_with_no_sources_is_an_error_not_a_zero() {
    let empty: [Value<i64>; 0] = [];
    match sum(&empty) {
        Err(FusionError::NoSources) => {}
        Ok(fused) => panic!("expected NoSources, got a fused value: {:?}", fused),
    }
}

#[test]
fn captured_at_survives_fusion() {
    let a = measured(10, "contract", "2026-01-01");
    let b = estimated(20, "spreadsheet", "2023-01-01");
    let fused = sum(&[a, b]).unwrap();

    let mut expected: BTreeMap<String, String> = BTreeMap::new();
    expected.insert("contract".to_string(), "2026-01-01".to_string());
    expected.insert("spreadsheet".to_string(), "2023-01-01".to_string());
    assert_eq!(fused.captured_at, expected);
}

#[test]
fn max_and_min_also_carry_the_weakest_trust_and_all_sources() {
    let a = measured(100, "contract", "2026-01-01");
    let b = estimated(40, "spreadsheet", "2023-01-01");

    let biggest = max(&[a.clone(), b.clone()]).unwrap();
    assert_eq!(biggest.value, 100);
    assert_eq!(biggest.trust, TrustLevel::Estimated);
    assert_eq!(biggest.sources, set(&["contract", "spreadsheet"]));

    let smallest = min(&[a, b]).unwrap();
    assert_eq!(smallest.value, 40);
    assert_eq!(smallest.trust, TrustLevel::Estimated);
    assert_eq!(smallest.sources, set(&["contract", "spreadsheet"]));
}

#[test]
fn comparison_over_nothing_is_also_an_error() {
    let empty: [Value<i64>; 0] = [];
    assert!(matches!(max(&empty), Err(FusionError::NoSources)));
    assert!(matches!(min(&empty), Err(FusionError::NoSources)));
}

#[test]
fn a_single_value_fuses_with_its_own_trust_source_and_time() {
    let v = measured(42, "contract", "2026-01-01");
    let fused: FusedValue<i64> = sum(std::slice::from_ref(&v)).unwrap();
    assert_eq!(fused.value, 42);
    assert_eq!(fused.trust, TrustLevel::Measured);
    assert_eq!(fused.sources, set(&["contract"]));
    assert_eq!(
        fused.captured_at.get("contract").map(String::as_str),
        Some("2026-01-01")
    );
}
