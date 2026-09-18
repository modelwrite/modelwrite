// SPDX-License-Identifier: AGPL-3.0-or-later
//! The binding registry: the one place that maps a binding id and version to an
//! implementation of the `Binding` trait. A new standard is a new arm here, not a new
//! code path anywhere else in the service.

use binding::Binding;

/// Resolve a binding id and version to its implementation. Matching is exact: an unknown
/// binding is `None`, which the caller reports as a clear 400 rather than a 500, because a
/// binding that does not exist is a request problem, not a server fault.
pub fn resolve(id: &str, version: &str) -> Option<Box<dyn Binding>> {
    match (id, version) {
        (binding_xmi::BINDING_ID, binding_xmi::BINDING_VERSION) => {
            Some(Box::new(binding_xmi::XmiBinding::new()))
        }
        _ => None,
    }
}
