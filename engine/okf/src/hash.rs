// SPDX-License-Identifier: AGPL-3.0-or-later
use sha2::{Digest, Sha256};

use crate::types::OkfRoot;

/// The ONE canonical serialization of an OKF document: the summary re-derived from the
/// document's own content, serialized in the fixed field order of the type definitions.
///
/// Every content address of a document - the commit's okf_hash, the gate's reference and
/// candidate hashes, the compliance model hash - is sha256 over THESE bytes. The summary is
/// part of the canonical form (it is derived, never trusted), and the field order is fixed by
/// the struct definitions, so the same logical document hashes identically no matter which
/// code path wrote it: struct-order re-serialisation, a serde_json::Value in BTreeMap order,
/// raw file bytes, or a binding round trip. Nothing a caller does with key order can change
/// the hash, because the bytes are produced here from the typed document alone.
pub fn canonical_bytes(root: &OkfRoot) -> Vec<u8> {
    let mut canonical = root.clone();
    crate::summary::recompute(&mut canonical);
    serde_json::to_vec(&canonical).expect("OkfRoot serialization cannot fail")
}

/// Content fingerprint of an OKF snapshot: sha256 over the canonical serialization. It is a
/// fingerprint of CONTENT, not of any particular writer: the summary is re-derived and the
/// field order is fixed, so two writers serialising the same document two different ways still
/// agree. It is a fingerprint, not a semantic identity; the gate uses diffing for equality.
pub fn canonical_hash(root: &OkfRoot) -> String {
    let digest = Sha256::digest(canonical_bytes(root));
    hex::encode(digest)
}
