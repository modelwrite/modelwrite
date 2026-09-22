// SPDX-License-Identifier: AGPL-3.0-or-later
//! The binding registry: the one place that maps a binding id and version to an
//! implementation of the Binding trait. A new standard is a new arm here, not a new
//! code path anywhere else in the service.

use binding::{Binding, BindingInfo};

/// Every binding the registry knows, as its declared BindingInfo, in registry order. The
/// import page renders this list as the choice a person makes, and resolve turns a chosen
/// id@version back into an implementation. Both read the same list, so a binding can never
/// be listed but unresolvable, or resolvable but unlisted.
pub fn bindings() -> Vec<BindingInfo> {
    implementations()
        .iter()
        .map(|binding| binding.info())
        .collect()
}

/// The concrete bindings, in registry order. resolve and bindings both read this, so a new
/// standard is added in exactly one place.
fn implementations() -> Vec<Box<dyn Binding>> {
    vec![
        Box::new(binding_xmi::XmiBinding::new()),
        Box::new(binding_sysmlv2::SysmlV2Binding::new()),
        // The Capella/Arcadia reader, a viewer like the SysML v2 one: it imports a
        // `.capella` semantic model into OKF and declares Direction::ImportOnly, so the
        // import page states the viewer direction and no round trip is measured for it.
        Box::new(binding_capella::CapellaBinding::new()),
    ]
}

/// Resolve a binding id and version to its implementation. Matching is exact: an unknown
/// binding is None, which the caller reports as a clear 400 rather than a 500, because a
/// binding that does not exist is a request problem, not a server fault.
pub fn resolve(id: &str, version: &str) -> Option<Box<dyn Binding>> {
    implementations().into_iter().find(|binding| {
        let info = binding.info();
        info.id == id && info.version == version
    })
}
