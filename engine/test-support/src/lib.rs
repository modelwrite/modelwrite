// SPDX-License-Identifier: AGPL-3.0-or-later
use std::path::PathBuf;

/// The repository root, two levels above this crate (engine/test-support).
pub fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

pub fn okf_expected() -> PathBuf {
    repo_root().join("sample/corpus/coffee-machine/okf/expected/coffee_machine_model.json")
}

pub fn okf_broken() -> PathBuf {
    repo_root().join("sample/corpus/coffee-machine/okf/corrupted/coffee_machine_model.json")
}

pub fn load_okf_expected() -> String {
    std::fs::read_to_string(okf_expected()).expect("corpus fixture must exist")
}

pub fn load_okf_broken() -> String {
    std::fs::read_to_string(okf_broken()).expect("corrupted fixture must exist")
}
