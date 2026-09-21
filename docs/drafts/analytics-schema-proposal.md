# mw-analytics-schema@1 — inventory and schema proposal

Status: DRAFT — for owner approval. Report-and-artefact phase (Packages 1 and 2 of the
analytics-interfaces brief). No transport code has been written and none will be until the
owner approves this proposal in writing.

---

## 0. How this proposal was produced

Every claim below was verified against the repository, not against the brief's memory. The
controller's impact analysis (the ten corrections) was itself re-verified; one correction is
contradicted by the code and is called out in §9.2. Sources are named as `file::symbol`.

---

## 1. Surface inventory — what exists today

### 1.1 The REST API (`mw-server`)

Route table: `server/src/lib.rs::app_with_limit`. The versioned JSON surface is:

| Route | Method | What it returns | Source |
|---|---|---|---|
| `/health` | GET | `{status, authMode}` | `lib.rs::health` |
| `/version` | GET | `{server, service}` | `lib.rs::version` |
| `/projects` | GET | `[{name, createdAt}]` | `api.rs::list_projects` |
| `/projects` | POST | `{name, createdAt}` | `api.rs::create_project` |
| `/projects/:p/commits?branch=` | GET | `[commit_json]` | `api.rs::list_commits` |
| `/projects/:p/commits/:hash` | GET | the OKF document | `api.rs::get_commit` |
| `/projects/:p/commits/:hash/record` | GET | the commit record (provenance) | `api.rs::get_commit_record` |
| `/projects/:p/commits/:hash/references[/resolve]` | GET | typed subsystem references / resolve state | `api.rs::list_references`, `resolve_references` |
| `/projects/:p/commits/:hash/checks` | GET | `{commit, checked, checks[]}` | `gate_api.rs::commit_checks` |
| `/projects/:p/branches` | GET | `[{name, tip}]` | `api.rs::list_branches` |
| `/projects/:p/gate-runs` | GET | recorded gate runs | `gate_api.rs::list_gate_runs` |
| `/projects/:p/import/:h/report` | GET | `{artifactHash, bindingId, bindingVersion, lossReport, fidelity}` | `binding_api.rs::import_report` |
| `/projects/:p/import/:h/artifact` | GET | retained source bytes, verbatim | `binding_api.rs::get_artifact` |
| `/projects/:p/audit` | GET | append-only audit log | `audit_api.rs::list_audit` |
| `/projects/:p/analytics?requirements=` | GET | `{project, commit, branch, compliance, costs}` | `analytics_api.rs::project_analytics` |
| `/projects/:p/proposals` | GET | proposals with decisions | `proposal_api.rs::list_proposals` |
| `/projects/:p/locks` | GET | live leases | `locks_api.rs::list_locks` |

The commit record is the single most relevant shape for the schema. `server/src/api.rs::commit_json`
returns `hash, project, branch, parents, okfHash, author, message, createdAt, provenance`.

**There is no OpenAPI document anywhere in the repository** (verified: no `openapi`/`swagger`
file under `server`, `cli`, `engine`, `docs`, `spec` or `website`). The REST API exists in
`server/src/lib.rs`; the OpenAPI document is **to be written** in Package 3.

### 1.2 The `mw` CLI

`cli/src/main.rs` (the `Command` enum and `USAGE`). Commands today: `project create|list`,
`commit`, `log`, `artifact`, `branch list|create|delete`, `merge`, `reset`, `gate`,
`lock acquire|release|list`, `audit`. Two transports: `--server <url>` (`cli/src/http.rs`) or
`--db <path>` (`cli/src/offline.rs`). **No `mw analytics` group exists** — Package 4 adds it.

### 1.3 The MCP server (`mw-mcp`)

The MCP server lives at `engine/mcp` (the brief's "bindings/mcp" is wrong). Its tool table is in
`engine/mcp/src/lib.rs`. 4 document-level tools (`okf.validate`, `graph.stats`, `gate.run`,
`okf.diff`) plus 17 repository tools, all named `repo.*` (`engine/mcp/src/repository.rs::run_tool`):

`repo.projects`, `repo.branches`, `repo.commits`, `repo.read`, `repo.find`, `repo.element`,
`repo.coverage`, `repo.references`, `repo.importReport`, `repo.lossSummary`, `repo.artifact`,
`repo.diff`, `repo.audit`, `repo.checks`, `repo.proposals`, `repo.analytics`, `repo.propose`.

The tools the analytics schema reads from are `repo.coverage` (calls
`graph::requirement_coverage`), `repo.lossSummary` (aggregates the loss report), `repo.analytics`
(calls `GET /analytics`), `repo.checks` (recorded gate runs). Package 5 **extends `repo.*`** — no
new `mw_*` family. There is also an npm wrapper at `packages/modelwrite-mcp` (installer only;
the Rust source is `engine/mcp`).

### 1.4 The engine's analytics modules

- `engine/graph/src/lib.rs` — `graph_stats` → `GraphStats{node_count, edge_count, isolated,
  component_count, component_sizes}`; `requirement_coverage` → `CoverageReport{total, satisfied,
  refined, verified, allocated, covered, uncovered}`. These are the graph-gaps measures.
- `engine/okf/src/validate.rs` — validation errors/warnings, including **dangling edge endpoints**
  (`validate.rs:185-216`).
- `engine/analytics/src/compliance.rs` — `Compliance{Covered, Uncovered, Unknown}` and
  `portfolio_report`.
- `engine/analytics/src/cost.rs` — `Cost{Uncosted, Costed(FusedValue<Money>)}`.
- `engine/analytics/src/confidence.rs` — `Value`, `FusedValue{value, trust, sources,
  captured_at, trusts}`, `TrustLevel{Measured, Reported, Estimated}` (weakest-link).
- `engine/analytics/src/source.rs` — `SourceKind{Model, Structured, Text}`, `TrustLevel`.
- `engine/gate/src/lib.rs` — `run` → gate evidence JSON (round-trip, integration, coverage,
  validation, `passed`).
- `engine/analytics/src/metrics.rs` — **added in this phase**: the metric catalog (see §6).

### 1.5 The store types (the source of the `projects` and `commits` tables)

`server/src/store/mod.rs`: `Project{name, created_at}`; `Commit{hash, project, branch, parents,
okf_hash, author, message, created_at, provenance}`; `CommitProvenance{Authored | Imported{…} |
Accepted{…} | Unknown}`; `ImportRecord{artifact_hash, project, binding_id, binding_version,
loss_report, fidelity_diff, commit_hash, accepted_losses, created_at}`.

---

## 2. The schema, adjusted

One schema, `mw-analytics-schema@1`. **Nine tables ship in v1; `findings` is deferred to v1.1**
(§9.1). Column names are snake_case at rest (CSV/Parquet/Arrow); the REST transport applies the
platform's camelCase convention at the wire (Package 3), with an explicit name map in the spec.

### 2.1 `projects` — one row per project

| Column | Type | Key | Source |
|---|---|---|---|
| `project` | string | PK | `Project.name` (`store/mod.rs`) |
| `created_at` | string | | `Project.created_at` (`store/mod.rs`) |

**Dropped from the brief:** `id` (there is no separate id — the name is the identity) and
`default_branch` (the store has no default-branch concept; "main" is only a query fallback in
`list_commits`/`resolve_commit`).

### 2.2 `commits` — one row per commit

| Column | Type | Key | Source |
|---|---|---|---|
| `hash` | string | PK | `Commit.hash` |
| `project` | string | | `Commit.project` |
| `branch` | string | | `Commit.branch` |
| `parents` | list<string> | | `Commit.parents` |
| `author` | string | | `Commit.author` |
| `message` | string | | `Commit.message` |
| `committed_at` | string | | `Commit.created_at` |
| `okf_hash` | string | | `Commit.okf_hash` (the canonical content hash) |
| `provenance_kind` | string (authored\|imported\|accepted\|unknown) | | `CommitProvenance` serde tag `kind` |
| `import_artifact_hash` | string\|null | | `CommitProvenance::Imported.artifact_hash` |
| `binding_id` | string\|null | | `CommitProvenance::Imported.binding_id` |
| `binding_version` | string\|null | | `CommitProvenance::Imported.binding_version` |

`okf_hash` is the **evidence hash** carried on every metric row (§5). The per-import columns are
null for authored/unknown commits; `import_artifact_hash` is the brief's "import id if any".

### 2.3 `elements` — one row per element per commit

Source is `okf::types::OkfRoot` (`engine/okf/src/types.rs`): the element sections
`structure`, `interfaces`, `signals` (each `Vec<Element>`).

| Column | Type | Key | Source |
|---|---|---|---|
| `project` | string | | `OkfRoot.project` |
| `commit` | string | PK | `Commit.hash` |
| `element_id` | string | PK | `Element.id` |
| `section` | string (structure\|interfaces\|signals) | | which `Vec<Element>` held it |
| `name` | string | | `Element.name` |
| `kind` | string | | `Element.kind` |
| `stereotypes` | list<string> | | `Element.stereotypes` |
| `attributes` | list<{name,type,aggregation,default}> | | `Element.attributes` (`Attribute`) |
| `documentation` | string | | `Element.documentation` |

**Dropped from the brief:** `qualified name` (OKF carries none — §9.3), `owner id` (the OKF
`Element` has no owner), per-element `provenance kind`/`source binding`/`source artifact hash`
(these are per-commit/per-import, not per-element — join through `commits`).

### 2.4 `relationships` — one row per graph edge per commit

Source is `OkfRoot.graph.edges` (`Vec<GraphEdge>`, `engine/okf/src/types.rs`).

| Column | Type | Key | Source |
|---|---|---|---|
| `project` | string | | `OkfRoot.project` |
| `commit` | string | PK | `Commit.hash` |
| `source` | string | PK | `GraphEdge.source` |
| `target` | string | PK | `GraphEdge.target` |
| `kind` | string | PK | `GraphEdge.kind` |
| `label` | string | | `GraphEdge.label` |

The graph edge is the full relationship inventory: `kind` is the edge kind (association, dependency,
contains, part, …), `label` carries the traceability word (Satisfy/Refine/Verify/Allocate) on
dependency edges. Edges may legitimately repeat, so the identity is the full tuple.

### 2.5 `requirements` — one row per requirement per commit

Source is `OkfRoot.requirements` (`Vec<Requirement>`), joined with the engine's coverage.

| Column | Type | Key | Source |
|---|---|---|---|
| `project` | string | | `OkfRoot.project` |
| `commit` | string | PK | `Commit.hash` |
| `element_id` | string | PK | `Requirement.id` |
| `identifier` | string | | `Requirement.req_id` |
| `text` | string | | `Requirement.req_text` |
| `name` | string | | `Requirement.name` |
| `kind` | string | | `Requirement.kind` |
| `covered` | boolean | | derived: `element_id ∉ CoverageReport.uncovered` (`graph::requirement_coverage`) |
| `covering_link_kinds` | list<string> | | the `label`s of dependency edges whose requirement endpoint is `element_id` |

`covered` and `covering_link_kinds` are engine outputs (the coverage report and the graph edges),
not invented columns. A requirement absent from the model has **no row** — that is the UNKNOWN
state, expressed explicitly by `compliance.unknown_count` (§5), never as a blank row.

### 2.6 `trace_links` — one row per coverage edge per commit

The coverage subset of `relationships`: dependency edges whose label is Satisfy/Refine/Verify/
Allocate, with the requirement endpoint resolved — exactly the edges
`graph::requirement_coverage` reads.

| Column | Type | Key | Source |
|---|---|---|---|
| `project` | string | | `OkfRoot.project` |
| `commit` | string | PK | `Commit.hash` |
| `requirement_id` | string | PK | the endpoint that is a requirement id |
| `element_id` | string | | the other endpoint |
| `link_kind` | string (Satisfy\|Refine\|Verify\|Allocate) | PK | `GraphEdge.label` |
| `source_id` | string | | `GraphEdge.source` |
| `target_id` | string | | `GraphEdge.target` |

### 2.7 `metrics` — one row per metric per commit

**The metrics table does not exist as a stored table today; it is the uniform projection of the
engine's five computations** (`graph_stats`, `requirement_coverage`, `portfolio_report`,
`cost_by_requirement`, `gate::run`) into one row shape. No metric is invented; every value is an
engine output. It is derived, read-only, and cached per (commit, metric, engine version) per design
decision 4 (a transport concern for Package 3+).

| Column | Type | Key | Source |
|---|---|---|---|
| `project` | string | PK | `OkfRoot.project` / `Commit.project` |
| `commit` | string | PK | `Commit.hash` |
| `metric_id` | string | PK | `analytics::metrics::ENGINE_METRIC_IDS` |
| `value` | number\|null | | the engine scalar; null only when `value_state` ≠ present |
| `value_state` | string (present\|unknown\|uncosted) | | present for scalars; unknown/uncosted for the compliance/cost states |
| `unit` | string | | constant per metric_id (count / bool / currency) |
| `trust` | string (measured\|reported\|estimated) | | `TrustLevel` — **weakest link, never blank** |
| `basis_element_count` | integer | | the element count the metric was computed over |
| `basis_relationship_count` | integer | | the relationship count the metric was computed over |
| `constructs_not_carried` | integer | | `LossReport.content_losses().len()` (0 for authored commits) |
| `basis_note` | string | | `ImportSummary.statement()` / gate evidence note |
| `engine_version` | string | | `env!("CARGO_PKG_VERSION")` of the computing crate |
| `evidence_hash` | string | | `Commit.okf_hash` / `okf::hash::canonical_hash` |

### 2.8 `metric_definitions` — one row per metric (GENERATED)

| Column | Type | Key | Source |
|---|---|---|---|
| `id` | string | PK | `MetricDefinition.id` (`analytics/src/metrics.rs`) |
| `name` | string | | `MetricDefinition.name` |
| `description` | string | | `MetricDefinition.description` |
| `depends_on` | list<string> | | `MetricDefinition.depends_on` |
| `status` | string (active) | | `MetricDefinition.status` |

**Generated, never hand-written** (§6): `engine/analytics/examples/gen_metric_definitions.rs`
serialises `metric_definitions()` to `spec/analytics/metric_definitions.json`.

### 2.9 `import_losses` — one row per loss-report mapping

Source is `binding::LossReport.mappings` (`engine/binding/src/report.rs`): one `Mapping{subject,
verdict, note}` per entry. **One row per mapping — nothing is dropped** (MCP ruling 3: aggregation
is a view, not a new truth).

| Column | Type | Key | Source |
|---|---|---|---|
| `import_artifact_hash` | string | PK | `LossReport.artifact_hash` / `ImportRecord.artifact_hash` |
| `project` | string | PK | `ImportRecord.project` |
| `binding_id` | string | | `LossReport.binding.id` |
| `binding_version` | string | | `LossReport.binding.version` |
| `construct` | string | | `construct_of(subject)` (the leading token, e.g. `uml:Port`) |
| `subject` | string | | `Mapping.subject` (the full named construct — carries the source id) |
| `severity` | string (exact\|lossy\|unmappable) | | `Mapping.verdict` |
| `note` | string | | `Mapping.note` |

The brief's "count" and "example ids" are the **aggregated view** (`repo.lossSummary` groups by
construct+verdict with counts); `subject` is the "example id" (the id is embedded in the subject,
e.g. `uml:Port _2026x_…`). Aggregation is documented as a view over this table, not a stored column.

---

## 3. The metrics table reconciled with the analytics design

The design spec (`docs/superpowers/specs/2026-09-17-modelwrite-analytics-design.md`) rules that
**confidence is the weakest link and is never blank**, and that **UNKNOWN/UNCOSTED are never blank**.
The metrics table honours both:

- **`trust` is always present, never blank.** Model-derived metrics (graph, coverage, gate) are
  `measured` (`SourceKind::Model` → `TrustLevel::Measured`, `source.rs`). A cost metric is the
  `FusedValue.trust` — the weakest link across every contributing dataset
  (`confidence.rs::weakest`), so one estimated input makes the whole row `estimated`.
- **`value_state` is always present.** A scalar count is `present`. An absent requirement is
  `unknown` (never a blank and never a zero — `compliance.rs`), an unpriced requirement is
  `uncosted` (never a zero — `cost.rs`). `value` is null exactly when the state is
  unknown/uncosted; the state column carries the meaning, so absence can never read as a number.
- The three compliance states and the two cost states are surfaced as **aggregate counts that make
  absence explicit as a number**: `compliance.unknown_count` and `cost.uncosted_count` — so a
  BI total that ignores them is visibly incomplete, never silently wrong.

### Basis columns — on every metric row, as data

A metric without a basis is a defect (design decision 2). Every `metrics` row carries:

- `basis_element_count` — e.g. `node_count` (graph), `total` (coverage), `specification_size`
  (compliance), requirement count (cost).
- `basis_relationship_count` — e.g. `edge_count` (graph), the dependency-edge count (coverage), 0
  (compliance/cost).
- `constructs_not_carried` — the import's `content_losses().len()` for imported commits, 0 for
  authored commits. This is the number of constructs the source could not carry into the model the
  metric was computed over.
- `basis_note` — the import summary statement or the gate evidence note, in prose.
- `engine_version` — the version of the crate that computed the metric.
- `evidence_hash` — the canonical content hash of the model (`Commit.okf_hash`).

---

## 4. WHAT THE GRAPH-GAPS ANALYTIC NEEDS (ruling 4)

Ruling 4 of the open/licensed boundary makes the graph-gaps analytic the open tier's headline:
**orphans, isolated groups, dangling links — named, addressable, diffed over baselines.** The
schema carries what that analytic needs in two layers:

**The raw graph** — so any consumer computes the gaps itself:
- `elements` (element_id, section, name, kind) names every node;
- `relationships` (source, target, kind, label) carries every edge.

**The engine's precomputed gap measures** — so the overview page and a BI tool agree, each with basis:
- `graph.isolated_count` — **orphans** (nodes with no incident edge). Named list:
  `GraphStats.isolated` (`graph.rs`).
- `graph.component_count` — **isolated groups** (more than one connected component). Sizes:
  `GraphStats.component_sizes`.
- `coverage.uncovered` + `requirements.covered=false` — **dangling/uncovered requirements**. Named
  list: `CoverageReport.uncovered`.
- `gate.validation_warnings` — **dangling links** (edges whose source/target is not a node,
  `validate.rs:185-216`) and unresolved subsystem references (`api.rs::resolve_references`).

The trend line is `metrics` joined to `commits` (committed_at, parents) on `commit` — one metric
row per baseline, each with its `evidence_hash` and basis, so "coverage trending towards a design
review" (boundary item 4) is a plain query.

**One honest gap:** the engine returns the **count and sizes** of disconnected components, but not
per-node component **membership**. "Isolated groups" can therefore be counted and sized today, but
naming which elements form each group needs a small, additive engine addition (expose the
union-find membership in `graph_stats`). This is a candidate for v1.1, additive-only; the raw
`elements` + `relationships` tables already let any consumer compute membership itself. The
schema does not invent a membership column the engine does not carry.

---

## 5. Versioning rule

Within a major version, changes to `mw-analytics-schema` are **ADDITIVE ONLY**: columns and tables
may be added, never removed, renamed or re-typed. Every response and every exported file carries the
schema version (`mw-analytics-schema@1`). A consumer pinned to `@1` can always read a later
`@1.x` payload.

---

## 6. The metric-definitions generator

`metric_definitions` is generated from the engine, not hand-written:

- `engine/analytics/src/metrics.rs` — `ENGINE_METRIC_IDS` (the closed list of 25 engine metrics)
  and `metric_definitions()` (the catalog).
- `engine/analytics/examples/gen_metric_definitions.rs` — writes
  `spec/analytics/metric_definitions.json`.
- `engine/analytics/tests/metric_definitions.rs` — **fails when an engine metric has no definition
  row** (and when a definition has no engine metric behind it — rule 7: nothing is invented).

The analytics page and the table therefore read one list and can never disagree.

---

## 7. Licence (flag for decision D1)

The schema files carry an **Apache-2.0** SPDX header (permissive, as the brief's Package 2 and the
boundary's ruling 3 intend). **This does not match the interchange licence the project actually
uses today**: the workspace is `AGPL-3.0-or-later` (`Cargo.toml`), every `.rs` file carries
`SPDX-License-Identifier: AGPL-3.0-or-later`, and the OKF spec (`docs/okf/okf-1.0-spec.md`)
carries **no licence header at all**. The premise "matching the interchange licence the project
already uses" is therefore not borne out — there is no existing Apache-2.0 interchange licence.
**This is decision D1 for the owner**: adopt the permissive Apache-2.0 header as drafted (open
connectors, per boundary ruling 3), or re-header to AGPL to match the code. Flagged here, not
decided unilaterally.

---

## 8. Package 5 item 7 — rule-pack mechanism

`agents/generate.py` **does not exist** (verified: `agents/` contains only `CLAUDE.md`). The rule
packs are the invariants in `agents/CLAUDE.md`. Package 5's "change is in all generated packs via
`agents/generate.py`" therefore needs a real mechanism: either the generator is written, or the
basis-rule is added to `agents/CLAUDE.md` directly. Noted for Package 5, not built here.

---

## 9. Corrections, verified — and one that does not survive verification

### 9.1 Findings deferred to v1.1

The analyses slice is not built; there are no findings in the platform yet (the gate produces
`failures` strings and validation warnings inside evidence, but no structured findings table).
**`findings` is deferred to v1.1** under the additive-only rule, and no empty findings table ships
in v1. Nine tables ship.

### 9.2 Correction 9 is contradicted by the code

The impact analysis states "there is no viewer role in auth today (admin-or-nothing)". **The code
says otherwise**: `server/src/auth.rs` defines `Permission{Read, Write, Review, Administer}` with
`granted_roles()` mapping `Read → ["viewer", "author", "reviewer", "admin"]` and an explicit
"viewer reads" role. The "admin-or-nothing" is true only of the **Open-mode default**
(`Identity::open()` = anonymous admin, `auth.rs:35`), which is what the idc-1 trial runs
(`MW_ALLOW_OPEN=yes`, `docs/deploy/idc-1-trial.md`). The auth **model** already has a viewer
role; the **deployment** does not. This tranche still adds no roles (correct), and the read-only
surface sits behind the existing auth (correct) — but the proposal records the viewer role as
**existing in the code**, with licensed-tier row-level security (SSO, classification marking, pull
audit) as the later work.

### 9.3 Qualified name dropped

OKF carries no qualified name: `Element` is `{id, name, kind, stereotypes, attributes,
documentation}` (`engine/okf/src/types.rs`) with no owner and no namespace. The SysML v2 reader
reports package-level structure as a **named Lossy** mapping — `engine/binding-sysmlv2/src/lib.rs:496-501`
records `"package {qname}"` with verdict Lossy, note "OKF has no package/namespace concept". The
brief's own rule 7 would delete `qualified name`; it is dropped, and `name` stands in its place.

### 9.4 Other corrections confirmed

- **No `bindings/` directory** — Python bindings are greenfield (Package 6).
- **`mw-mcp` at `engine/mcp`** — tools are `repo.*` (17) + 4 document tools.
- **No OpenAPI document** — written in Package 3.
- **No `server/src/qa_pipeline.rs`** — the import/artifact path is `binding_api.rs` + the store's
  `ImportRecord` (retain artifact → binding read → round-trip fidelity → loss report → commit).
- **No `agents/generate.py`** — rule packs are `agents/CLAUDE.md` (§8).
- **TMT is not a loadable project** — 36 MB / 255k-line XMI; the offline measurement
  (`engine/binding-xmi/tests/real_tmt.rs`) pins 362 blocks, 7 requirements, 369 nodes, 633 edges,
  45,725 content losses + 2,828 lossy = 48,553 blocking, 2,231 declarations. Tests must use the
  coffee-machine XMI import (the corpus carries 49 structure / 25 requirements / 9 signals / 8
  activities / 99 graph nodes / 165 edges — verified from
  `sample/corpus/coffee-machine/okf/expected/coffee_machine_model.json`). No TMT-loaded trial is
  claimed.
- **Trial is open-mode with hourly reset** — `docs/deploy/idc-1-trial.md` (`OnCalendar=hourly`).
  Trend history on the trial is at most one hour; a real trend needs a persistent deployment.

---

## 10. Owner's approval

This proposal ships no transport code and starts nothing in Packages 3-8.

> **Approve mw-analytics-schema@1 as proposed? Transport packages 3-8 start only after your yes.**

