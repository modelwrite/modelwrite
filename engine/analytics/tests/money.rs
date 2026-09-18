// SPDX-License-Identifier: AGPL-3.0-or-later
//! The money type, exercised directly: exact decimal arithmetic with no floating
//! point, and the proof that a binary float fails exactly where it must not.

use analytics::Money;

#[test]
fn one_tenth_plus_two_tenths_is_exactly_three_tenths() {
    let a: Money = "0.1".parse().unwrap();
    let b: Money = "0.2".parse().unwrap();
    let expected: Money = "0.3".parse().unwrap();
    assert_eq!(a + b, expected);
    assert_eq!((a + b).to_string(), "0.3");
}

#[test]
fn a_binary_float_is_not_exact_where_money_is() {
    // The reason the type exists: 0.1 + 0.2 in a binary float drifts off 0.3 to
    // 0.30000000000000004. Money is exact here; a float is not.
    let drifted = (0.1_f64 + 0.2_f64) - 0.3_f64;
    assert!(
        drifted.abs() > 0.0,
        "a binary float is expected to drift off 0.3"
    );
}

#[test]
fn decimal_whole_and_negative_amounts_all_parse() {
    assert_eq!(
        "12.50".parse::<Money>().unwrap(),
        "12.5".parse::<Money>().unwrap()
    );
    assert_eq!(
        "12".parse::<Money>().unwrap(),
        "12".parse::<Money>().unwrap()
    );
    assert_eq!(
        "-3.25".parse::<Money>().unwrap(),
        "-3.25".parse::<Money>().unwrap()
    );
}

#[test]
fn amounts_are_scaled_integers_not_floats() {
    let m: Money = "12.50".parse().unwrap();
    assert_eq!(m.minor_units(), 125);
    assert_eq!(m.scale(), 1);
}

#[test]
fn amounts_format_back_to_their_canonical_decimal() {
    assert_eq!("12.5".parse::<Money>().unwrap().to_string(), "12.5");
    assert_eq!("12".parse::<Money>().unwrap().to_string(), "12");
    assert_eq!("-3.25".parse::<Money>().unwrap().to_string(), "-3.25");
    assert_eq!("0.05".parse::<Money>().unwrap().to_string(), "0.05");
}

#[test]
fn ordering_is_value_ordering_not_scale_ordering() {
    assert!("0.9".parse::<Money>().unwrap() < "1.0".parse::<Money>().unwrap());
    assert!("1.5".parse::<Money>().unwrap() > "1.05".parse::<Money>().unwrap());
    assert!("-3.25".parse::<Money>().unwrap() < "0".parse::<Money>().unwrap());
}

#[test]
fn equality_is_value_equality_across_encodings() {
    assert_eq!(
        "1".parse::<Money>().unwrap(),
        "1.0".parse::<Money>().unwrap()
    );
    assert_eq!(
        "0".parse::<Money>().unwrap(),
        "0.00".parse::<Money>().unwrap()
    );
}

#[test]
fn garbage_is_not_money() {
    for bad in [
        "",
        "   ",
        "free",
        "$12.50",
        "1,000",
        "12.",
        ".",
        "1.2.3",
        "12abc",
        "0.0000000001",
    ] {
        assert!(
            bad.parse::<Money>().is_err(),
            "expected {:?} to be refused",
            bad
        );
    }
}

#[test]
fn money_serializes_as_an_exact_decimal_string() {
    let m: Money = "12.5".parse().unwrap();
    let json = serde_json::to_string(&m).unwrap();
    assert_eq!(json, "\"12.5\"");
    let back: Money = serde_json::from_str(&json).unwrap();
    assert_eq!(back, m);
}
