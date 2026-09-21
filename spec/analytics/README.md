<!-- SPDX-License-Identifier: Apache-2.0 -->
# mw-analytics-schema@1 — specification

Version: **1**. Status: draft artefact (Packages 1-2 of the analytics-interfaces brief).
Licence: Apache-2.0 (see `LICENSE` below and decision D1 in the proposal).

## What this is

A single, versioned, read-only analytics schema for the modelwrite platform. The same tables are
delivered through every transport (REST, CLI, MCP, Python) so a BI tool, a notebook, a data-lake
job or an agent can read the model and its measures without glue code.

## Versioning rule

Within a major version, changes are **ADDITIVE ONLY**: columns and tables may be added, never
removed, renamed or re-typed. Every response and every exported file carries the schema version
(`mw-analytics-schema@1`). A consumer pinned to `@1` can always read a later `@1.x` payload.

## Naming

Logical column names are snake_case at rest (CSV, Parquet, Arrow). The REST transport serialises
the same logical columns in the platform's camelCase convention at the wire (Package 3); the
mapping is mechanical (e.g. `basis_element_count` ↔ `basisElementCount`) and documented in the
transport's OpenAPI document.

## Tables (v1 — nine tables)

| Table | One row per | Keys |
|---|---|---|
| `projects` | project | `project` |
| `commits` | commit | `hash` |
| `elements` | element per commit | `(commit, element_id)` |
| `relationships` | graph edge per commit | `(commit, source, target, kind, label)` |
| `requirements` | requirement per commit | `(commit, element_id)` |
| `trace_links` | coverage edge per commit | `(commit, requirement_id, link_kind)` |
| `metrics` | metric per commit | `(commit, metric_id)` |
| `metric_definitions` | metric | `id` |
| `import_losses` | loss-report mapping | `(import_artifact_hash, project)` |

`findings` is **deferred to v1.1** (the analyses slice is not built; there are no findings to ship).

## Artefacts in this directory

- `README.md` — this prose specification.
- `metric_definitions.json` — the `metric_definitions` table, **generated** from
  `engine/analytics/src/metrics.rs` (run `cargo run -p mw-analytics --example gen_metric_definitions`).
- `jsonschema/<table>.schema.json` — a JSON Schema (draft-07) per table.
- `arrow/<table>.arrow.json` — an Arrow schema per table, expressed in the Arrow DataType
  vocabulary (utf8, int64, bool, float64, null, list<…>, struct<…>). Native Arrow validation/build
  lands with the arrow crate in Package 4.

## The basis rule

Every `metrics` row carries what it was computed over, as data, never only as prose:
`basis_element_count`, `basis_relationship_count`, `constructs_not_carried`, `basis_note`,
`engine_version`, `evidence_hash`. A metric without them is a defect.

## The confidence rule

`metrics.trust` is the weakest link and is never blank (measured / reported / estimated).
`metrics.value_state` is always present and expresses UNKNOWN / UNCOSTED as states, never as
blank and never as zero.

## LICENCE

Copyright 2026 the modelwrite contributors.

Licensed under the Apache License, Version 2.0 (the "License"); you may not use these files except
in compliance with the License. You may obtain a copy of the License at
http://www.apache.org/licenses/LICENSE-2.0

Unless required by applicable law or agreed to in writing, software distributed under the License
is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express
or implied. See the License for the specific language governing permissions and limitations under
the License.

NOTE: the licence here is **decision D1 for the owner** — the proposal flags that the project's
code and OKF spec are AGPL-3.0-or-later / header-less, not Apache-2.0. See §7 of the proposal.
