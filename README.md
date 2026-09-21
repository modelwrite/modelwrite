# Modelwrite

**The model you can prove.**

Modelwrite is the open source analytics platform for getting value from a SysML
investment. It reads the models you already have, measures coverage,
traceability and integration, tracks them baseline by baseline, and answers
questions with the evidence attached. You keep your modelling tool.

## Documentation

The user guide lives in [docs/guide/](docs/guide/README.md). It covers what
modelwrite is and is not, the mental model ([concepts](docs/guide/concepts.md)),
the click paths ([tasks](docs/guide/tasks.md)), the MCP contract for AI agents
([mcp-agents](docs/guide/mcp-agents.md)), the command line
([cli](docs/guide/cli.md)), the honest [FAQ](docs/guide/faq.md), the standing
[limits](docs/guide/limits.md), and [troubleshooting](docs/guide/troubleshooting.md).
Start at docs/guide/README.md.

## Try it

Two tiers, both live:

- **The showcase** — <https://trial.modelwrite.org> — open, no registration. Six
  seeded models (coffee-machine, sandwich-toaster, purchasing-terminal,
  floor-robot, microduck, cafe-stand) with an hourly reset. The cafe-stand
  project holds the two-robot trade study, one branch with the floor-robot on
  floor care and one with microduck.
- **The registered trial** — <https://app.modelwrite.org> — your own private
  workspace, entered with an emailed code (no password), in a rolling 14-day
  window (activity renews it; 14 days idle turns read-only; 21 days idle
  archives). The service is up, but email delivery is still console-only pending
  an SMTP credential, so the website door reads "live, email pending".

## What is in this repository

- engine/ — the Rust engine (AGPL-3.0-or-later): OKF types, validation, hashing
  and diffing; graph analysis; the round-trip fidelity gate with evidence
  records; the XMI and SysML v2 readers; the C ABI; the MCP server; analytics
- server/ — the `mw-server` HTTP service (SQLite or PostgreSQL backend) and the
  workbench UI
- cli/ — the `mw` command line, which reaches the service over plain HTTP or
  opens a SQLite store directly for offline use
- sample/ — the coffee-machine corpus and the open example corpus
  ([sample/examples/](sample/examples/))
- docs/ — the OKF 1.0 spec, the design documents, the evidence annex, and the guide
- spec/analytics/ — the `mw-analytics-schema@1` schema (Apache-2.0)
- website/ — the hand-written site, deployed to GitHub Pages on push to `main`
- deploy/ — the container image, a Docker Compose trial stack, and a Helm chart

## The analytics schema

`mw-analytics-schema@1` ([spec/analytics/](spec/analytics/README.md)) is the
versioned, read-only analytics schema — nine tables, additive-only within a major
version.

<!-- generated:metric-definitions -->
25 metric definitions, listed in [spec/analytics/metric_definitions.json](spec/analytics/metric_definitions.json); the website's analytics section renders the same file, so the list is never hand-duplicated.
<!-- /generated -->

The same tables are delivered through two shipped transports:

- the CLI — `mw analytics` (schema, tables, export to CSV/NDJSON/Parquet,
  metrics, trend);
- the MCP surface — see the generated tool list below.

<!-- generated:mcp-tools -->
The MCP server exposes **26** tools. The names and schemas are contract-enforced against the live tool table and live in [docs/agents/mcp-tools.json](docs/agents/mcp-tools.json):

| Tool | Required arguments | Summary |
|---|---|---|
| `okf.validate` | okf | Validate an OKF document supplied in the request and return the validation report (document-level; no repository, no network) |
| `graph.stats` | okf | Graph health of an OKF document supplied in the request: nodes, edges, isolated nodes, components (document-level; no repository, no network) |
| `gate.run` | reference, candidate | Run the round-trip fidelity gate between two OKF documents supplied in the request and return its evidence record (document-level; no repository, no network) |
| `okf.diff` | reference, candidate | Semantic diff between two OKF documents supplied in the request: elements, edges and attributes (document-level; no repository, no network) |
| `repo.projects` | — | List the projects the configured token may see. Repository mode (opt-in): requires MW_MCP_SERVICE_URL and MW_MCP_TOKEN; read permission. |
| `repo.branches` | project | List a project's branches and their tips. Repository mode (opt-in): read permission. |
| `repo.commits` | project | List the commits on a project's branch (default main). Repository mode (opt-in): read permission. |
| `repo.read` | project, hash | Read the OKF model behind a commit and its commit record (provenance). Repository mode (opt-in): read permission. |
| `repo.find` | project, hash, query | Search a revision's elements by name, id or stereotype fragment, returning each match's id, name and kind. Repository mode (opt-in): read permission. |
| `repo.element` | project, hash, id | Read one element by id from a revision: its attributes and its graph edges. Repository mode (opt-in): read permission. |
| `repo.coverage` | project, hash | Requirement coverage for a revision: covered and uncovered, each named, from the engine's requirement_coverage. Repository mode (opt-in): read permission. |
| `repo.references` | project, hash | A model's typed subsystem references, with an optional resolve flag for each reference's resolve state (the same resolution the UI and API use). Repository mode (opt-in): read permission. |
| `repo.importReport` | project, artifactHash | Read a migration's loss report and the engine's fidelity measurement, by retained artifact hash. Repository mode (opt-in): read permission. |
| `repo.lossSummary` | project, artifactHash | A migration's loss report aggregated by construct and verdict with counts, plus paging over the full list. Repository mode (opt-in): read permission. |
| `repo.artifact` | project, artifactHash | The retained original bytes of a migrated artifact, byte for byte. Repository mode (opt-in): read permission. |
| `repo.diff` | project, reference, candidate | Diff two commits and run the round-trip gate LOCALLY from the two models read from the repository, returning the verdict and the evidence; never records a run (recording is a write). Repository mode (opt-in): read permission. |
| `repo.audit` | project | Read a project's append-only audit log. Repository mode (opt-in): read permission. |
| `repo.checks` | project, hash | Read the gate checks recorded against a commit (what has been checked about this model). Repository mode (opt-in): review permission. |
| `repo.proposals` | project | The project's proposals with their decisions and who made them. Repository mode (opt-in): read permission. |
| `repo.analytics` | project, requirements | The portfolio question: which models meet which requirements, read from the project analytics route. Repository mode (opt-in): read permission. |
| `repo.propose` | project, reviewArtifact | Record a proposal for a human to review and decide. The agent's output: it persists a proposal (review permission) and never writes a change - a human accepts it. Repository mode (opt-in). |
| `repo.analyticsSchema` | — | The analytics schema (mw-analytics-schema@1): its tables, their columns and the metric definitions, so an agent can plan before it queries. Repository mode (opt-in): read permission. |
| `repo.metrics` | project | The metrics for a project at a commit or branch head, each WITH its basis (basis_element_count, basis_relationship_count, constructs_not_carried, basis_note, engine_version, evidence_hash). Repository mode (opt-in): read permission. |
| `repo.trend` | project, metric, branch | One metric across the commits of a branch, in commit order, each value WITH its basis. Repository mode (opt-in): read permission. |
| `repo.table` | project, table | Bounded rows from one analytics table, with key filters and a cursor. Default 50 rows, maximum 500; every page returns the TOTAL row count. Repository mode (opt-in): read permission. |
| `repo.losses` | project | The paged full import-loss list (repo.lossSummary aggregates; this pages the underlying entries). Repository mode (opt-in): read permission. |
<!-- /generated -->

Every metric row carries its basis (what it was computed over) as data, never
only as prose. See [docs/guide/mcp-agents.md](docs/guide/mcp-agents.md) and
[docs/guide/cli.md](docs/guide/cli.md).

## Readers

<!-- generated:binding-list -->
| Binding | id@version | Direction | Reads |
|---|---|---|---|
| SysML v1 (UML profile) XMI | `sysml-v1-xmi@2.4` | read/write | blocks, requirements, properties and Satisfy/Allocate traceability |
| SysML v2 textual notation (.sysml) | `sysml-v2-textual@1.0` | viewer (ImportOnly) | part/attribute/item definitions, requirements, satisfy traceability and documentation |

The workbench import surface (the server's binding registry) offers `sysml-v1-xmi@2.4` and `sysml-v2-textual@1.0`.
<!-- /generated -->

- **XMI (SysML v1)** — import and export through the XMI reader, with a named loss
  report. On the coffee-machine MagicDraw model it carries 41 structure elements,
  25 requirements and 66 graph nodes with 95 edges, and names 252 blocking losses.
  The Thirty Meter Telescope model (a 36&nbsp;MB Cameo export) carries 362 blocks and
  633 edges and round-trips exactly, naming 48,553 blocking losses (45,725 unmappable
  + 2,828 lossy) for the constructs it does not carry. Reproduce with
  `cargo test -p mw-binding-xmi --test real_magicdraw`
  and `cargo test -p mw-binding-xmi --test real_tmt`.
- **SysML v2 textual** — a stated-subset **viewer**: it imports and does not write
  back. Reproduce with `cargo test -p mw-binding-sysmlv2 --test real_examples`.

## STPA/STAMP completeness — in progress, not deployed

The STPA/STAMP completeness checks (unanalysed control actions, control loops with
no feedback, hazards and constraints that do not reach, UCAs with no loss scenario,
and baseline trend) are being built in `engine/graph/src/stpa.rs` with fixtures in
`sample/stpa/`. They are **not** deployed — the live trial runs an earlier commit —
and no claim here or on the site should read them as shipped.

## Quickstart

Requirements: Rust (stable, via rustup).

    cargo build --workspace
    cargo test --workspace

Run the gate on the corpus:

    cargo run -p mw-gate -- --reference sample/corpus/coffee-machine/okf/expected/coffee_machine_model.json --candidate sample/corpus/coffee-machine/okf/expected/coffee_machine_model.json

Expected output: `GATE PASS`.

## Install and run

The `mw` command line installs from crates.io (`cargo install mw-cli`) or builds
from this repository (`cargo install --path cli`). Both modes — HTTP and offline —
are documented in [docs/guide/cli.md](docs/guide/cli.md).

Twelve crates are published on crates.io at 0.2.0: `mw-okf`, `mw-graph`,
`mw-gate`, `mw-binding`, `mw-binding-xmi`, `mw-agent`, `mw-capi`,
`mw-mcp`, `mw-analytics`, `mw-test-support`, `mw-server` and `mw-cli`.
The SysML v2 viewer (`mw-binding-sysmlv2`) is at 0.1.0 in this repository and is
not yet published.

- **Container image** — `docker build -f deploy/Dockerfile -t modelwrite/modelwrite:0.1.0 .` (see deploy/README.md).

## Run the service

    docker build -f deploy/Dockerfile -t modelwrite/modelwrite:0.1.0 .
    docker compose -f deploy/docker-compose.yml up --build   # a trial on one machine
    helm install modelwrite deploy/helm/modelwrite          # a real install

**The service runs in OPEN MODE (anonymous admin) when no authentication is
configured. The image refuses to start that way unless you set
`MW_ALLOW_OPEN=yes`; for anything shared, set `MW_AUTH_TOKEN` or `MW_AUTH_JWKS`
instead.** See deploy/README.md for the full story, including the air-gapped
install path and what the chart does not do (TLS, backups, HA).

## Licence

<!-- generated:crate-licences -->
| Crate | Licence |
|---|---|
| `mw-agent` | AGPL-3.0-or-later |
| `mw-analytics` | AGPL-3.0-or-later |
| `mw-binding` | AGPL-3.0-or-later |
| `mw-binding-sysmlv2` | AGPL-3.0-or-later |
| `mw-binding-xmi` | AGPL-3.0-or-later |
| `mw-capi` | AGPL-3.0-or-later |
| `mw-cli` | AGPL-3.0-or-later |
| `mw-gate` | AGPL-3.0-or-later |
| `mw-graph` | AGPL-3.0-or-later |
| `mw-mcp` | AGPL-3.0-or-later |
| `mw-okf` | AGPL-3.0-or-later |
| `mw-server` | AGPL-3.0-or-later |
| `mw-test-support` | AGPL-3.0-or-later |
<!-- /generated -->

The analytics schema (`spec/analytics/`) is **Apache-2.0**. Project policy (see
CONTRIBUTING.md) designates importers and exporters as Apache-2.0 as they land; the
table above states what each crate carries today. See NOTICE.md for attribution.

## Limits

The standing limits are in [docs/guide/limits.md](docs/guide/limits.md). In short:
no numeric or physical simulation; the native XMI-to-OKF read is not independently
measured; typed cross-model edges resolve per edge, but the global system-of-systems
graph property is asserted rather than computed; and the showcase trial is
open-mode and ephemeral.
