# Modelwrite — Design & Comprehensive Plan

Date: 2026-09-17 · Author: Alex Kovaceski · Status: draft for review
Pattern source: bylazora (the-migration-you-can-prove) · Collateral: MEMKO MBSE Coffee-Machine Knowledge Pack

> **Superseded by v2:** docs/superpowers/specs/2026-09-17-modelwrite-platform-design.md. This v1 framed the deliverable as a portal built over one exported model. The v2 design reframes modelwrite as the organisation-grade platform that replaces CATIA Magic, and keeps the coffee-machine corpus as the proof fixture. The OKF contract, the provable gate, the evidence annex and the grounded-AI decisions in this document still stand.

## 1. Purpose

Modelwrite is an open-source Model-Based Systems Engineering (MBSE) platform that lets
organisations move off legacy modelling tools — CATIA Magic / Cameo first — by making
**the model, not the tool, the asset**. It productises the five patterns proven in the
coffee-machine knowledge pack:

1. **Macro-as-API** — drive the authoring tool through its OpenAPI, never the GUI.
2. **Model as single source of truth** — every downstream view is generated, never hand-copied.
3. **Knowledge file (OKF)** — one self-describing, versioned export is the contract between
   the modelling tool and every web/AI/analytics layer.
4. **Grounded local AI** — LLMs and ML models that answer only from the model, offline-capable.
5. **Open-data fusion** — join model requirements with real external data for trade-off decisions.

The context that makes this urgent: organisations cannot staff enough people who can drive
legacy MBSE tools. Modelwrite uses AI to automate the skilled modelling work — authoring,
repair, review, analysis — so scarce systems engineers supervise instead of clicking.

## 2. Context

### 2.1 What the collateral proves

The coffee-machine knowledge pack demonstrates the complete pipeline modelwrite must own:

- A CATIA Magic SysML v1 model built by **Groovy OpenAPI macros** (sessioned, file-verified).
- An **OKF export** — a single JSON of elements + a relationship graph (99 nodes / 165 edges).
- **Graph review** found 36 isolated nodes and 5 disconnected components that diagram review
  missed; fixes collapsed the model to 1 connected component.
- A **digital twin** runs the state machine + boiler physics, evaluates REQ 1.1 in real time,
  and injects faults (including the RCD/electrocution hazard for REQ 3.1.5).
- **OEE/Pareto**, **requirements-vs-market sensitivity**, an **ontology explainer**, and a
  **local LLM** (Ask the Model, Ollama, fully offline) all read the same OKF file.
- The field guide documents the legacy-tool traps an interoperability layer must handle:
  associations owning their two ends, state entry/do behaviours re-parenting activities,
  profile packages polluting exports, save semantics, GUI automation brittleness.

### 2.2 The bylazora pattern to mirror

| Bylazora element | Modelwrite equivalent |
|---|---|
| Byte-exact migration gate | **Round-trip fidelity gate** (legacy model → OKF → re-export → zero-loss semantic equality) |
| Rust engine + C ABI + MCP server | Rust engine (okf/graph/gate/twin) + C ABI + MCP server |
| Apache-2.0 parsers mirror | Apache-2.0 importers/exporters (adapter macros, ReqIF, Excel, XMI) |
| judge/ gate-as-judge harness | judge/ — scores agent runs, standalone or in agent loops |
| agents/ rule packs | agents/ — metamodel invariants AI agents must respect |
| sample/ + CI regression | sample/ — the coffee-machine corpus, gate as CI test |
| docs/ evidence annex (claims trace to recorded runs) | docs/ — OKF spec, white papers, evidence annex |
| Private business repo (portal, dashboard, keygen, legal) | modelwrite-business (private) — same boundary rules |
| Static proof site | modelwrite-site → modelwrite.org |

Identity: **The model you can prove.** — every claim about import/export fidelity, graph
health, and AI-generated model changes traces to a recorded, signed gate run.

## 3. Approaches considered

### Approach A — Portal + automation core first (recommended)

Productise what already works: OKF spec, Rust engine (validation/graph/gate/twin), React
portal, AI copilot + ML analytics, import/export connectors. The full browser modelling
editor is a later milestone. Fastest path to a real open-source project with evidence;
matches the collateral 1:1; every phase is independently shippable.
*Trade-off:* does not replace CATIA's authoring surface on day one.

### Approach B — Full web modelling editor first

Build a browser SysML authoring environment (diagrams, matrices, simulation) before
portals/AI/connectors. Truest CATIA replacement but the largest scope; AI automation and
seamless import/export — the user's stated priorities — would land much later, and the
collateral would be under-used.
*Trade-off:* high risk of stalling before any provable value ships.

### Approach C — Python-first analytics platform

Everything in Python (parsers, graph, portal via Streamlit/Gradio, ML). Fastest prototyping,
weakest foundation for the bylazora-style story (single distributable binary, C ABI, MCP
server, performance evidence, industrial deployment).
*Trade-off:* fights the pattern the user asked to follow.

**Recommendation: Approach A.**

## 4. Chosen design

### 4.1 Architecture (layers, OKF as the contract)

Legacy tools (CATIA Magic, Rhapsody, EA, DOORS, Excel)
  → importers (Groovy OpenAPI kit, ReqIF, XMI, CSV)  [Apache-2.0]
  → OKF 1.0 — Open Knowledge Format: versioned JSON Schema — elements + graph + provenance
  → Rust engine (AGPL): okf · graph · gate · twin · mcp — C ABI — MCP server
      ├─ Web portal (TS/React): explorer · twin · OEE · sensitivity · ontology · ask-the-model · diff · evidence
      ├─ AI layer (Python): copilot agents + rule packs · ML quality/pattern models · local-first, optional cloud
      └─ Exporters (Apache-2.0): OKF → ReqIF/XMI/Excel/Magic-macros/SysML v2

**OKF 1.0** (formalised from the collateral export): versioned JSON Schema; element kinds
(block, requirement, state, activity, signal, interface, use case, actor, constraint block);
relationship kinds (satisfy, refine, verify, allocate, part, reference, association,
generalization, dependency, transition, behavior, effect, trigger, uses, contains, include,
subject); graph section; provenance block (source tool, exporter version, timestamps,
checksums); parametrics and values. One file is the single contract consumed by every layer.

**Rust engine crates**
- okf — schema, validation, hashing, semantic diff between two OKF snapshots.
- graph — connected components, orphans, traceability coverage, requirement-coverage
  matrices, centrality (the coffee-machine checks as library functions).
- gate — the provable gate: import → OKF → export → OKF' → semantic equality assertions
  (0 lost elements, 0 lost relationships, attribute equality); every run records signed
  evidence JSON; graph-integration gate (1 component / 0 orphans); requirement-coverage gate;
  twin replay gate (state-machine traces reproduce).
- twin — generic state-machine interpreter + parametric solver (the digital twin, portable
  to any OKF model, with fault injection).
- mcp — MCP server exposing the engine to AI coding agents; C ABI (modelwrite.h) for
  embedding, bylazora-style.

**Web portal (TypeScript/React)** — the productised coffee-machine portal: graph explorer,
structure/requirements/STM/activities views, digital twin, OEE/Pareto, market-fit
sensitivity, ontology explainer, Ask the Model chat — plus model diff view, gate/evidence
view, copilot panel, and import/export wizards. Serves any OKF file; runs fully local.

**AI layer (Python)** — the automation that addresses the skills shortage:
- *Copilot agents:* natural language → model edits as sessioned operations through a
  **model-edit API** (never raw tool macros); rule packs in agents/ encode the metamodel
  invariants (association ends, state re-parenting, profile exclusion, session wrapping);
  RAG grounded in OKF; local-first (Ollama-class) with optional cloud; **the gate scores
  every agent run** — no tool parameter can mark a run proven (bylazora invariant).
- *ML models:* graph-embedding based gap/smell detection and model-quality scoring (trained
  on labelled corpora, starting with the coffee-machine before/after states); pattern
  mining and similarity search across a model corpus; requirement clustering; fault
  prediction from twin runs. All consume OKF; models served through a small API.

**Interoperability layer (Phase 4 — seamless)**
- CATIA Magic / Cameo: productised Groovy import/export macro kit (from the collateral),
  Teamwork Cloud REST adapter, watch-folder sync (mdzip → export → OKF), write-back via
  OpenAPI sessions; the round-trip gate proves nothing is lost.
- ReqIF / DOORS-class, Excel/CSV requirements exchange, XMI (Rhapsody, Enterprise Architect).
- SysML v2 JSON as the forward-looking interchange backbone.
- Seamless = bidirectional connectors + the fidelity gate as the CI regression on every
  import/export cycle + evidence, not vibes.

### 4.2 Repository structure (public modelwrite repo)

engine/       Rust workspace: okf, graph, gate, twin, mcp + C ABI header   (AGPL-3.0-or-later)
importers/    magic-groovy-kit, reqif, xmi, excel                          (Apache-2.0)
exporters/    okf→reqif/xmi/excel/magic, sysml-v2                          (Apache-2.0)
judge/        gate-as-judge harness for agent loops (standalone)
agents/       rule packs for AI coding agents (CLAUDE.md + invariants)
ml/           Python: copilot API, edit API, ML models, training, serving
portal/       TypeScript/React web app
sample/       coffee-machine corpus: .mdzip, macros, expected OKF, evidence
docs/         OKF spec, white papers, evidence annex, this design
.github/      CI: gate runs on every change

Private modelwrite-business and public modelwrite-site follow bylazora's split and
boundary rules exactly (seeds never in git, public repos ship public keys only, pricing and
target lists never public).

### 4.3 Phased plan

**Phase 0 — Foundations & corpus (weeks 1–2)**
Scaffold the public repo (licence, CODE_OF_CONDUCT, CONTRIBUTING, SECURITY, NOTICE); write
the OKF 1.0 spec + JSON Schema; port the collateral into sample/ (model, macros, exported
JSON, app); CI skeleton; modelwrite.org landing page in modelwrite-site.

**Phase 1 — Engine core (weeks 3–6)**
okf + graph + gate crates; first gate run against the corpus recorded in docs/evidence;
C ABI; MCP server; judge/ harness. TDD throughout; the corpus is the regression fixture.

**Phase 2 — Portal productisation (weeks 7–10)**
React portal over OKF: all collateral views plus diff and evidence views; import wizard;
single-file standalone export (the showcase pattern) for zero-install demos.

**Phase 3 — AI automation (weeks 11–16)**
Model-edit API + copilot agents + agents/ rule packs; ML quality/gap models trained on the
labelled corpus; MCP integration so coding agents can do MBSE tasks; judged agent loop
(gate scores runs); local-first, optional cloud.

**Phase 4 — Seamless interoperability (weeks 17–22)**
CATIA Magic import/export kit + Teamwork Cloud adapter + watch-folder sync; ReqIF, Excel,
XMI connectors; SysML v2 export path; round-trip gate as CI on every connector; evidence
annex documenting measured fidelity for each legacy target.

**Phase 5 — Business hardening (ongoing, private)**
Licence portal, managed-run dashboard, keygen, legal (patent/trademark/entity) in
modelwrite-business; modelwrite.org evidence pages, white papers, and the interactive
standalone demo embedded on the site.

Each phase is test-driven, ends with the gate passing on the corpus, and updates the
evidence annex. Public claims trace to recorded runs — never percentages without proof.

## 5. Key decisions (assumed, pending confirmation)

1. **V1 scope:** portal + automation core first; full browser editor later (Approach A).
2. **Legacy targets:** CATIA Magic first; ReqIF/Excel next; XMI (Rhapsody/EA); SysML v2 path.
3. **Provable gate:** round-trip fidelity (layered with graph-integration health).
4. **AI scope:** copilot agents + ML analytics, local-first with optional cloud.
5. **Stack:** Rust core + TypeScript/React portal + Python ML.
6. **Split:** AGPL core + Apache importers/exporters + private business repo + static site.
7. **Naming:** GitHub org modelwrite → repos modelwrite / modelwrite-site /
   modelwrite-business (private); modelwrite.org as the public site.

## 6. Risks & mitigations

- *mdzip native parsing is hard* (MagicDraw's container is proprietary-ish). Mitigation:
  ship the proven Groovy OpenAPI kit + Teamwork Cloud REST first; treat a native parser as
  an optional later enhancement.
- *LLM edits corrupting models.* Mitigation: model-edit API + rule packs + the gate as
  mandatory scoring; corrupt runs are rejected, not shipped.
- *Scope creep toward a full editor.* Mitigation: phase gates; the editor is an explicit,
  separately-approved milestone.
- *Open-source sustainability.* Mitigation: bylazora's open-core/private-business split and
  evidence-led credibility.

## 7. Open questions

- Confirm the seven decisions in §5 (the interactive question batch timed out unanswered).
- Exact GitHub org/repo names and who owns them (a GitHub search shows a pre-existing
  ModelWriter org with unrelated EU-research projects — is that org ours?).
- Priority order for Phase 4 legacy targets beyond CATIA Magic.
- Whether importing and exposing data seamlessly includes live bidirectional sync with
  Teamwork Cloud, or file-based round-trips suffice for v1 of that phase.
