# Modelwrite

**The model you can prove.**

Modelwrite is an open-source MBSE platform that turns legacy SysML models
into portable, provable data — the Open Knowledge Format (OKF) — and
automates the skilled work of building and reviewing models with grounded
AI.

## Documentation

The user guide lives in [docs/guide/](docs/guide/README.md). It covers what
modelwrite is and is not, the mental model (concepts), the click paths (tasks),
the MCP contract for AI agents (agents), the command line (cli), the honest FAQ, the
standing limits, and troubleshooting. Start at docs/guide/README.md.

## Website

The project website is hand-written HTML and CSS in `website/` — no build
step, no external requests. It is deployed to GitHub Pages by
`.github/workflows/pages.yml` on push to `main`.

## What is in this repository

- engine/ — the Rust engine (AGPL-3.0-or-later): okf types, validation,
  hashing and diffing; graph analysis; the round-trip fidelity gate with
  evidence records; the C ABI; the MCP server
- judge/ — the gate-as-judge harness for scoring model migrations
- agents/ — rule packs for AI coding agents working against OKF
- sample/ — the coffee-machine corpus: the legacy CATIA Magic model, its
  OKF export, and the corrupted fixture the gate must reject
- docs/ — the OKF 1.0 spec, the design documents, and the evidence annex
- server/ — the `mw-server` HTTP service (SQLite or PostgreSQL backend)
- cli/ — the `mw` command line, which reaches the service over plain HTTP or
  opens a SQLite store directly for offline use (see [docs/guide/cli.md](docs/guide/cli.md)):
  `mw --db modelwrite.db project create coffee`
- deploy/ — the container image, a Docker Compose trial stack, and a Helm chart

## Quickstart

Requirements: Rust (stable, via rustup) and Python 3.12+ for the judge
harness.

cargo build --workspace
cargo test --workspace

Run the gate on the corpus:

cargo run -p mw-gate -- --reference sample/corpus/coffee-machine/okf/expected/coffee_machine_model.json --candidate sample/corpus/coffee-machine/okf/expected/coffee_machine_model.json

Expected output: GATE PASS.

## Install and run

The `mw` command line ships as a prebuilt binary on the GitHub releases page
(`mw-linux-x64`, `mw-linux-arm64`, `mw-darwin-x64`, `mw-darwin-arm64`,
`mw-windows-x64.exe`), or build it from this repository with `cargo install --path cli`.
Both modes — HTTP and offline — are documented in [docs/guide/cli.md](docs/guide/cli.md).

The `mw-cli` crate is prepared for crates.io (metadata and version are in place) but is not
published yet: publishing it requires first re-releasing the engine layer at a bumped
version, because the engine crates already on crates.io predate the subsystem-reference and
write-path-convergence work.

- **Crates** — `cargo build --workspace` (see Quickstart above). The service is the
  `mw-server` crate; the CLI is the `mw-cli` crate, which builds the `mw` binary.
- **Container image** — `docker build -f deploy/Dockerfile -t
  modelwrite/modelwrite:0.1.0 .` (see the next section and deploy/README.md).

## Run the service

Deploy it as a container, a Compose trial, or a Helm release:

docker build -f deploy/Dockerfile -t modelwrite/modelwrite:0.1.0 .
docker compose -f deploy/docker-compose.yml up --build   # a trial on one machine
helm install modelwrite deploy/helm/modelwrite          # a real install

**The service runs in OPEN MODE (anonymous admin) when no authentication is
configured. The image refuses to start that way unless you set
`MW_ALLOW_OPEN=yes`; for anything shared, set `MW_AUTH_TOKEN` or `MW_AUTH_JWKS`
instead.** See deploy/README.md for the full story, including the air-gapped
install path and what the chart does not do (TLS, backups, HA).

The JWKS file is read once at startup, so adding or removing a signing key
requires restarting the service to take effect.

## Licence

The engine is AGPL-3.0-or-later. Importers and exporters will ship under
Apache-2.0 as they land. See NOTICE.md for attribution.