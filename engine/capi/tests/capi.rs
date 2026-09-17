// SPDX-License-Identifier: AGPL-3.0-or-later
use std::ffi::CStr;

#[test]
fn version_returns_string() {
    let ptr = capi::modelwrite_version();
    let s = unsafe { CStr::from_ptr(ptr) }
        .to_string_lossy()
        .into_owned();
    assert!(s.starts_with("modelwrite "));
}

#[test]
fn validate_returns_json_report() {
    let okf = test_support::load_okf_expected();
    let ptr = unsafe { capi::modelwrite_validate(okf.as_ptr(), okf.len()) };
    let s = unsafe { CStr::from_ptr(ptr) }
        .to_string_lossy()
        .into_owned();
    let v: serde_json::Value = serde_json::from_str(&s).expect("JSON report");
    assert!(v["valid"].as_bool().unwrap());
    unsafe { capi::modelwrite_free_string(ptr) };
}

#[test]
fn gate_returns_evidence() {
    let a = test_support::load_okf_expected();
    let b = a.clone();
    let ptr = unsafe { capi::modelwrite_gate(a.as_ptr(), a.len(), b.as_ptr(), b.len()) };
    let s = unsafe { CStr::from_ptr(ptr) }
        .to_string_lossy()
        .into_owned();
    let v: serde_json::Value = serde_json::from_str(&s).expect("JSON evidence");
    assert!(v["passed"].as_bool().unwrap());
    unsafe { capi::modelwrite_free_string(ptr) };
}

#[test]
fn validate_rejects_garbage() {
    let garbage = "not json";
    let ptr = unsafe { capi::modelwrite_validate(garbage.as_ptr(), garbage.len()) };
    let s = unsafe { CStr::from_ptr(ptr) }
        .to_string_lossy()
        .into_owned();
    let v: serde_json::Value = serde_json::from_str(&s).expect("JSON error report");
    assert!(v["error"].is_string());
    unsafe { capi::modelwrite_free_string(ptr) };
}
