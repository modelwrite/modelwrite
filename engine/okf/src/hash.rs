// SPDX-License-Identifier: AGPL-3.0-or-later
use sha2::{Digest, Sha256};

use crate::types::OkfRoot;

/// Content fingerprint of an OKF snapshot: sha256 over the canonical
/// serialization. Field order is fixed by the type definitions, so a given
/// snapshot always hashes to the same value. It is a fingerprint, not a
/// semantic identity; the gate uses diffing for equality.
pub fn canonical_hash(root: &OkfRoot) -> String {
    let bytes = serde_json::to_vec(root).expect("OkfRoot serialization cannot fail");
    let digest = Sha256::digest(&bytes);
    hex::encode(digest)
}
