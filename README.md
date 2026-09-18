# Modelwrite

**The model you can prove.**

Modelwrite is an open-source MBSE platform that turns legacy SysML models
into portable, provable data — the Open Knowledge Format (OKF) — and
automates the skilled work of building and reviewing models with grounded
AI.

## What is in this repository

- engine/ — the Rust engine (AGPL-3.0-or-later): okf types, validation,
  hashing and diffing; graph analysis; the round-trip fidelity gate with
  evidence records; the C ABI; the MCP server
- judge/ — the gate-as-judge harness for scoring model migrations
- agents/ — rule packs for AI coding agents working against OKF
- sample/ — the coffee-machine corpus: the legacy CATIA Magic model, its
  OKF export, and the corrupted fixture the gate must reject
- docs/ — the OKF 1.0 spec, the design documents, and the evidence annex
- deploy/ — the container image, a Docker Compose trial stack, and a Helm chart

## Quickstart

Requirements: Rust (stable, via rustup) and Python 3.12+ for the judge
harness.

cargo build --workspace
cargo test --workspace

Run the gate on the corpus:

cargo run -p mw-gate -- --reference sample/corpus/coffee-machine/okf/expected/coffee_machine_model.json --candidate sample/corpus/coffee-machine/okf/expected/coffee_machine_model.json

Expected output: GATE PASS.

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

## Licence

The engine is AGPL-3.0-or-later. Importers and exporters will ship under
Apache-2.0 as they land. See NOTICE.md for attribution.