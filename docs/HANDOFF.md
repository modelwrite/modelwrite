# Modelwrite — handoff memory

**Written:** 2026-11-16, to move this project off the machine that built it.
**Repo:** https://github.com/modelwrite/modelwrite.git
**Branch:** `slice-2-repository` (identical to `main`; `main` is what deploys the website)
**State at handoff:** 309 commits · all pushed · **748 Rust tests passing** · fmt and clippy clean · no uncommitted files.

Read this first, then `docs/superpowers/specs/` (16 decision records) and `docs/superpowers/plans/` (13 plans). Those files are the reasoning; this is the map.

---

## 1. What this is

**The open-source analytics platform for getting value from a SysML investment.** It reads the models an organisation already has, measures coverage, traceability and integration, names what the model has lost or left unconnected, and tracks it baseline by baseline. The buyer's line: *"Your organisation has spent years building SysML models. Modelwrite shows what they are worth. You keep your modelling tool."*

**Proof is the differentiator, not the headline.** Every number names the command that produced it, and the limits sit at the same weight as the claims. That rule is the product.

## 2. Try it, right now

| URL | What |
|---|---|
| **https://trial.modelwrite.org** | the **Showcase**: open, no registration, seeded models. Resets hourly (see §7) |
| **https://app.modelwrite.org** | the **Registered Trial**: your own isolated workspace for 14 days, extends while you use it. Real email via Postmark |
| **http://modelwrite.org** | the website. Plain **http** — the apex TLS certificate never issued (see §9) |

They run on **idc-1**, not on the machine this was written on.

## 3. Warehouse the code

```
engine/          Rust crates, msrv 1.75 (CI-verified)
  okf/           the document format: canonical JSON, content-addressed, immutable commits
  graph/         coverage, graph stats, layered layout, orthogonal routing, STPA checks, symbols
  gate/          the fidelity gate + deterministic evidence records
  binding/       the Binding trait + the loss report (exact | lossy | unmappable)
  binding-xmi/   XMI/MagicDraw reader AND writer (the only bidirectional one)
  binding-sysmlv2/ SysML v2 textual — ImportOnly VIEWER
  binding-capella/ Capella/Arcadia — ImportOnly VIEWER
  agent/         the loss/identity model
  analytics/     compliance, cost, confidence, metrics
  mcp/           17+ read-only MCP tools (repo.*) over stdio
server/          axum 0.7, msrv 1.88: REST API + server-rendered workbench (maud + inline SVG)
cli/             `mw` — works against a server (--server) or a local store (--db)
bindings/python/ the Python analytics client (thin REST client; Arrow/pandas optional)
website/         static site, no framework, no build step, no external requests
spec/analytics/  the Apache-2.0 analytics schema (9 tables) — the only Apache-2.0 artefact
sample/          corpus, examples, STPA fixtures
docs/            guide, specs, plans, deploy, evidence, okf, stpa, agents
e2e/             Playwright + the browser MEASUREMENT harnesses (see §11)
```

**No build step, no framework, nothing fetched at page load.** The workbench is server-rendered HTML and must stay usable with JavaScript disabled.

## 4. The rules that must not be broken

1. **A claim must be checkable, or labelled unchecked.** This is why there is a Limits page, loss reports, evidence records, and "basis" on every metric.
2. **Nothing is dropped in silence.** Every unmapped construct is NAMED (exact / lossy / unmappable). A migration can never be silently lossy.
3. **The LLM authors and interprets; the ENGINE measures.** No number in any AI-authored prose may come from the model. Enforced in code: see `server/src/ui/dropzone.rs` and the test that scans every digit in its prose.
4. **UNKNOWN is never blank. "Nothing to measure" is not "nothing wrong".** A model with no commits says so; an STPA analysis with no hazards reports **NOT STARTED**, never a clean result.
5. **Commit hashes are canonical.** One serializer (`okf::hash::canonical_bytes`) governs every write path; the same document through any path must produce the same hash.
6. **An agent cannot write.** It proposes; a human accepts. Enforced at 3 layers and denial-tested.
7. **A pinned revision is part of a reference's identity.** Platform models REFERENCE subsystems at pinned commits; they never copy them.
8. **An export is generated from the commit, not the screen**, and names its revision and its scope.

## 5. What is BUILT and working

- The provable commit model (content-addressed, immutable, diffable) + the fidelity gate with deterministic evidence CI regenerates byte for byte.
- The workbench: navigation, structure, requirements, traceability, diagram (structure / process / control, with **orthogonal routing, no edge through a box, and edge labels placed against the drawing**), composition (system-of-systems), **Health** (orphans, isolated groups, dangling links, uncovered requirements — on one screen), **STPA completeness**, **Analyses**, Checks, Changes, Proposals, Import, Assist.
- **Three readers**: XMI (read/write, round-trip measured), SysML v2 textual (viewer), Capella/Arcadia (viewer). Capella means a team can keep doing STPA in Capella — where MIT's STAMP Tools plugin lives — and bring the model here for the check.
- **The low-friction entry**: drop a `.mdzip` / XMI / `.sysml` / `.capella` → the format is detected from the bytes, the project is named from the file, and it reports in plain language what it carried and what it lost, then offers **one button** to accept the losses. It then links to the health view.
- **Analyses frame**: an analysis is a RECORD (definition + run + findings), with a LIBRARY and a DIFF that compares by finding identity.
- **Analytics**: the 9-table schema with REST, CLI, MCP and Python transports, a cross-transport EQUIVALENCE evidence record, byte-identical exports, and DuckDB-verified Parquet with hive partitioning.
- **Two-tier trials**: the open Showcase and the registered trial with per-trial SQLite databases, session isolation (cross-trial refusal proven), real email, and a rolling 14-day window.
- **A real 36 MB Cameo vendor model migrated end to end**: 362 elements, 633 relationships, 48,553 named losses accepted, commit `b715131f…`. The analytics then reported 134 orphaned elements and 153 disconnected groups in it.

## 6. What is NOT built (do not claim it)

- **No numeric or physical simulation.** No FEA, thermal or fluid solvers. Model-level simulation is **designed, not built** (spec tranche A4).
- **No CAD or geometry authoring.** It integrates with CAD; it is not a drawing tool.
- **The SysML v2 and Capella readers are VIEWERS** — they read into OKF and cannot export. An export attempt is refused naming the direction.
- **The native XMI-to-OKF direction is not independently measured** — the gate measures the binding's own round trip; the read rests on the binding's loss report.
- **No bundled open-source example is "loadable" except the XMI corpus.** The TMT needs 48,553 losses accepted by hand; the Gaphor files are a different format with no binding; SysML v2 models are viewers.
- **Per-node component membership** is not exposed (the counts are; the raw tables let a consumer compute it).
- **No plugin system, no LLM-authored analysis definition, no simulator** (later slices).

## 7. Deployment (idc-1)

Two **separate** instances of the same binary, both on `127.0.0.1`, fronted by a **cloudflared** tunnel (haproxy on that host serves the model fleet, not these):

| | Showcase | Registered trial |
|---|---|---|
| URL | trial.modelwrite.org | app.modelwrite.org |
| port | 3102 | 3103 |
| unit | `modelwrite.service` | `modelwrite-app.service` |
| env | `/opt/modelwrite/deploy/modelwrite.env` | `/opt/modelwrite/deploy/modelwrite-app.env` (mode 600) |
| data | shared DB, seed-replayed | `/opt/modelwrite/trials/` — one SQLite DB per trial |

- Build path: **build from the public repo on the host** (`docker build`, `--cpuset-cpus=0-3` because the host runs a live vLLM fleet). The ghcr image was private and unpullable; that is why the build happens on the box.
- **Timers:** `modelwrite-trial-reset.timer` (hourly, with a change-detection guard that SKIPS when nothing changed) and `modelwrite-trial-verify.timer` (weekly `FORCE=1` canary — it exists because the skip-guard once hid a broken reset).
- **Backups:** before every reset, to `/opt/modelwrite/backups/`, timestamped, last 48 kept.
- **The seed manifest is reproducible from the repo**: `docs/deploy/seed-trial.mjs`. The host copy is `/opt/modelwrite/seed/`.
- Full deployment notes: `docs/deploy/idc-1-trial.md` and `docs/deploy/app-trial.md`.

## 8. Secrets — NOT in git, and where they are

**Nothing secret is in the repository. Verify that before pushing anything.**

- `.env` on the authoring machine (gitignored, never committed): `CARGO_REGISTRY_TOKEN`, `GITHUB_TOKEN`.
- **Postmark** server + account tokens: `hatch-core/.env` on the authoring machine; the SMTP credentials are in `/opt/modelwrite/deploy/modelwrite-app.env` on idc-1 (`MW_MAILER=smtp`, `MW_MAIL_SMTP_HOST=smtp.postmarkapp.com`, port 587, username/password = the server token, `MW_MAIL_FROM=alex@axoquant.com`).
- **Cloudflare** write token (DNS + tunnel): `insights-oaic-poc/.env`. The `algolotl-core` token is read-only.
- **crates.io / npm / ghcr** publishing credentials are on the authoring machine.

⚠️ **ACTION REQUIRED — ROTATE THE POSTMARK SERVER TOKEN.** It was printed verbatim into a session transcript during development (a failed redaction). Rotate it, then update both the workstation `.env` and `MW_MAIL_SMTP_USERNAME`/`MW_MAIL_SMTP_PASSWORD` on idc-1, and restart `modelwrite-app`.

## 9. Known open items

1. **Rotate the Postmark token** (above). Until then, treat it as exposed.
2. **The apex TLS certificate never issued.** `https://modelwrite.org` fails with `SEC_E_WRONG_PRINCIPAL`; `http://` works. Fix by putting Cloudflare in front (proxied, SSL Full) or by re-saving the custom domain in GitHub Pages settings.
3. **`mw-binding-sysmlv2` and `mw-binding-capella` are not on crates.io.** The other 12 crates are at 0.2.0. The viewers exist in the repo and cannot be `cargo add`-ed.
4. **A scoped diagram's routes are not covered by a test** — only the corpus and cafe-stand are. A scope whose geometry puts a box flush against a source side could still cross.
5. **Edge-label placement has one residual**: 1 label in 583 has a jump-arc graze of 0.39 units (0.3 px on a slide), measured in the browser only.
6. **The analyses frame has no API/JSON routes yet** — the pages are the surface; agent access is a later slice.
7. **STPA's method-coverage stages** cover losses, hazards, system-constraints, control-structure and UCAs. Deeper STPA (refined constraints, multiple control structures) is not modelled.

## 10. How to resume — the first five commands

```bash
git clone https://github.com/modelwrite/modelwrite.git && cd modelwrite
cargo test --workspace --no-fail-fast        # expect 748 passing, 0 failed
cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings
node website/generate-docs.mjs && git diff --exit-code   # CI fails on doc drift
```

To run the workbench locally: `cargo run -p mw-server` with `MW_DB` pointing at a SQLite file, then open `/ui`. Seed a model from `e2e/models/*.json`.

**Windows notes:** Rust is not on PATH in a fresh shell — prefix with `$env:Path = "$env:USERPROFILE\.cargo\bin;" + $env:Path`. PowerShell's `-Encoding utf8` writes a **BOM** and breaks JSON bodies; use ASCII or the file tools. Use `CARGO_TARGET_DIR=target-tests` if a running server locks `target/debug`.

## 11. The process that produced this — read before dispatching work

This project's quality came from four habits. Dropping them will drop the quality.

1. **Verify, don't remember.** Every number in the docs is checked against the repository or a running server, or it is labelled unchecked. Stale claims kept appearing *because documentation is written once and the system keeps moving* — so the volatile facts (crate licences, the binding list, MCP tools, metric definitions) are now **generated** into the docs, and **CI fails if a generated section drifts** (`website/generate-docs.mjs`).
2. **Look at the thing.** The subagents cannot see images; the visual review is the controller's job. Every defect that mattered was found by looking or measuring: arrowheads painted black by a CSS class mismatch, 12 of 12 labels drawn over a box, a favicon that dissolved at 16 px, ragged card rows, an unreadable fit-to-view. **The browser measurement harnesses are committed** (`e2e/label-overlap.mjs`, `scope-proof.mjs`, `render-scope-proof.mjs`, `contrast.js`, `netcheck.js`, `probe-arrows.mjs`) — use them rather than eyeballing.
3. **Serialise work that shares files.** Parallel agents on the same files cost real work three times (twice by the controller's own `git add`). Give each task explicit file ownership, and **never `git add -A` in a shared tree** — stage by path.
4. **Make the claim checkable in code.** The rules in §4 are not conventions; they are tests. If you add a capability, add the test that fails when it regresses — and show it failing first.

**A recurring defect class, worth watching for:** *the thing is built and nobody can reach it.* The CLI was unpublishable, the SysML v2 reader was unregistered, `MW_MAILER=smtp` was a stub that silently fell back to the console, and the site described a product from hours earlier. When you ship, check that a stranger can actually get to it.

## 12. The decision record — read these before changing direction

`docs/superpowers/specs/` — 16 specs, each answering a question with evidence:

platform design · standards interoperability (the binding discipline) · analytics design · **system of systems** (reference-not-copy; measured vs asserted) · **domain symbology** (MIL-STD-2525/APP-6; the three licence classes) · **visual vocabulary** (notation before decoration; no trademarked marks) · **visual outputs** (export from the commit) · **renderer decision** (own the model-to-picture mapping; export to Mermaid/D2 rather than adopt) · **MCP surface review** (what an agent can see) · **OKF as configuration** (an install is a document) · **analyses and simulations** (an analysis is a record) · **low-friction entry** · **open vs licensed boundary** (*free to prove your model; paid to govern the data*) · **registered trials** · **STPA and safety** (the analysis is authored, the check is computed).

`docs/superpowers/plans/` — 13 plans: the 7 slices, the capability roadmap, navigation and versions.

## 13. For the next agent

- **Do not soften the Limits page.** It is the reason the rest of the site is believable. If a limit stops being true, remove it because you changed the product — never because it reads badly.
- **Do not let an AI number into the prose.** The digit-scan test exists for a reason.
- **If you fix a vacuous pass, prove it red-green.** The STPA fix did: revert the coverage layer and the four tests fail with `left: Performed, right: NotStarted`.
- **Back up before you destroy.** The hourly reset backs up every time — an overnight reset that destroys a visitor's work with no copy is not a reset, it is data loss.
- **Tell the truth about what is not built.** The whole position is that this platform publishes its own gaps accurately enough that they can be found.
