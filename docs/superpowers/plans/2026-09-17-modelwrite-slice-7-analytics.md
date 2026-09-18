# Slice 7 (tranche 1) - Analytics: fusing models with the rest of the record

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development. Steps use checkbox (- [ ]) syntax.

**Goal:** answer the questions that cross a single model's boundary - which products meet which specifications, which requirements have no provider, what the expensive ones are - by fusing models with external structured and unstructured sources, where EVERY ANSWER IS TRACEABLE TO ITS SOURCES AND ITS CONFIDENCE IS ITS WEAKEST LINK.

**Read first:** docs/superpowers/specs/2026-09-17-modelwrite-analytics-design.md. It is the specification this plan implements.

**Architecture:** a new engine crate for the fusion and query logic (no network, no model provider), and server-side endpoints that read models through the existing store and datasets through content-addressed snapshots. Analytics READS; anything that implies a model change becomes a proposal in the Slice 5 shape.

## Global Constraints

- Rust stable; engine crates at rust-version 1.75, server at 1.88.
- Every source file begins with: // SPDX-License-Identifier: AGPL-3.0-or-later
- Cargo is not on PATH in fresh shells: begin every shell command sequence with $env:Path = "$env:USERPROFILE\\.cargo\\bin;" + $env:Path
- Use --no-fail-fast when running a test suite.
- Stage and commit only the paths a task owns. **SERIALISE TASKS THAT TOUCH THE SAME FILES** - two concurrent tasks on one file caused a near-miss earlier in this project.
- **CONFIDENCE IS THE WEAKEST LINK.** A figure built from a measured value and an estimated one is an ESTIMATE and must be labelled so. Never average, never round up, never present a fused number as measured.
- **UNKNOWN IS AN ANSWER, AND NEVER A BLANK.** A requirement that is absent from a model is UNKNOWN, not satisfied and not blank. Rendering absence as a blank is the single most dangerous thing this slice could do.
- **ANALYTICS NEVER WRITES TO A MODEL.** Findings become proposals.
- **NO NETWORK AT QUERY TIME.** Sources are snapshotted deliberately and locally.

---

## Task 1: The source registry and the dataset snapshot

**Files:** create engine/analytics/src/lib.rs, src/source.rs, src/dataset.rs; modify Cargo.toml; test engine/analytics/tests/dataset.rs.

**Interfaces:** `SourceKind::{Model, Structured, Text}`, `TrustLevel::{Measured, Reported, Estimated}`, `Source { id, kind, trust, description }`; `Dataset { source, captured_at, rows, content_hash }` with `Dataset::from_csv(source, bytes)` producing a content-addressed snapshot; a registry that REFUSES a dataset whose source is not registered.

**Acceptance:** a CSV becomes a dataset with a stable content hash; the same bytes produce the same hash; an unregistered source is refused; the trust level travels with the data rather than being supplied at query time.

---

## Task 2: The fusion rule, enforced

**Files:** create engine/analytics/src/confidence.rs; test engine/analytics/tests/confidence.rs.

**Interfaces:** `Value { value, source, trust, captured_at }` and `fused_trust(values) -> TrustLevel` returning the WEAKEST trust among the inputs, plus `is_estimate()`; a sum or comparison over values returns a fused value carrying that trust and the SET of contributing sources.

**Acceptance:** a sum of one measured and one estimated value is ESTIMATED; a sum of measured values is MEASURED; every fused value names all its sources; a fused value with no sources is an error rather than a zero.

---

## Task 3: Which products meet which specifications, and what is UNKNOWN

**Files:** create engine/analytics/src/compliance.rs; test engine/analytics/tests/compliance.rs.

**Interfaces:** `classify(requirement: &str, model: &OkfRoot) -> Compliance::{Satisfied, NotSatisfied, Unknown}` using the ENGINE's graph coverage for the first two and reporting UNKNOWN when the requirement is absent from the model entirely; `portfolio_report(requirement_set, models) -> Report` grouping by state with counts that ADD UP to the number of requirements in the specification.

**Acceptance:** a requirement absent from a model is UNKNOWN, never Satisfied and never omitted; a requirement present but uncovered is NotSatisfied; the counts sum to the specification's size, so a missing row cannot hide.

---

## Task 4: Cost joins, labelled

**Files:** create engine/analytics/src/cost.rs; test engine/analytics/tests/cost.rs.

**Interfaces:** a declared column mapping per dataset (which column is which requirement id, which is cost), and `cost_by_requirement(models, datasets) -> Vec<CostedRequirement>` where each carries the value, its source and its captured_at; a requirement with NO cost record is reported as UNCOSTED rather than as zero, because zero is a number somebody would act on.

**Acceptance:** an uncosted requirement is UNCOSTED and not 0; a cost is attributed to its source and date; a dataset missing a declared column is a clear error.

---

## Completion criteria

- [ ] cargo test --workspace --no-fail-fast passes; fmt and clippy clean.
- [ ] Every figure an answer contains is traceable to a named source with a date and a trust level.
- [ ] A fused figure's trust is the weakest of its inputs, proven by a test.
- [ ] Absence is reported as UNKNOWN or UNCOSTED, never as a blank, a zero or a pass, proven by tests.
- [ ] Nothing in this slice writes to a model.

## Later tranches of this slice

- Agent-assisted extraction from unstructured sources, producing findings with quoted evidence for human review, in the Slice 5 proposal shape.
- Refreshing a registered source deliberately, with the snapshot history preserved so an answer can be reproduced.
- Portfolio dashboards, once the answers they render are trusted.
