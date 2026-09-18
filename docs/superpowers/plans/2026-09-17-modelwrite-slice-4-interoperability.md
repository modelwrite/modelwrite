# Slice 4 (tranche 1) - The binding framework and the first real binding

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (- [ ]) syntax for tracking.

**Goal:** an organisation can bring a model built under a DIFFERENT standard into modelwrite, see exactly what survived the journey, keep the original forever, and keep working.

**Architecture:** a standard is not the centre of this system; OKF is. A standard version is a **binding**: a versioned adapter with a conformance corpus, a mapping matrix and a fidelity record. This tranche builds the harness every later binding will use, and then proves it with one binding that matters - **SysML v1 XMI**, the format most likely to be stranded, because it is what organisations exported years ago from tools they may no longer licence.

**Spec:** docs/superpowers/specs/2026-09-17-modelwrite-standards-interoperability.md. Read it before starting any task: it defines the binding contract, the three-way mapping verdicts, and what is and is not promised.

**Tech Stack:** Rust, the existing engine crates. One new dependency is declared in Task 2 and only there: a streaming XML reader, because XML is not a format to parse by hand and the existing crates do not read it.

## Global Constraints

- Rust stable; edition 2021. The server crate is at rust-version 1.88; the engine crates are at 1.75 and MUST STAY THERE, because a binding belongs in the engine layer, not the service.
- Every source file begins with: // SPDX-License-Identifier: AGPL-3.0-or-later
- Cargo is not on PATH in fresh shells: begin every shell command sequence with $env:Path = "$env:USERPROFILE\\.cargo\\bin;" + $env:Path
- Use --no-fail-fast when running a test suite.
- Stage and commit only the paths a task owns. Never amend or rewrite a commit.
- **NO MAPPING IS EVER APPLIED SILENTLY.** Every loss is named, with the subject and the reason, before anything is committed. This is the whole point of the slice.
- **THE SOURCE ARTIFACT IS RETAINED BYTE FOR BYTE** and content-addressed before anything else happens, so a migration can never destroy the thing it migrated.
- **FIDELITY IS MEASURED, NOT ASSERTED.** A binding's claim about what it preserves is a corpus plus a report, produced by the existing gate, not a paragraph.
- A binding that cannot export is a VIEWER and must say so in its own metadata, so nobody mistakes reading for migrating.
- No network access in any test.

---

## Task 1: The binding contract and the fidelity harness

**Files:**
- Create: engine/binding/src/lib.rs (the trait and its metadata), engine/binding/src/report.rs (the loss report)
- Modify: Cargo.toml (a new workspace member)
- Test: engine/binding/tests/harness.rs

**Interfaces:**
- `pub struct BindingInfo { pub id: String, pub version: String, pub direction: Direction, pub description: String }` with `Direction::{ImportOnly, ImportAndExport}`.
- `pub enum MappingVerdict { Exact, Lossy, Unmappable }` and `pub struct Mapping { pub subject: String, pub verdict: MappingVerdict, pub note: String }`.
- `pub struct LossReport { pub binding: BindingInfo, pub mappings: Vec<Mapping>, pub artifact_hash: String }` with `is_lossless()` and `blocking()` (the entries a human must decide on).
- `pub trait Binding { fn info(&self) -> BindingInfo; fn import(&self, source: &[u8]) -> Result<(OkfRoot, LossReport), BindingError>; fn export(&self, root: &OkfRoot) -> Result<Vec<u8>, BindingError>; }`
- `pub fn round_trip(binding: &dyn Binding, source: &[u8]) -> Result<FidelityOutcome, BindingError>` where the outcome carries the import loss report AND the engine's own diff between the round-tripped and the original document, so a binding cannot claim fidelity the engine contradicts.

**Acceptance:** a deliberately broken test binding that drops an element on import produces a fidelity outcome whose engine diff names that element; a binding whose metadata claims export but which returns an error from export is rejected by the harness rather than trusted.

---

## Task 2: The SysML v1 XMI reader

**Files:**
- Create: engine/binding-xmi/src/lib.rs, engine/binding-xmi/src/model.rs (the subset it understands)
- Modify: Cargo.toml, engine/binding/Cargo.toml
- Test: engine/binding-xmi/tests/xmi.rs, fixtures under engine/binding-xmi/fixtures/

**Interfaces:**
- `pub struct XmiBinding` implementing `Binding` with id `sysml-v1-xmi`, version `2.4`, direction ImportAndExport.
- The subset it understands, stated as data in its mapping table: UML/SysML `packagedElement` of type `uml:Package`, `uml:Class` with the `Block` stereotype, `uml:Property` with an aggregation, `uml:Dependency` with the `Satisfy`/`Allocate`/`Refine`/`Verify` stereotypes, `uml:Comment` as documentation, and xmi:id as the element id.
- EVERYTHING ELSE is reported: unmapped elements and attributes are recorded as Unmappable mappings with the XMI element name and xmi:id, never dropped in silence.

**Rulings that shape this task:**
- The fixture is HAND-WRITTEN and says so. A synthetic XMI document exercising these structures is a test of the READER, not evidence about any vendor's exporter. Real vendor XMI needs its own corpus and its own report, and pretending otherwise would be the exact dishonesty the slice exists to prevent.
- Unknown elements are reported, not skipped.

**Acceptance:** the fixture imports to an OKF document whose blocks, properties and dependency edges match a hand-checked expectation; an unknown XMI element in the fixture produces an Unmappable entry naming it; a malformed document is a clean error, never a panic.

---

## Task 3: Migration as a gated operation

**Files:**
- Modify: server/src/api.rs (a migration endpoint) or create server/src/binding_api.rs
- Modify: server/src/lib.rs (the route)
- Test: server/tests/migration.rs

**Interfaces:**
- `POST /projects/:project/import` with `{ "binding": "sysml-v1-xmi@2.4", "branch": "main", "author": "...", "message": "...", "artifact": "<base64 or raw body>" }`:
  1. stores the source artifact as a BLOB first, so it is content-addressed and retained before anything can fail;
  2. imports it, producing a document and a loss report;
  3. runs the ENGINE's fidelity check on the result;
  4. refuses to commit when the loss report has blocking entries, returning them, UNLESS the request carries an explicit acceptance of named losses;
  5. commits on success with provenance recording the artifact hash, the binding id and version, and the accepted losses.
- `GET /projects/:project/import/:artifactHash/report` returns the loss report for a retained artifact.

**Acceptance:** importing the fixture commits and the commit's provenance names the artifact hash and the binding; importing a document with an unmapped element is refused until the request names that loss as accepted, and then the audit entry records that it was accepted; the original artifact is retrievable byte-identical after the import.

---

## Task 4: The workbench shows a migration

**Files:**
- Create: server/src/ui/import.rs
- Modify: server/src/ui/mod.rs, pages.rs, lib.rs
- Test: server/tests/ui.rs

**Interfaces:**
- `GET /ui/projects/:project/import` renders a form to paste or upload an artifact with a binding chosen from the registry, and, after a run, the loss report grouped by verdict with the blocking entries first, each naming its subject, and a control to accept a named loss.
- The page states plainly, in the page and not only in a log, what was retained and what was lost.

**Acceptance:** the fixture imports through the page; a lossy import shows its losses and requires the acceptance before committing; the page never claims a lossless migration the report contradicts.

---

## Completion criteria

- [ ] cargo test --workspace --no-fail-fast passes; fmt and clippy clean.
- [ ] The binding framework is proven by a deliberately broken binding, so the harness is known to catch infidelity rather than merely to exist.
- [ ] The XMI binding imports the fixture, reports every element it does not understand, and retains the original byte for byte.
- [ ] A lossy migration cannot be committed without the loss being named and accepted, and the acceptance is in the audit record.

## What later tranches must add (not in this plan)

- Real vendor corpora: an export from an actual tool, with its own fidelity report, and the honest statement of what it shows.
- Export back to XMI, so an organisation can keep using the tool it already pays for while the repository becomes the system of record.
- The v1-to-v2 mapping matrix, and the bindings that follow: v2, ReqIF, AP242, FMI, and the vendor and CAD formats.
- The extension mechanism for what a target standard cannot express (layout, colours, vendor properties), which the design calls binding extensions.
