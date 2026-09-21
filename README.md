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
  an SMTP credential, so the website door reads "rolling out".

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
versioned, read-only analytics schema — nine tables and 25 metric definitions,
additive-only within a major version. The same tables are delivered through two
shipped transports:

- the CLI — `mw analytics` (schema, tables, export to CSV/NDJSON/Parquet,
  metrics, trend);
- the MCP surface — `repo.analyticsSchema`, `repo.metrics`, `repo.trend`,
  `repo.table` and `repo.losses`.

Every metric row carries its basis (what it was computed over) as data, never
only as prose. See [docs/guide/mcp-agents.md](docs/guide/mcp-agents.md) and
[docs/guide/cli.md](docs/guide/cli.md).

## Readers

- **XMI (SysML v1)** — import and export through the XMI reader, with a named loss
  report. On the coffee-machine MagicDraw model it carries 41 structure elements,
  25 requirements and 66 graph nodes with 95 edges, and names 252 blocking losses.
  The Thirty Meter Telescope model (a 36&nbsp;MB Cameo export) carries 362 blocks and
  633 edges and round-trips exactly, naming 45,725 losses for the constructs it does
  not carry. Reproduce with `cargo test -p mw-binding-xmi --test real_magicdraw`
  and `cargo test -p mw-binding-xmi --test real_tmt`.
- **SysML v2 textual** — a stated-subset **viewer**: it imports and does not write
  back. Reproduce with `cargo test -p mw-binding-sysmlv2 --test real_examples`.

## STPA/STAMP completeness — in progress, not shipped

The STPA/STAMP completeness checks (unanalysed control actions, control loops with
no feedback, hazards and constraints that do not reach, UCAs with no loss scenario,
and baseline trend) exist as uncommitted work in `engine/graph/src/stpa.rs` with
fixtures in `sample/stpa/`. They are **not** committed, shipped or deployed, and
no claim here or on the site should read them as such.

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

The implementation — the engine crates, the server and the CLI — is
**AGPL-3.0-or-later**. The analytics schema (`spec/analytics/`) is **Apache-2.0**.
Project policy (see CONTRIBUTING.md) designates importers and exporters as
Apache-2.0 as they land. See NOTICE.md for attribution.

## Limits

The standing limits are in [docs/guide/limits.md](docs/guide/limits.md). In short:
no numeric or physical simulation; the native XMI-to-OKF read is not independently
measured; an activity's link to a subsystem element is a role name, not a
cross-model edge; and the showcase trial is open-mode and ephemeral.
