// SPDX-License-Identifier: AGPL-3.0-or-later
//! The audit-action vocabulary. Every action name a write path can record lives here as a
//! single constant, so a page and an API handler cannot record different names for the same
//! event, and a rename is one edit that the compiler finds every user of. The names are the
//! wire contract of the audit log: a reader depends on \`commit.create\` meaning exactly one
//! thing wherever it appears.

pub const PROJECT_CREATE: &str = "project.create";
pub const COMMIT_CREATE: &str = "commit.create";
pub const COMMIT_REFUSED: &str = "commit.refused";
pub const BRANCH_CREATE: &str = "branch.create";
pub const BRANCH_DELETE: &str = "branch.delete";
pub const BRANCH_RESET: &str = "branch.reset";
pub const MERGE_CLEAN: &str = "merge.clean";
pub const MERGE_CONFLICT: &str = "merge.conflict";
pub const LOCK_ACQUIRE: &str = "lock.acquire";
pub const LOCK_RELEASE: &str = "lock.release";
pub const LOCK_DENIED: &str = "lock.denied";
pub const GATE_RUN: &str = "gate.run";
pub const IMPORT_ACCEPT: &str = "import.accept";
pub const IMPORT_REFUSED: &str = "import.refused";
pub const PROPOSAL_RECORD: &str = "proposal.record";
pub const PROPOSAL_ACCEPT: &str = "proposal.accept";
pub const PROPOSAL_REFUSED: &str = "proposal.refused";
