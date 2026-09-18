# Slice 6 (tranche 1) - Governance: making claims checkable

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development. Steps use checkbox (- [ ]) syntax.

**Goal:** every claim this platform makes about a model can be CHECKED from the repository, a year later, by somebody who was not there. That is what governance means here, and it is the last thing standing between a good tool and one an organisation will trust with its engineering record.

**Why these tasks, in this order:** they are not invented. Every one was FOUND by a review during Slices 2 to 5, and each is a place where something looked authoritative and was not:
1. A commit that entered by migration cannot be told from one that was authored - found by the interoperability whole-slice review.
2. The migration rules hold on the IMPORT path only, so a document converted outside the tool and committed normally lands with no loss report and no marker - same review.
3. A binding's mapping table is dead data whose documentation claims more than it does - same review.
4. There is no way, from a model, to ask WHICH checks have been run on it and what they said - the gate runs live in a list keyed by project, not attached to the commit they justify.

**Architecture:** governance is not a new subsystem; it is the existing records made addressable. The audit log, the gate runs, the imports table and the commit provenance already exist. This slice connects them to the commits they describe and gives a reader ONE place to ask what is known about a model and what is merely claimed.

## Global Constraints

- Rust stable; engine crates at rust-version 1.75, server at 1.88.
- Every source file begins with: // SPDX-License-Identifier: AGPL-3.0-or-later
- Cargo is not on PATH in fresh shells: begin every shell command sequence with $env:Path = "$env:USERPROFILE\\.cargo\\bin;" + $env:Path
- Use --no-fail-fast when running a test suite.
- Stage and commit only the paths a task owns. Never amend or rewrite a commit.
- **A CLAIM MUST BE CHECKABLE OR IT MUST BE LABELLED AS UNCHECKED.** The platform may say "this was measured", "this was asserted by a binding", or "this is unknown". It must never blur the three.
- **PROVENANCE IS PERMANENT AND ADDITIVE.** Governance adds records; it never rewrites history to make a story tidier.
- No path panics; a missing record is a 404 or an explicit "unknown", never a silent empty success.

---

## Task 1: A commit knows how it arrived

**Files:** modify server/src/store/{mod.rs,sqlite.rs,postgres.rs} (commit provenance), server/src/api.rs (commit_json), server/src/binding_api.rs (write it on import); test server/tests/provenance.rs.

**Interfaces:** a commit carries an OPTIONAL provenance recording how it was produced: `authored` for a normal commit or edit, `imported` with the artifact hash, binding id and version, and the accepted losses, for a migration. It is written in the SAME transaction as the commit. `GET /projects/:project/commits/:hash` returns it, and a commit with none says so plainly rather than omitting the field.

**Acceptance:** a commit made by the editor records `authored`; a commit made by an import records `imported` with its artifact hash and binding; a reader can tell them apart from the commit alone; an existing commit with no provenance reads as unknown rather than as authored.

---

## Task 2: The migration rules are a property of the repository, not of one endpoint

**Files:** modify server/src/api.rs or the shared commit core; test server/tests/migration.rs or a new server/tests/governance.rs.

**Interfaces:** a commit declaring `imported` provenance must name the artifact it came from, and the artifact must exist; a commit whose document contains losses that were never accepted cannot be marked imported. The rule is enforced by the commit path, so a migration performed by ANY route - the endpoint, the CLI, or a future one - meets it.

**Ruling:** the honest boundary found in Slice 4 was that the four rules held on the import path only. This task closes it by moving the enforcement from the endpoint to the commit, and a test must prove a NON-import route cannot produce an imported commit without provenance.

---

## Task 3: A model knows which checks have been run on it

**Files:** modify server/src/gate_api.rs (record the candidate commit on a run), server/src/store/*, server/src/gate_api.rs or a new endpoint; test.

**Interfaces:** a gate run records the CANDIDATE COMMIT it was run against, so `GET /projects/:project/commits/:hash/checks` answers what has been run on this model and what it said, and a commit with no runs says so rather than returning an empty list that looks like a pass.

**Acceptance:** running the gate on a commit makes it appear in that commit's checks with its verdict and evidence; a commit nobody has checked says so explicitly.

---

## Task 4: The mapping table is read rather than described

**Files:** modify engine/binding (reconcile or correct the doc), engine/binding-xmi; test engine/binding/tests or binding-xmi/tests.

**Interfaces:** either the gate CONSULTS `mapping_table()` when checking an import, or the trait documentation states plainly that the table is declaration rather than gate input - and either way a test pins which it is. The doc currently claims the gate reads it, and nothing does.

**Acceptance:** a reader of the trait can tell whether the table is enforced or declarative, and the claim matches the code.

---

## Completion criteria

- [ ] cargo test --workspace --no-fail-fast passes; fmt and clippy clean.
- [ ] A migrated commit is distinguishable from an authored one from the commit alone.
- [ ] The migration rules hold on every route that can create an imported commit, proven by a test on a non-import route.
- [ ] A model can be asked which checks have run on it, and an unchecked model says so.
- [ ] No documentation in this slice claims more than the code does.

## Later tranches of this slice

- Baselines and sign-off: marking a commit as reviewed or released, with who and when.
- Export of a complete evidence pack for a model, so an audit can be answered without the system running.
- The certification mappings (DO-178C, ISO 26262) that turn the records into the artefacts a safety case needs.
