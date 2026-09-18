# Slice 2 (tranche 4) - Identity, roles and verified actors

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (- [ ]) syntax for tracking.

**Goal:** every request has a verified actor and an enforced role, and the audit log records who REALLY did it rather than who the request claimed to be.

**Why this tranche, and why now:** tranche 3 gave the repository an append-only audit log, and the log currently records whatever the request says: `author` in a commit body, `holder` in a lock body, and `"unknown"` for project creation, branch creation, branch deletion and gate runs. An unattributable record is not evidence. This tranche is what turns the log into something an organisation can defend in a review a year later, which is the precondition for the repository being a system of record at all.

**Architecture:** authentication is a middleware layer that turns a bearer token into an `Identity { subject, roles, projects }`, and authorisation is a small permission check the handlers call. Two token kinds are supported on purpose: a static token for pilots, CI and air-gapped installs where an identity provider does not exist, and signed JWTs for organisations that have one. Nothing else in the service changes shape: handlers receive an Identity through a request extension and stop trusting the body for who the caller is.

**Tech Stack:** unchanged plus one declared dependency, added only in Task 4.

**Spec:** docs/superpowers/specs/2026-09-17-modelwrite-platform-design.md (sections 4.3 and 4.6).

## Global Constraints

- Rust stable; edition 2021; rust-version 1.75. NO new dependency before Task 4, which declares exactly one and says why.
- Every source file begins with: // SPDX-License-Identifier: AGPL-3.0-or-later
- Cargo is not on PATH in fresh shells: begin every shell command sequence with $env:Path = "$env:USERPROFILE\\.cargo\\bin;" + $env:Path
- Use --no-fail-fast when running a test suite.
- Stage and commit only the paths a task owns. Never amend or rewrite a commit.
- AUTHENTICATION MUST BE OPT-IN so the existing 57 tests keep working: when no auth configuration is present (no MW_AUTH_TOKEN, no MW_AUTH_JWKS), the service runs in OPEN mode, every request is accepted, and the identity is `Identity::open()` with role admin and subject "anonymous". A test asserts open mode still works, and the startup path logs a loud warning that the service is unauthenticated.
- A VERIFIED ACTOR REPLACES A CLAIMED ONE: once auth is configured, the audit actor is the identity's subject and the request body's author or holder is IGNORED for attribution. If the body names a different actor, the request is refused with 403 rather than quietly recorded under the wrong name.
- Comparison of secrets is CONSTANT TIME. A token comparison that returns early on the first differing byte is a timing oracle.
- Never log a token, and never put one in an error body.
- Roles are `admin`, `author`, `reviewer`, `viewer`. Permissions: viewer reads; author adds commits, branches, resets, merges and locks; reviewer adds reading gate runs and evidence; admin adds project creation, branch deletion and lock breaking.
- Every request path returns a structured error and never panics: 401 for a missing or invalid token, 403 for an authenticated caller without the permission, and 500 with a generic message and no internal detail.

---

## Task 1: Identity, the permission model and open mode

**Files:**
- Create: server/src/auth.rs
- Modify: server/src/lib.rs (module, AppState gains auth configuration)
- Test: server/tests/auth.rs

**Interfaces:**
- Produces:
  - `pub struct Identity { pub subject: String, pub roles: Vec<String>, pub projects: Vec<String> }` with `Identity::open()`.
  - `Identity::has_role(&self, role: &str) -> bool`, `Identity::may(&self, permission: Permission) -> bool`, `Identity::may_reach(&self, project: &str) -> bool` (a projects entry of "*" reaches every project).
  - `pub enum Permission { Read, Write, Review, Administer }` and a mapping from permission to the roles that grant it.
  - `pub enum AuthConfig { Open, Static { token_hash: String }, Jwt { .. } }` - Task 1 implements Open and Static only; Jwt arrives in Task 4.
  - `pub fn parse_identity_from_static(token: &str, config: &AuthConfig) -> Option<Identity>` comparing SHA-256 of the presented token against the configured hash in constant time.

**Steps:**

- [ ] **Step 1: Write server/src/auth.rs** with the types above plus unit tests in the file for the role mapping and constant-time comparison. Hash the configured token once at startup so comparison is over fixed-width digests and no length is leaked.

- [ ] **Step 2: AppState gains the configuration.** `pub struct AppState { pub store: Arc<dyn Store>, pub evidence_dir: PathBuf, pub auth: AuthConfig }`. Update every existing test that constructs AppState to pass `AuthConfig::Open`; they are the proof that open mode is unchanged.

- [ ] **Step 3: An extraction layer.** Add `pub async fn identity(...)` as an axum extractor that reads `Authorization: Bearer <token>`, resolves it against the configuration, and returns `Result<Identity, ApiError>` - 401 for a missing or bad token. In open mode it returns `Identity::open()`.

- [ ] **Step 4: Tests** in server/tests/auth.rs: open mode accepts an unauthenticated request; static mode rejects a missing token with 401 and a wrong token with 401; static mode accepts the right token; unit tests in auth.rs cover the role mapping, including that viewer cannot write and author cannot administer.

- [ ] **Step 5: Run, lint and commit** with `feat: add identity, roles and an opt-in authentication mode`. Expected: 57 service tests plus the new ones.

---

## Task 2: Enforce identity and roles across every route

**Files:**
- Modify: server/src/lib.rs (apply the extractor), api.rs, gate_api.rs, merge_api.rs, locks_api.rs, audit_api.rs
- Test: server/tests/auth.rs

**Interfaces:**
- Produces: every handler takes `Identity` as an extractor and calls `identity.may(...)` before doing anything; read routes require `Read`, mutations require `Write`, project creation and branch deletion require `Administer`, gate runs require `Write` to run and `Review` to list evidence. Every handler also checks `identity.may_reach(&project)`.

**Steps:**

- [ ] **Step 1: Thread the identity through the routes.** Where a handler already takes several extractors, Identity goes first. Keep each handler's body otherwise unchanged.

- [ ] **Step 2: Test the denials** in server/tests/auth.rs: a viewer is refused a commit with 403; an author is refused project creation with 403; a token scoped to project "coffee" is refused on project "tea" with 403; every route that exists appears in at least one denial test, so a new route cannot be added without a permission decision.

- [ ] **Step 3: Run, lint and commit** with `feat: enforce roles and project scope on every route`.

---

## Task 3: The audit actor becomes the verified identity

**Files:**
- Modify: server/src/api.rs, merge_api.rs, locks_api.rs, gate_api.rs (audit entries take the subject from the Identity)
- Modify: server/src/store/mod.rs (AuditEntry documented as carrying a verified subject)
- Test: server/tests/audit.rs

**Interfaces:**
- Produces: the audit actor is `identity.subject` on every path, including the four that currently record `"unknown"`. In open mode the subject is `"anonymous"`, which is honest: nobody was authenticated. A request body that names an actor different from the identity is refused with 403, so a caller can never write another person's name into the record.

**Steps:**

- [ ] **Step 1: Replace every claimed actor** with the identity's subject, and add the 403 when a body's author or holder disagrees with the identity.

- [ ] **Step 2: Tests** in server/tests/audit.rs: with auth configured, a commit by identity "alex" records actor "alex" even though the body says "someone-else" (which is refused with 403, so the test asserts the refusal AND that nothing was recorded); project creation records the identity rather than "unknown"; open mode records "anonymous", which is a truthful statement that nobody was authenticated.

- [ ] **Step 3: Run, lint and commit** with `feat: record verified actors in the audit log`.

---

## Task 4: Signed tokens (JWT) against a JWKS

**Files:**
- Modify: server/Cargo.toml (ONE new dependency, and only here)
- Modify: server/src/auth.rs (the Jwt variant)
- Test: server/tests/auth.rs

**Interfaces:**
- Produces: `AuthConfig::Jwt { keys: HashMap<String, DecodingKey>, issuer: Option<String>, audience: Option<String> }`, built at startup from a JWKS file path in `MW_AUTH_JWKS` (a mounted secret, which is what an air-gapped install has) or fetched from `MW_AUTH_JWKS_URL` once at startup. Roles come from a configurable claim, default `roles`; projects from a claim, default `projects`. A token that fails signature, expiry, issuer or audience verification is a 401.

**Steps:**

- [ ] **Step 1: Declare the dependency.** `jsonwebtoken` is the one addition, and the plan says why: verifying an RSA signature correctly is not something to hand-roll, and the crate is the de-facto standard for this in Rust. Record the version chosen and that a cargo-deny or advisory check is a later governance task.

- [ ] **Step 2: Implement the Jwt variant** including clock skew tolerance of 60 seconds and a rejection of `alg: none`.

- [ ] **Step 3: Tests** with a key pair generated in the test itself: a valid token is accepted; an expired token is rejected; a token signed by the wrong key is rejected; a token with a different issuer or audience is rejected; a token whose roles claim is absent gets no roles and therefore cannot write. If generating a key pair in a test proves unreasonable, use a committed test key pair and say so in the report.

- [ ] **Step 4: Run, lint and commit** with `feat: accept signed tokens from an identity provider`.

---

## Completion criteria

- [ ] cargo test --workspace --no-fail-fast passes; fmt and clippy clean.
- [ ] Open mode is unchanged and proven by the pre-existing tests still passing.
- [ ] Every route enforces a role and a project scope, and a test exists for each denial.
- [ ] The audit actor is the verified identity everywhere, and a body cannot write another person's name into the record.
- [ ] A token is never logged and never appears in a response body.

## What the next tranche must add (not in this plan)

- The PostgreSQL backend behind the same Store trait, verified in CI with a service container.
- The CLI, which authenticates with the same tokens and no browser.
- The deployment skeleton: docker compose and Helm, with the auth configuration as mounted secrets and the loud open-mode warning surfaced in the install documentation.
- Token revocation and rotation, which need a store of their own and belong with the audit work.
