// SPDX-License-Identifier: AGPL-3.0-or-later
//! The fusion rule, exercised through the public surface: a fused value carries
//! the weakest trust among its inputs, names every contributing source and which
//! of them was the weak one, refuses to fuse nothing, and keeps each source's read
//! time. Trust is resolved from the registry, never self-attested.

use std::collections::BTreeMap;
use std::collections::BTreeSet;

use analytics::confidence::{fused_trust, max, min, sum, FusedValue, FusionError, Value};
use analytics::{Registry, RegistryError, Source, SourceKind, TrustLevel};

fn registry(sources: &[(&str, TrustLevel)]) -> Registry {
    let mut r = Registry::new();
    for (id, trust) in sources {
        r.register(Source {
            id: id.to_string(),
            kind: SourceKind::Structured,
            trust: *trust,
            description: String::new(),
        })
        .unwrap();
    }
    r
}

fn value(v: i64, source: &str, at: &str, registry: &Registry) -> Value<i64> {
    Value::from_registry(v, source, at, registry).unwrap()
}

fn set(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|s| s.to_string()).collect()
}

#[test]
fn a_sum_of_one_measured_and_one_estimated_value_is_estimated() {
    let r = registry(&[
        ("contract", TrustLevel::Measured),
        ("spreadsheet", TrustLevel::Estimated),
    ]);
    let a = value(10, "contract", "2026-01-01", &r);
    let b = value(20, "spreadsheet", "2023-01-01", &r);
    let fused = sum(&[a, b]).unwrap();
    assert_eq!(fused.value, 30);
    assert_eq!(fused.trust, TrustLevel::Estimated);
    assert!(fused.is_estimate());
}

#[test]
fn a_sum_of_measured_values_is_measured() {
    let r = registry(&[
        ("contract", TrustLevel::Measured),
        ("erp", TrustLevel::Measured),
    ]);
    let a = value(10, "contract", "2026-01-01", &r);
    let b = value(20, "erp", "2026-01-02", &r);
    let fused = sum(&[a, b]).unwrap();
    assert_eq!(fused.value, 30);
    assert_eq!(fused.trust, TrustLevel::Measured);
    assert!(!fused.is_estimate());
}

#[test]
fn reported_is_weaker_than_measured() {
    let r = registry(&[
        ("contract", TrustLevel::Measured),
        ("erp", TrustLevel::Reported),
    ]);
    let a = value(1, "contract", "2026-01-01", &r);
    let b = value(2, "erp", "2026-01-02", &r);
    let fused = sum(&[a, b]).unwrap();
    assert_eq!(fused.trust, TrustLevel::Reported);
    assert!(!fused.is_estimate());
}

#[test]
fn a_majority_of_measured_values_does_not_mask_one_estimate() {
    let r = registry(&[
        ("contract", TrustLevel::Measured),
        ("erp", TrustLevel::Measured),
        ("plm", TrustLevel::Measured),
        ("spreadsheet", TrustLevel::Estimated),
    ]);
    let values = [
        value(10, "contract", "2026-01-01", &r),
        value(10, "erp", "2026-01-02", &r),
        value(10, "plm", "2026-01-03", &r),
        value(1, "spreadsheet", "2023-01-01", &r),
    ];
    let fused = sum(&values).unwrap();
    assert_eq!(fused.trust, TrustLevel::Estimated);
    assert!(fused.is_estimate());
}

#[test]
fn fused_trust_is_the_weakest_of_its_inputs() {
    let r = registry(&[
        ("a", TrustLevel::Measured),
        ("b", TrustLevel::Reported),
        ("c", TrustLevel::Estimated),
    ]);
    let values = [
        value(1, "a", "t", &r),
        value(2, "b", "t", &r),
        value(3, "c", "t", &r),
    ];
    assert_eq!(fused_trust(&values), Some(TrustLevel::Estimated));
    assert_eq!(fused_trust(&values[..2]), Some(TrustLevel::Reported));
    assert_eq!(fused_trust(&values[..1]), Some(TrustLevel::Measured));
    assert_eq!(fused_trust(&values[..0]), None);
}

#[test]
fn every_fused_value_names_all_its_sources() {
    let r = registry(&[
        ("contract", TrustLevel::Measured),
        ("spreadsheet", TrustLevel::Estimated),
        ("erp", TrustLevel::Reported),
    ]);
    let a = value(10, "contract", "2026-01-01", &r);
    let b = value(20, "spreadsheet", "2023-01-01", &r);
    let c = value(5, "erp", "2025-06-01", &r);
    let fused = sum(&[a, b, c]).unwrap();
    assert_eq!(fused.sources, set(&["contract", "erp", "spreadsheet"]));
    assert_eq!(fused.sources.len(), 3);
}

#[test]
fn a_fused_value_names_which_source_was_the_weak_one() {
    let r = registry(&[
        ("contract", TrustLevel::Measured),
        ("spreadsheet", TrustLevel::Estimated),
    ]);
    let a = value(10, "contract", "2026-01-01", &r);
    let b = value(20, "spreadsheet", "2023-01-01", &r);
    let fused = sum(&[a, b]).unwrap();
    assert_eq!(fused.trust, TrustLevel::Estimated);
    assert_eq!(fused.trusts.get("contract"), Some(&TrustLevel::Measured));
    assert_eq!(
        fused.trusts.get("spreadsheet"),
        Some(&TrustLevel::Estimated)
    );
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
    let r = registry(&[
        ("contract", TrustLevel::Measured),
        ("spreadsheet", TrustLevel::Estimated),
    ]);
    let a = value(10, "contract", "2026-01-01", &r);
    let b = value(20, "spreadsheet", "2023-01-01", &r);
    let fused = sum(&[a, b]).unwrap();

    let mut expected: BTreeMap<String, String> = BTreeMap::new();
    expected.insert("contract".to_string(), "2026-01-01".to_string());
    expected.insert("spreadsheet".to_string(), "2023-01-01".to_string());
    assert_eq!(fused.captured_at, expected);
}

#[test]
fn max_and_min_also_carry_the_weakest_trust_and_all_sources() {
    let r = registry(&[
        ("contract", TrustLevel::Measured),
        ("spreadsheet", TrustLevel::Estimated),
    ]);
    let a = value(100, "contract", "2026-01-01", &r);
    let b = value(40, "spreadsheet", "2023-01-01", &r);

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
    let r = registry(&[("contract", TrustLevel::Measured)]);
    let v = value(42, "contract", "2026-01-01", &r);
    let fused: FusedValue<i64> = sum(std::slice::from_ref(&v)).unwrap();
    assert_eq!(fused.value, 42);
    assert_eq!(fused.trust, TrustLevel::Measured);
    assert_eq!(fused.sources, set(&["contract"]));
    assert_eq!(
        fused.captured_at.get("contract").map(String::as_str),
        Some("2026-01-01")
    );
    assert_eq!(fused.trusts.get("contract"), Some(&TrustLevel::Measured));
}

#[test]
fn a_value_cannot_be_built_for_an_unregistered_source() {
    // The registry is the only place a source's trust is declared; a value for an
    // unregistered source has no trust level and is refused outright.
    let empty = Registry::new();
    match Value::from_registry(1, "ghost", "t", &empty) {
        Err(RegistryError::UnregisteredSource(id)) => assert_eq!(id, "ghost"),
        other => panic!("expected UnregisteredSource, got {:?}", other),
    }
}

#[test]
fn a_value_reads_the_trust_the_registry_fixed_not_the_callers_intent() {
    // A caller supplies only a source id; whatever trust it might wish for, the
    // value carries the registry's trust. There is no constructor that accepts a
    // trust level, so the only trust a value can ever carry is the registered one.
    let r = registry(&[("spreadsheet", TrustLevel::Estimated)]);
    let v = value(5, "spreadsheet", "t", &r);
    assert_eq!(v.trust(), TrustLevel::Estimated);
    assert!(v.is_estimate());
}
