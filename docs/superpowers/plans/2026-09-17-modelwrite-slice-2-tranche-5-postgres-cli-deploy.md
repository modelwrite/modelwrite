# Slice 2 (tranche 5) - PostgreSQL, the CLI, and deployment

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (- [ ]) syntax for tracking.

**Goal:** the repository can run on a real database, be driven from a command line, and be installed the way an engineering organisation installs software. That completes Slice 2.

**Architecture:** the PostgreSQL backend implements the SAME Store trait as SQLite, so nothing above the store changes - which is the payoff for having kept persistence behind a trait since tranche 1. The CLI talks to the service over HTTP, with an offline mode that opens a local database directly, because an air-gapped site will have one and not the other. Deployment is a container image, a compose file for a trial, and a Helm chart for a real install, with the open-mode warning surfaced where an operator will actually see it.

**A note this plan must not hide:** this machine has NO Docker and NO PostgreSQL. The PostgreSQL backend therefore cannot be exercised locally at all, and the deployment files cannot be run locally. Both are verified in CI - GitHub Actions provides a Postgres service container - and every report must say plainly what was and was not actually run. A backend that compiles and passes only in CI is a weaker claim than one that passes locally, and the honest framing is part of the deliverable.

**Tech Stack:** adds NO dependency to the server beyond a Postgres driver. The CLI adds NO dependency at all: it speaks HTTP/1.1 over std::net::TcpStream for http:// endpoints and opens the store directly for offline use. TLS is expected to be terminated by a proxy, and the CLI says so rather than pretending to do it.

**Spec:** docs/superpowers/specs/2026-09-17-modelwrite-platform-design.md (sections 4.2, 4.5, 5).

## Global Constraints

- Rust stable; edition 2021; rust-version 1.75.
- Every source file begins with: // SPDX-License-Identifier: AGPL-3.0-or-later
- Cargo is not on PATH in fresh shells: begin every shell command sequence with $env:Path = "$env:USERPROFILE\\.cargo\\bin;" + $env:Path
- Use --no-fail-fast when running a test suite.
- Stage and commit only the paths a task owns. Never amend or rewrite a commit.
- The PostgreSQL backend must honour EVERY invariant the SQLite backend enforces, in the database rather than in a process mutex: the tip read and the commit write in ONE transaction, the lock check inside the write transaction, the audit insert inside the mutation's transaction, and parents verified before a merge commit cites them. The SQLite tests that assert these are the specification; where a test is backend-specific, write the Postgres equivalent rather than skipping it.
- Postgres tests SKIP when MW_TEST_DATABASE_URL is unset, and the report must state how many tests were skipped and that they were.
- The audit table's append-only property is enforced by a trigger on Postgres too, not by convention.
- The CLI must never print a token, and must exit non-zero on a failed command with a message a person can act on.
- No test may bind a port outside the ephemeral range, and no test may depend on the network.

---

## Task 1: The PostgreSQL store behind the same trait

**Files:**
- Modify: server/Cargo.toml (the postgres driver, feature-gated off by default)
- Create: server/src/store/postgres.rs
- Modify: server/src/store/mod.rs (a constructor that picks the backend from configuration)
- Test: server/tests/postgres.rs (skipped without MW_TEST_DATABASE_URL)

**Interfaces:**
- Produces: `PostgresStore::open(url: &str) -> Result<Self, StoreError>` implementing the whole Store trait, and `pub fn open_store(config: &StoreConfig) -> Result<Arc<dyn Store>, StoreError>` where `StoreConfig` is `Sqlite(PathBuf)` or `Postgres(String)` chosen from `MW_DB` (a path) or `MW_DATABASE_URL`.

**Steps:**

- [ ] **Step 1: Add the driver.** Choose the rustls-based driver so the image does not need OpenSSL, and record the choice and version in the report. Feature-gate it so a build without Postgres still works if that proves simpler.

- [ ] **Step 2: The schema, as SQL that enforces the invariants.** Port the SQLite schema to Postgres types (TEXT stays TEXT, INTEGER becomes BIGINT, the audit id becomes BIGSERIAL), keeping every constraint and adding: a foreign key from commits to their parents is impossible in a single column, so instead keep the explicit parent check; a trigger that raises on UPDATE and DELETE of the audit table; and the CHECK constraints that reject an audit entry naming nobody.

- [ ] **Step 3: Implement the trait.** Every method that mutates must use ONE database transaction, and the tip read must happen inside it. Where the SQLite backend relies on a process-wide mutex for serialisation, Postgres must rely on the transaction and on row locks: use `SELECT ... FOR UPDATE` on the branch row where a tip is read before a write, so two concurrent commits to one branch serialise in the database rather than in a mutex.

- [ ] **Step 4: Tests** in server/tests/postgres.rs, mirroring the SQLite store tests: the eight-thread linearity test, the merge refusal when the branch moved, the lock lease boundary with an injected clock, the audit rollback when the entry cannot be written, and branch deletion leaving commits readable.

- [ ] **Step 5: Run what can be run, and say what could not.** Locally there is no PostgreSQL, so the tests skip. Verify the code compiles, lints and that the skip path works. Expected: the local suite is green with the Postgres tests reported as skipped.

- [ ] **Step 6: Commit** with `feat: add a PostgreSQL backend behind the store trait`.

---

## Task 2: The CLI

**Files:**
- Create: cli/Cargo.toml, cli/src/main.rs, cli/src/http.rs, cli/src/offline.rs
- Modify: Cargo.toml (workspace members)
- Test: cli/tests/cli.rs (argument parsing and the offline path; the HTTP path tested against an in-process server)

**Interfaces:**
- Produces a binary named `mw` with commands: `project create|list`, `commit --branch --message --file --holder`, `log --branch`, `branch list|create|delete`, `merge --branch --other --holder`, `reset --branch --to --holder`, `gate --reference --candidate`, `lock acquire|release|list`, `audit --limit`. Global flags: `--server <url>` with `--token <env:VAR>`, or `--db <path>` for offline use.

**Steps:**

- [ ] **Step 1: The skeleton and argument parsing by hand**, so the CLI adds no dependency: a small parser that produces a typed Command, with a unit test per command and a clear error for an unknown flag.

- [ ] **Step 2: The offline path** opens the store directly and performs the same operations, so an air-gapped user can commit and inspect history with no server at all.

- [ ] **Step 3: The HTTP path** is a minimal HTTP/1.1 client over std::net::TcpStream: request line, headers, content-length body, and response parsing with status and body. It refuses an https:// URL with a message explaining that TLS belongs at the proxy.

- [ ] **Step 4: The token comes from an environment variable named by the flag**, never from the command line, because a token in a command line is a token in the shell history and in the process list.

- [ ] **Step 5: Tests**: each command parses; a failed command exits non-zero; offline commit then log round-trips; the HTTP path is exercised against the in-process router bound to an ephemeral port; a token is never echoed in output or in an error.

- [ ] **Step 6: Commit** with `feat: add the modelwrite command line`.

---

## Task 3: Deployment

**Files:**
- Create: deploy/Dockerfile, deploy/docker-compose.yml, deploy/helm/modelwrite/Chart.yaml, values.yaml, templates/deployment.yaml, templates/service.yaml, templates/configmap.yaml, templates/secret.yaml.example
- Create: deploy/README.md
- Modify: README.md (a quick start that uses them)

**Interfaces:**
- Produces: a container image that runs the service as a non-root user with a read-only root filesystem where possible; a compose file that brings up the service with a local volume for trials; a Helm chart with values for the database URL, the auth configuration, resources, and persistence; and a README that states the open-mode warning in bold, explains the air-gapped install path (image transfer, mounted JWKS file, local Postgres or none), and lists what the chart does NOT do (no TLS termination, no backups, no HA).

**Steps:**

- [ ] **Step 1: The image**: a multi-stage build, the binary copied into a slim runtime, a non-root user, and an entrypoint that refuses to start when neither auth configuration nor an explicit MW_ALLOW_OPEN=yes is present. **A production service should not be able to run unauthenticated by accident.**

- [ ] **Step 2: compose** for a trial: service plus a Postgres container, a named volume, and the environment variables documented inline.

- [ ] **Step 3: The Helm chart**, with values documented in values.yaml, secrets referenced rather than inlined, a liveness and readiness probe against /health, and resource requests and limits.

- [ ] **Step 4: Validate what can be validated here.** There is no Docker on this machine, so nothing can be built or run. Verify the YAML parses, the Helm templates are well-formed YAML, and that the Kubernetes API fields used are real for the target version. Say plainly in the report that the image was never built and the chart never installed.

- [ ] **Step 5: Commit** with `feat: add container, compose and Helm deployment`.

---

## Task 4: CI proves the parts this machine cannot

**Files:**
- Modify: .github/workflows/ci.yml
- Modify: deploy/README.md (what CI proves)

**Interfaces:**
- Produces a CI job with a postgres service container that sets MW_TEST_DATABASE_URL and runs the Postgres tests for real, plus a job that lints the Helm chart and validates the compose file with a YAML parser, and a job that builds the container image.

**Steps:**

- [ ] **Step 1: The Postgres job**, with a service container, a health check, and the same workspace tests run against it so the skipped-locally tests actually execute.

- [ ] **Step 2: The deployment job** that parses every YAML file in deploy/ and fails on a syntax error, and builds the image.

- [ ] **Step 3: Prove it** by pushing and reading the CI result, then recording the run id and the outcome in the ledger. A green CI run is the only evidence that the Postgres and container paths work at all.

- [ ] **Step 4: Commit** with `ci: exercise PostgreSQL and the container image`.

---

## Completion criteria

- [ ] cargo test --workspace --no-fail-fast passes locally, with Postgres tests skipping honestly.
- [ ] CI is green, including the Postgres tests running against a real database and the image building.
- [ ] The CLI performs a full round trip offline and against a running service.
- [ ] The image refuses to start unauthenticated without an explicit opt-in.
- [ ] Every report states what was run and what was not.

## What the next slice must add (not in this plan)

Slice 3, the authoring workbench: a browser interface over the same API, with the engineering views an MBSE practitioner expects (structure, requirements traceability, state, activity), diff and merge visualisation, and the gate rendered where a reviewer will see it.
