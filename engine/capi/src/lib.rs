// SPDX-License-Identifier: AGPL-3.0-or-later
//! C ABI for the modelwrite engine. All returned strings are owned by the
//! caller and must be released with modelwrite_free_string.
use std::ffi::{c_char, CString};

/// Hand a payload to the caller without any panic path. A panic unwinding across an
/// `extern "C"` boundary is undefined behaviour, so `expect` is not acceptable here
/// even where the current payloads provably cannot fail.
fn into_c_string(payload: String) -> *mut c_char {
    match CString::new(payload) {
        Ok(s) => s.into_raw(),
        // Unreachable in practice: serde_json escapes control characters, so a
        // serialized payload contains no NUL. Null is returned rather than panicking.
        Err(_) => std::ptr::null_mut(),
    }
}

fn err_string(message: &str) -> *mut c_char {
    into_c_string(serde_json::json!({ "error": message }).to_string())
}

/// Version string; deliberately leaked, never freed.
#[no_mangle]
pub extern "C" fn modelwrite_version() -> *const c_char {
    into_c_string(format!("modelwrite {}", env!("CARGO_PKG_VERSION"))) as *const c_char
}

/// Validate one OKF document; returns a JSON ValidationReport string.
///
/// # Safety
/// `json` must point to `len` readable bytes and remain valid for the call.
#[no_mangle]
pub unsafe extern "C" fn modelwrite_validate(json: *const u8, len: usize) -> *mut c_char {
    if json.is_null() || len == 0 {
        return err_string("empty input");
    }
    let bytes = unsafe { std::slice::from_raw_parts(json, len) };
    let text = match std::str::from_utf8(bytes) {
        Ok(t) => t,
        Err(_) => return err_string("input is not UTF-8"),
    };
    let root = match serde_json::from_str::<okf::types::OkfRoot>(text) {
        Ok(r) => r,
        Err(e) => return err_string(&format!("parse error: {}", e)),
    };
    let report = okf::validate::validate(&root);
    match serde_json::to_string(&report) {
        Ok(payload) => into_c_string(payload),
        Err(e) => err_string(&format!("failed to serialize the validation report: {}", e)),
    }
}

/// Run the round-trip gate; returns the JSON evidence string.
///
/// # Safety
/// `reference` and `candidate` must each point to their corresponding
/// length of readable bytes and remain valid for the call.
#[no_mangle]
pub unsafe extern "C" fn modelwrite_gate(
    reference: *const u8,
    reference_len: usize,
    candidate: *const u8,
    candidate_len: usize,
) -> *mut c_char {
    if reference.is_null() || reference_len == 0 || candidate.is_null() || candidate_len == 0 {
        return err_string("empty input");
    }
    let parse = |ptr: *const u8, len: usize| -> Option<okf::types::OkfRoot> {
        let bytes = unsafe { std::slice::from_raw_parts(ptr, len) };
        let text = std::str::from_utf8(bytes).ok()?;
        serde_json::from_str(text).ok()
    };
    let (Some(reference), Some(candidate)) = (
        parse(reference, reference_len),
        parse(candidate, candidate_len),
    ) else {
        return err_string("parse error: inputs must be OKF JSON documents");
    };
    let outcome = gate::run(&reference, &candidate, false);
    match serde_json::to_string(&outcome.evidence) {
        Ok(payload) => into_c_string(payload),
        Err(e) => err_string(&format!("failed to serialize the gate evidence: {}", e)),
    }
}

/// Free a string returned by this library.
///
/// # Safety
/// `ptr` must be null or a pointer previously returned by this library.
#[no_mangle]
pub unsafe extern "C" fn modelwrite_free_string(ptr: *mut c_char) {
    if !ptr.is_null() {
        drop(unsafe { CString::from_raw(ptr) });
    }
}
