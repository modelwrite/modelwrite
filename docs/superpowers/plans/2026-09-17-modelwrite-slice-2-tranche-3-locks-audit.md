# Slice 2 (tranche 3) - Locks and the append-only audit log

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (- [ ]) syntax for tracking.

**Goal:** make concurrent editing safe and every action attributable. Element locks stop two people editing the same element without knowing; the audit log records who did what, when, and under which lock, and can never be edited or deleted.

**Architecture:** both features are store-level tables plus thin endpoints. Locks are leases with an expiry, so a crashed client cannot hold an element forever. The audit log is append-only by construction: the store exposes only an append operation and a read, and a test proves no update or delete path exists.

**Why this tranche, and why now:** tranche 2 made concurrent commits safe (the store refuses a merge whose tip moved) but left two gaps the whole-tranche review named: nothing stops two people editing the same element and overwriting each other through separate branches, and nothing records who moved a branch pointer, merged, reverted or ran a gate. An organisation cannot adopt a system of record whose history is unattributable.

**Tech Stack:** unchanged - Rust stable (edition 2021, rust-version 1.75), axum 0.7, tokio, rusqlite 0.31 bundled, serde/serde_json, tower and tempfile for tests.

**Spec:** docs/superpowers/specs/2026-09-17-modelwrite-platform-design.md (sections 4.2, 4.3, 4.6).

## Global Constraints

- Rust stable; edition 2021; rust-version 1.75; no new dependencies.
- Every source file begins with: // SPDX-License-Identifier: AGPL-3.0-or-later
- Cargo is not on PATH in fresh shells: begin every shell command sequence with $env:Path = "$env:USERPROFILE\\.cargo\\bin;" + $env:Path
- Use --no-fail-fast when running a test suite.
- Stage and commit only the paths a task owns. Never amend or rewrite a commit.
- History stays append-only: no operation deletes or rewrites a commit row.
- THE AUDIT LOG IS APPEND-ONLY: the Store exposes an append and a read, never an update or a delete, and a test must prove that a second append cannot alter an earlier entry.
- A lock is a LEASE: it carries an expiry, and an expired lock is available to anyone. A crashed client must never be able to block an element forever.
- Time is passed IN to the store, never read inside it, so lock expiry and audit timestamps are testable without sleeping. The store takes a now: i64 parameter; the HTTP layer supplies the real clock.
- Every request path returns a structured error and never panics: 400 malformed, 404 missing, 409 held by someone else or conflict, 422 invalid model, 500 internal with a generic message.
- Names (project, branch) are validated by api::validate_name before use.
- Tests drive the router in process with tower oneshot; no port is bound.

---

## Task 1: Element locks with leases

**Files:**
- Modify: server/src/store/mod.rs (Lock type, four trait methods)
- Modify: server/src/store/sqlite.rs (the locks table, the four methods)
- Create: server/src/locks_api.rs
- Modify: server/src/lib.rs (pub mod locks_api, three routes)
- Test: server/tests/locks.rs

**Interfaces:**
- Consumes: Store, ApiState, validate_name, map_store_error, now_epoch.
- Produces:
  - `pub struct Lock { pub id: String, pub project: String, pub branch: String, pub element: String, pub holder: String, pub acquired_at: i64, pub expires_at: i64 }`, serialised camelCase.
  - `Store::acquire_locks(&self, project: &str, branch: &str, elements: &[String], holder: &str, ttl_seconds: i64, now: i64) -> Result<Vec<Lock>, StoreError>` - all or nothing: if ANY element is held by another live lease, nothing is acquired and the error is Conflict naming the holder. Re-acquiring an element the same holder already holds EXTENDS the lease rather than failing.
  - `Store::release_locks(&self, project: &str, holder: &str, ids: &[String]) -> Result<usize, StoreError>` - only the holder may release; returns how many were released.
  - `Store::locks(&self, project: &str, now: i64) -> Result<Vec<Lock>, StoreError>` - live locks only; expired rows are reported as gone (and may be swept).
  - `Store::holders_of(&self, project: &str, elements: &[String], now: i64) -> Result<Vec<Lock>, StoreError>` - the live locks covering any of those elements, used by the commit path in Task 2.
  - `POST /projects/:project/locks` {"branch", "elements": [...], "holder", "ttlSeconds"} -> 201 [Lock] or 409.
  - `DELETE /projects/:project/locks` {"holder", "ids": [...]} -> 200 {"released": n}; a DELETE with a body is unusual, so ALSO accept `POST /projects/:project/locks/release` with the same body, which is what the CLI and agents use.
  - `GET /projects/:project/locks` -> 200 [Lock], live only.

**Steps:**

- [ ] **Step 1: Add the locks table to the schema** in server/src/store/sqlite.rs, next to the existing tables:

```sql
CREATE TABLE IF NOT EXISTS locks (
    id TEXT PRIMARY KEY,
    project TEXT NOT NULL,
    branch TEXT NOT NULL,
    element TEXT NOT NULL,
    holder TEXT NOT NULL,
    acquired_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS locks_element ON locks(project, element);
```

- [ ] **Step 2: Add the four trait methods** to server/src/store/mod.rs with the interfaces above, and implement them in sqlite.rs inside a single transaction each. The id is a hash of project, branch, element and holder plus a random-free counter derived from now, so it is deterministic in tests: `format!("{:x}", Sha256::digest(format!("{}|{}|{}|{}|{}", project, branch, element, holder, now)))` truncated to 32 hex characters. Reuse the crate's existing sha2 and hex dependencies.

- [ ] **Step 3: Implement acquire_locks** so that, inside one transaction, it first deletes rows whose expires_at <= now (sweeping dead leases), then checks for any remaining live lease on the requested elements held by a DIFFERENT holder, returning Conflict with the holder and expiry if found, then inserts or updates the rows for this holder. **All or nothing**: on conflict nothing is inserted.

- [ ] **Step 4: Write server/src/locks_api.rs** with the three handlers, following the gate_api.rs pattern for state, error mapping and validation. `ttlSeconds` is validated to be between 30 and 86400, so a lease cannot be infinite or uselessly short. `elements` must be non-empty and each name validated with validate_name.

- [ ] **Step 5: Add the routes** in server/src/lib.rs:

```rust
        .route(
            "/projects/:project/locks",
            post(locks_api::acquire_locks).get(locks_api::list_locks),
        )
        .route("/projects/:project/locks/release", post(locks_api::release_locks))
```

- [ ] **Step 6: Write server/tests/locks.rs** with these tests, each using its own temporary store:

1. `a_lock_can_be_acquired_listed_and_released` - acquire two elements, assert both appear in GET, release by id, assert GET is empty and the released count is 2.
2. `a_second_holder_is_refused_and_nothing_is_acquired` - holder A locks b1; holder B requests [b1, b2] and gets 409; GET shows ONLY A's lock, proving the request was all-or-nothing rather than partially applied.
3. `the_same_holder_can_extend_its_own_lease` - A locks b1, then A locks b1 again with a longer ttl and gets 201, and exactly one lock exists for b1.
4. `an_expired_lease_stops_blocking` - this is the one that needs the injected clock: acquire with a store-level call at now = 1000 and ttl 30, assert a request at now = 1000 is refused, then assert a request at now = 1031 succeeds. Drive it through the store directly, because the HTTP layer always passes the real clock.
5. `only_the_holder_can_release` - A locks b1, B tries to release A's id and releases zero; A's lock is still listed.

- [ ] **Step 7: Run, lint and commit**

```powershell
cargo test -p mw-server --no-fail-fast
cargo fmt --all
cargo clippy -p mw-server --all-targets -- -D warnings
git add server
git commit -m "feat: add element locks with leases"
```

Expected: 47 service tests pass (42 plus the five lock tests).

---

## Task 2: A commit refuses to change an element someone else has locked

**Files:**
- Modify: server/src/api.rs (create_commit: optional lock assertion)
- Modify: server/src/store/mod.rs and sqlite.rs (commit_model gains an optional allowed-holder check)
- Test: server/tests/locks.rs (two more tests)

**Interfaces:**
- Consumes: Store::holders_of, okf::diff (to find which elements a commit touches).
- Produces:
  - `CreateCommit` gains `pub holder: Option<String>`.
  - When `holder` is supplied, create_commit computes the elements the commit CHANGES - the ids present in the diff between the branch tip's model and the incoming model, plus the ids of elements added or removed, plus the endpoints of changed edges - and refuses with 409 if any of them is locked by a different holder. When `holder` is absent the commit behaves exactly as today, so this is opt-in and nothing existing changes.

**Steps:**

- [ ] **Step 1: Compute the touched element ids.** Add a helper in api.rs:

```rust
/// The elements a commit changes: everything the diff reports for any section, plus the
/// endpoints of changed edges. A lock protects an element from being CHANGED, so an
/// untouched element elsewhere in the document does not block the commit.
fn touched_elements(reference: &okf::types::OkfRoot, candidate: &okf::types::OkfRoot) -> Vec<String> {
    let report = okf::diff::diff(reference, candidate);
    let mut ids: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for change in report.changes {
        // The engine already keys changes as "<section>:<id>"; the id is what a lock names.
        if let Some((_, id)) = change.key.split_once(':') {
            ids.insert(id.trim_matches('"').to_string());
        }
    }
    ids.into_iter().collect()
}
```

- [ ] **Step 2: Assert the locks** in create_commit, after validation and before put_blob:

```rust
    if let Some(holder) = body.holder.as_deref() {
        let tip_model = match state.store.branch_tip(&project, &body.branch).map_err(map_store_error)? {
            Some(tip) => load_model(&state, &project, &tip)?,
            None => incoming.clone(),
        };
        let touched = touched_elements(&tip_model, &incoming);
        let held = state
            .store
            .holders_of(&project, &touched, now_seconds())
            .map_err(map_store_error)?;
        let blocked: Vec<&Lock> = held.iter().filter(|l| l.holder != holder).collect();
        if !blocked.is_empty() {
            return Err(ApiError::conflict(format!(
                "{} is locked by {} until {}",
                blocked[0].element, blocked[0].holder, blocked[0].expires_at
            )));
        }
    }
```

- [ ] **Step 3: Tests** in server/tests/locks.rs:

6. `a_commit_is_refused_when_another_holder_locks_a_touched_element` - A locks b1 on main; B commits a model that RENAMES b1 with holder B and gets 409; the branch tip is unchanged. Then B commits a model that changes only an element nobody locked, with holder B, and succeeds.
7. `the_holder_may_commit_its_own_locked_element` - A locks b1 and commits the rename with holder A; 201.

- [ ] **Step 4: Run, lint and commit** as in Task 1, expected 49 service tests.

---

## Task 3: The append-only audit log

**Files:**
- Modify: server/src/store/mod.rs (AuditEntry type, two trait methods)
- Modify: server/src/store/sqlite.rs (the audit table, append and read)
- Modify: server/src/api.rs, gate_api.rs, merge_api.rs, locks_api.rs (record on every mutation)
- Create: server/src/audit_api.rs
- Modify: server/src/lib.rs (pub mod audit_api, one route)
- Test: server/tests/audit.rs

**Interfaces:**
- Produces:
  - `pub struct AuditEntry { pub id: i64, pub project: String, pub at: i64, pub actor: String, pub action: String, pub subject: String, pub detail: String }`, serialised camelCase.
  - `Store::append_audit(&self, entry: &AuditEntry) -> Result<i64, StoreError>` - returns the row id; there is deliberately NO update, delete or truncate method anywhere.
  - `Store::audit(&self, project: &str, limit: i64) -> Result<Vec<AuditEntry>, StoreError>` - newest first, capped at 1000.
  - Actions recorded, with their subject: `project.create`, `commit.create`, `branch.create`, `branch.delete`, `branch.reset`, `merge.clean`, `merge.conflict`, `lock.acquire`, `lock.release`, `gate.run`. The actor is the request's author or holder.
  - `GET /projects/:project/audit?limit=n` -> 200 [AuditEntry] newest first.

**Steps:**

- [ ] **Step 1: The table**, append-only in shape as well as intent:

```sql
CREATE TABLE IF NOT EXISTS audit (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project TEXT NOT NULL,
    at INTEGER NOT NULL,
    actor TEXT NOT NULL,
    action TEXT NOT NULL,
    subject TEXT NOT NULL,
    detail TEXT NOT NULL
);
```

- [ ] **Step 2: Append and read** in the store, one transaction each, with the project's auditing written INSIDE the same transaction as the mutation it describes wherever that is practical (a commit and its audit row succeed or fail together). Where a mutation already owns a transaction - commit_model, commit_merge - the audit insert belongs inside it; the implementer must add an optional audit entry parameter to those two rather than writing the audit row in a second transaction, and must say so in the report if that proves awkward.

- [ ] **Step 3: Record from every mutation path.** A gate run records `gate.run` with the reference and candidate hashes and the verdict, including a FAILED gate: the audit trail exists to record what was attempted, not only what succeeded.

- [ ] **Step 4: server/src/audit_api.rs and the route** `GET /projects/:project/audit`.

- [ ] **Step 5: Tests** in server/tests/audit.rs:

1. `every_mutation_is_recorded` - create a project, commit, branch, merge cleanly, reset, acquire and release a lock, run a gate; then GET the audit and assert all nine actions appear with actor, subject and a non-empty detail.
2. `a_failed_gate_is_recorded` - run the gate on the corrupted corpus model and assert a `gate.run` entry whose detail says the run failed.
3. `a_conflicting_merge_is_recorded` - assert `merge.conflict` names the branches.
4. `entries_are_never_rewritten` - read the audit, append two more actions, read again, and assert the earlier entries are BYTE IDENTICAL (same ids, timestamps, actors, subjects and details) rather than merely present. This is the test that proves append-only.
5. `the_log_is_newest_first_and_capped` - request limit=2 and assert exactly two entries, newest first.

- [ ] **Step 6: Run, lint and commit** as before; expected 54 service tests, and the whole workspace green.

---

## Completion criteria

- [ ] cargo test --workspace --no-fail-fast passes, including the 12 new service tests.
- [ ] cargo fmt --all -- --check and cargo clippy --workspace --all-targets -- -D warnings are clean.
- [ ] Two holders cannot edit one element without one of them being told, and a refusing acquire leaves NOTHING acquired.
- [ ] An expired lease stops blocking, proven by an injected clock rather than a sleep.
- [ ] A commit that touches an element locked by someone else is refused, and a commit that touches nothing locked is not.
- [ ] Every mutation writes exactly one audit entry, including failures, and the log can never be rewritten.

## What the next tranche must add (not in this plan)

- Identity (OIDC and LDAP), roles and per-project scoping; today the actor is whatever the request says it is, which is exactly why the audit log is not yet evidence in a dispute. This is the single most important follow-on.
- The PostgreSQL backend behind the same Store trait, verified in CI with a service container, since this machine has no Docker.
- The CLI, and the docker compose and Helm deployment skeleton.
- Lock-aware merge: when a merge would change a locked element, refuse it the same way a commit is refused.
