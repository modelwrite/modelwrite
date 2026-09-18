# Modelwrite — Platform Design (v2): the open-source MBSE platform an organisation runs on

Date: 2026-09-17 Â· Author: Alex Kovaceski Â· Status: draft for review
Supersedes: docs/superpowers/specs/2026-09-17-modelwrite-design.md (v1)
Pattern source: bylazora Â· Collateral: MEMKO MBSE coffee-machine knowledge pack

## 1. What modelwrite is

Modelwrite is an organisation-grade, open-source Model-Based Systems Engineering
platform. An organisation installs it and does its systems engineering in it:
authoring models, collaborating in a shared repository, running analysis and
simulation, governing change, and exchanging data with the tools it already owns.

CATIA Magic / Cameo is a migration source and a coexistence partner, not the
reference point. The coffee-machine corpus is the proof fixture that demonstrates
fidelity, not the product.

Identity: **the model you can prove** — every claim about import fidelity, model
health, and AI-generated change traces to a recorded, signed gate run.

### 1.1 Non-goals (explicit)

- CAD and geometry authoring (the 3D side of the CATIA portfolio). Modelwrite
  integrates with CAD rather than replacing it.
- Certified, complete SysML v2 conformance on day one; the platform targets a
  documented, growing subset plus an honest conformance matrix.
- Real-time multi-cursor co-editing in the first release; element-level locking
  plus branch and merge lands first.
- Becoming a general-purpose diagramming or drawing tool.

## 2. What changes from the v1 design

The v1 design proved the right foundation — an open canonical format (OKF), a
provable gate, graph-based model review, grounded AI — but framed the deliverable
as a portal over one exported model. That is a demonstration, not a platform an
organisation can move onto.

v2 keeps every v1 decision about the contract and the gate, and adds the
capabilities an organisation needs before it can leave a legacy vendor:

1. **Model repository** — shared, versioned, permissioned, audited. The answer to
   Teamwork Cloud.
2. **Authoring workbench** — graphical and textual modelling in the browser, with
   matrices, tables, reports and live validation. The answer to the MagicDraw and
   Cameo authoring surface.
3. **Model patterns and libraries** — a catalogue of reusable architectures and
   reference models that can be instantiated, checked and evolved.
4. **AI automation, including a shipped agent** — the platform is not only a tool
   surface for other AI tools: it ships its own MCP agent (`agent/`) that intakes
   requirements, generates models by instantiating patterns, repairs what the gate
   rejects, and reviews at scale. The answer to the skills shortage: the platform
   does the skilled repetition so scarce engineers supervise and accept.
5. **Interoperability and migration** — bidirectional connectors for CATIA Magic,
   Rhapsody, Enterprise Architect, ReqIF, Excel and the SysML v2 API, with the
   round-trip gate proving zero loss in both directions.
6. **Governance and assurance** — baselines, approvals, audit trails, CI model
   gates and an evidence annex, so regulated organisations can trust the record.
7. **Deployment and operations** — self-hosted, air-gapped, containerised, with
   enterprise identity. Software an organisation cannot install is not a
   replacement.
8. **Analytics and data fusion** — bring the outside world into the platform:
   product catalogues, supplier prices, ERP and PLM exports, test and telemetry
   data, cost models and standards registers, bound to the model, and answerable
   in plain language. The answer to the question every programme asks: does
   anything on the market meet this specification, and which requirements cost us
   the most.

## 3. The CATIA replacement backlog (from the field guide)

The CATIA field guide in the collateral lists capabilities held in reserve. That
list is the parity backlog for replacing the tool, mapped to delivery slices:

| CATIA capability (field guide) | Modelwrite equivalent | Slice |
|---|---|---|
| Teamwork Cloud — multi-user modelling, versioning | Model repository: commits, branches, baselines, locks, audit | S2 |
| Groovy OpenAPI automation — scripted model changes | Model-edit API plus the migration macro kit | S2, S4 |
| Active validation and validation suites | Rules engine: live validation in the workbench, CI gates | S1, S3 |
| Matrices and tables (dependency matrix, generic, instance) | Traceability matrix and table views over OKF | S3 |
| Relation Map and model browser | Graph explorer with neighbourhood walk and impact analysis | S3 |
| Cameo Simulation Toolkit — execute the model, solve parametrics, time series | Twin engine: state-machine execution, parametric solver, scenario traces | S3, S6 |
| Report Wizard — Word and HTML reports from the model | Reporting service with templated model reports | S7 |
| Requirements import/export — Excel, ReqIF, DOORS via DataHub | Interchange connectors: ReqIF, Excel, CSV; DOORS later | S4 |
| Variants and trade studies | Variant configuration and trade-study scoring | S5, S6 |
| **Beyond CATIA parity:** nothing in the legacy toolkit fuses the model with the outside world | Analytics and data fusion: semantic bindings, compliance matrix, feasibility gap, cost drivers, sensitivity, plain-language questions | S5 |

That table is the definition of done for the replacement claim: an organisation
moves when the right-hand column covers the work it does today.

## 4. Architecture

```
Clients
  web workbench (authoring, explorer, matrices, dashboards, admin)
  agent runtime (shipped MCP agent: skills, rule packs, replay logs)
  CLI and CI runner        MCP clients and AI agents        REST integrations
        |                        |                              |
Platform services (Rust)
  identity and access (OIDC/LDAP/SSO, roles, project scoping)
  repository service (OKF store, commits, branches, baselines, locks, audit)
  analysis service (graph, coverage, rules, gate, evidence)
  simulation service (state machines, parametrics, scenarios)
  analytics service (data fabric, semantic bindings, query engine, provenance)
  AI service (copilot, model-edit API, ML quality and pattern models)
  reporting service (model reports, traceability packs, evidence bundles)
        |
Engine crates (AGPL-3.0-or-later, shared by services and CLI)
  okf Â· graph Â· gate Â· twin Â· rules Â· patterns Â· sysml2 Â· mcp Â· capi
        |
Interchange layer (Apache-2.0)
  magic-groovy-kit Â· teamwork-cloud adapter Â· reqif Â· xmi Â· excel Â· sysml-v2-api
        |
Storage and deployment
  PostgreSQL (metadata, versions, audit, permissions) with an artefact store;
  SQLite for single-user and edge installs; Docker Compose, Helm, air-gapped bundle
```

### 4.1 OKF stays canonical; SysML v2 becomes the authoring language

OKF remains the vendor-neutral canonical form and the contract between every
layer. SysML v2 (KerML and the SysML v2 API and JSON) is the modern authoring and
interchange standard the platform targets; SysML v1 models arrive through OKF
mapping for migration. The mapping between OKF and SysML v2 is a first-class,
tested module (engine/sysml2), not an afterthought.

### 4.2 Concurrency model

Element-level locking inside a branch, with semantic diff and merge at the OKF
level (the engine diff already computes element, edge and attribute changes).
Long transactions are avoided; every save is a commit with an audit entry.
Real-time co-editing is a later optimisation, not a prerequisite.

### 4.3 Assurance model

Every state of the repository can be gated: traceability coverage, orphan and
component checks, pattern conformance, requirement verification status, and
round-trip fidelity against an imported legacy baseline. Each gate run writes a
deterministic evidence record; approvals reference the evidence, not assertions.

### 4.4 The modelwrite agent — AI-generated MBSE, end to end

Modelwrite ships an agent, not merely a tool surface. An organisation gets
AI-generated MBSE out of the box: the platform can take requirements and intent in
and produce a model, repair a model, or review a model, with the gate as the
objective function and a human in the loop for acceptance.

**Shape.** The agent is an MCP client of the modelwrite MCP server, packaged in the
repository as agent/ (runtime plus skill packs), with agents/ holding the rule packs
and ml/ the model providers and retrieval. Providers are pluggable: a local model by
default (air-gapped friendly), a customer-provided endpoint, or an approved cloud
endpoint. Nothing in the loop requires the cloud.

**What it does.**

1. **Intake** — requirements from Excel, ReqIF, Word, PDF and standards clauses, plus
   existing legacy models, become structured intent rather than prose.
2. **Synthesis** — the model is generated by instantiating patterns from the pattern
   library, parameterised and consistently named, rather than free-form invention.
   Patterns are the generation substrate: that is what makes a generated model
   structurally sound before anyone reviews it.
3. **Repair** — gate and rule findings (orphans, uncovered requirements, missing
   traceability, broken interfaces, pattern non-conformance) become a work queue, and
   the agent iterates until the gate passes or reports why it cannot.
4. **Review and explanation** — review packs, change impact, and answers about the
   model cited to element ids.
5. **Migration assistance** — driving the legacy kit, resolving mapping gaps, and
   proposing resolutions when fidelity fails.
6. **Analytics and document skills** — with the analytics workstream, questions across
   bound structured and unstructured data.

**The discipline that makes it trustworthy.** Every agent action is an instruction on
the model-edit API, never a direct edit. Every loop iteration ends in a gate run. The
agent can never mark its own work correct. Output is a draft commit for human review.
Runs are budgeted (steps, tokens, wall clock) and replayable from a recorded log of
prompts, tool calls and results, and a completed run writes evidence beside the gate
records.

**The tool surface is the contract.** The MCP server exposes the stable tool set the
agent binds to — validation, diff, graph statistics, rules, gate, model read, model
propose, patterns list and instantiate, evidence write — versioned and published as a
machine-readable manifest so an organisation can add its own skills without forking
the agent.

**What fully AI-generated MBSE means here** (measured in Slice 5, not asserted):

- From a requirements workbook and a stated pattern set, the agent produces a model
  with zero human edits that passes validation, the rules (traceability coverage at
  target), graph integration (no orphans, one component) and the round-trip gate.
- Given a deliberately broken model, the agent repairs it to the same standard.
- Ambiguity is surfaced as questions, never invented away.
- The run replays deterministically from its log, and its evidence is committed.
### 4.5 Analytics and data fusion

Organisations do not decide from a model alone. They decide from the model plus
the world it has to exist in: what the market offers, what suppliers charge, what
tests measured, what operations reported, what a standard demands. Modelwrite
treats that as a first-class capability rather than a spreadsheet bolted on the
side.

**This is the long-horizon workstream (Slice 7), and it is deliberately shaped
differently from the others.** It is not a bounded feature but a continuing
programme of ingesting additional data sources, structured and unstructured, so it
gets its own specification, its own plan and its own evaluation before
implementation. It sits last in delivery order precisely so it can have that room.
Section 4.4 describes the shape of that programme, not a fixed scope.

**1. Data fabric.** Connectors bring external datasets in and keep them fresh:
CSV, Excel, JSON and REST for product catalogues and price lists; Parquet for
bulk analytical data; SQL sources (PostgreSQL, SQL Server) and ERP or PLM exports
(SAP, Teamcenter) for enterprise data; time series for test and telemetry data;
document and register extracts for standards and regulatory material. Every
dataset is registered with an owner, a licence, a classification, a refresh
policy and a content hash. Imports are versioned and reproducible, and an
air-gapped organisation imports the same datasets from a signed bundle.

**2. Semantic bindings.** A binding links a model element to a dataset concept: a
requirement threshold to a column, a block or part to a supplier entity, a value
property to a measured field, a measure of effectiveness to a derived column.
Bindings live in the model as versioned content (an OKF 1.1 bindings section), so
they are reviewed, diffed and gated like everything else, and the validator
enforces referential integrity: a binding to a deleted requirement is an error,
not a silent orphan. Because OKF readers ignore unknown fields, OKF 1.0 tooling
stays compatible with 1.1 documents.

**3. Analysis engine.** DuckDB runs SQL directly over the registered datasets and
the model export, so no separate warehouse server is needed and air-gapped
installs stay simple. Standard analysis packs built on it:

- **Compliance matrix** — every requirement with a quantitative threshold scored
  against every candidate product, part or supplier.
- **Feasibility gap** — the requirements no candidate satisfies, with the margin
  by which the best candidate falls short: the distance from the specification to
  the market.
- **Cost drivers** — per requirement, the number of candidates it excludes, the
  price difference between the cheapest conforming and non-conforming options,
  and the cost of relaxing it. This is the direct answer to which requirements
  are the most expensive.
- **Sensitivity and what-if** — relax or tighten a threshold and recompute the
  candidate set and cost curve live, ranking requirements by marginal impact.
- **Make versus buy and trade studies** — score external candidates against
  in-house options taken from the model, using measures of effectiveness defined
  once in the model.
- **Internal analytics** — coverage, churn, change impact, effort and verification
  status computed over the model itself.

**4. Unstructured data.** Most of an organisation knowledge is not tabular:
standards and regulations, specifications, test reports, supplier documents,
drawings, maintenance histories, correspondence. This half of the workstream adds
pipelines for PDF, Office, HTML, images and scans with text, table and OCR
extraction; entity and relation extraction; embedding and vector search; and
proposed links from extracted concepts back to model elements, reviewed by a human
before they become content. Every fragment keeps its source, page and offsets so it
can be cited, and retrieval quality is measured against a labelled question set
rather than asserted.

**5. Questions with provenance.** A question in natural language is planned into a
semantic query over bindings and datasets, executed, and answered with citations:
the model commit, the dataset versions, the bindings used and the exact query. An
answer that cannot cite its evidence is refused rather than guessed, and every
answer can be re-run deterministically and stored as an evidence record —
analytics you can prove, in the same way the gate proves fidelity.

Dashboards and reports consume the same queries: compliance and sensitivity views,
a cost-driver Pareto, supplier gap analysis and generated report packs for design
reviews and procurement.
## 5. Repository layout for the platform

```
engine/      Rust crates: okf, graph, gate, twin, rules, patterns, sysml2, mcp, capi
server/      platform services: api, repository, auth, audit, simulation, reporting
apps/web/    the workbench: authoring, explorer, matrices, dashboards, admin
agent/       the shipped MCP agent: runtime, skill packs, replay logs
ml/          Python: copilot service, model-edit API, ML quality and pattern models
data/        analytics: connectors, dataset registry, bindings, query packs  (Apache-2.0)
importers/   magic-groovy-kit, teamwork-cloud, reqif, xmi, excel   (Apache-2.0)
exporters/   okf to reqif, xmi, excel, magic, sysml v2             (Apache-2.0)
judge/       gate-as-judge harness for migrations and agent loops
agents/      rule packs for AI coding agents
sample/      the coffee-machine corpus as the proof fixture
deploy/      docker compose, helm chart, air-gapped bundle scripts
docs/        OKF spec, design, plans, evidence annex, conformance matrix
```

Public core repository stays AGPL-3.0-or-later with Apache-2.0 interchange code;
modelwrite-business stays private (support portal, managed-run dashboard, signing
keys, legal); modelwrite-site publishes modelwrite.org with the evidence-backed
claims.

## 6. Delivery slices

Each slice ships working, tested software on its own, and is proven by the gate
against the corpus plus new fixtures.

| Slice | Capability | The organisation can now |
|---|---|---|
| S0 | Foundations: repo, OKF 1.0 spec and schema, corpus as proof fixture, CI | Read the standard; reproduce the proof |
| S1 | Engine core: okf, graph, gate, rules seed, C ABI, MCP server, judge | Prove fidelity and model health in CI |
| S2 | Repository and collaboration: server, commits, branches, baselines, locks, RBAC, audit, deployment skeleton | Run a team on a shared, governed model store |
| S3 | Authoring workbench: structure, requirements, state machine, activity, parametric views, matrices, tables, live validation, patterns library, SysML v2 textual editing | Build and review models without a legacy licence |
| S4 | Interoperability and migration: CATIA Magic and Teamwork Cloud, ReqIF, XMI, Excel, SysML v2 API, both directions | Move off CATIA incrementally, with zero-loss proof |
| S5 | AI layer: model-edit API, copilot agents and rule packs, ML quality and pattern models, simulation service, model question answering | Model faster than the available headcount allows |
| S6 | Governance and assurance: CI model gates, approvals and baselines, reporting, air-gapped deployment, identity integration, conformance matrix | Certify and operate the platform under audit |
| S7 | Analytics and data fusion (long-horizon workstream): dataset registry, semantic bindings, DuckDB analysis engine, structured decision analyses, plus unstructured data — documents, standards, reports, scans, drawings — with extraction, linking and grounded question answering | Ask whether anything on the market meets the specification, which requirements cost the most, and what the documents actually say |

Slice 1 already has a bite-sized implementation plan:
docs/superpowers/plans/2026-09-17-modelwrite-p0-p1-engine-core.md. Each later
slice gets its own plan, written with the same discipline when the slice starts,
so the plans stay accurate instead of speculative.

## 7. Decisions and assumptions to confirm

1. **Standards posture:** SysML v2 as the primary authoring and interchange
   standard, with SysML v1 and CATIA arriving through OKF mapping (recommended).
   The alternative is SysML v1 first for maximum CATIA similarity.
2. **Repository storage:** PostgreSQL plus a Rust service for team installs,
   SQLite for single-user (recommended); git-backed model-as-code as an export
   and integration option rather than the primary store.
3. **Authoring surface:** browser-first workbench (recommended), with an optional
   desktop shell later for offline field use.
4. **Concurrency:** locking plus branch and merge first, real-time co-editing
   later (recommended).
5. **Deployment:** Docker Compose and Helm, air-gapped bundle, AGPL with no
   licence server (recommended).
6. **Proof fixture:** the coffee-machine corpus gates every slice.
7. **Repo naming:** GitHub org modelwrite, repos modelwrite, modelwrite-site,
   modelwrite-business (private).
8. **Analytics engine:** embedded DuckDB over Parquet and registered datasets, with
   PostgreSQL optional for shared catalogues and no separate warehouse server
   (recommended). The alternative, an external warehouse or BI tool, complicates
   air-gapped installs and provenance.
9. **Bindings live in the model:** an OKF 1.1 bindings section, versioned, diffed
   and gated alongside the model (recommended), rather than an external mapping
   file that can drift out of step.
10. **First external data sources:** product catalogues, price lists and supplier
    data (the collateral case), then ERP and PLM exports and test and telemetry
    data. Items 8, 9 and 10 confirmed on 2026-09-17.
11. **The agent ships in the library** as an MCP client with pluggable providers,
    local-first by default, with the pattern library as its generation substrate and the
    gate as its objective function. Confirmed on 2026-09-17.
12. **The MCP tool surface is versioned and published** as a machine-readable manifest,
    so an organisation can extend the agent with its own skills without forking it.
    Confirmed on 2026-09-17.
13. **Unstructured data is in scope** for the analytics workstream: documents,
    standards, reports, scans and drawings, with extraction, linking and grounded
    question answering, specified separately before implementation. Agreed in
    principle; the detailed design conversation is open.

## 8. Risks and mitigations

- *Authoring is the hardest part of replacing CATIA.* Mitigation: start from the
  views the corpus already drives (explorer, requirements, state machine,
  activities), add matrices and tables early because they are fast to build and
  heavily used, and track parity against the field-guide backlog table.
- *SysML v2 grammar and API are large.* Mitigation: own a documented subset,
  publish the conformance matrix, and import v1 rather than blocking on v2
  completeness.
- *Multi-user merge complexity.* Mitigation: element locking plus semantic diff;
  the engine diff is already the merge primitive.
- *AI changes damaging models.* Mitigation: model-edit API with rule packs, and
  the gate as mandatory scoring on every AI change.
- *Air-gapped AI.* Mitigation: local model serving by default; customer-provided
  endpoints optional; no data leaves the installation.
- *Migration trust.* Mitigation: the round-trip gate and evidence annex, run on a
  customer model before any commitment.
- *External data quality and licensing.* Mitigation: every dataset carries an owner,
  licence, classification and content hash, and every analysis cites the dataset
  versions it used, so a weak source is visible and correctable rather than
  silently baked into a decision.
- *Analysis drifting from the model.* Mitigation: bindings are versioned model
  content validated against element ids; an analysis whose bindings no longer
  resolve is void and says so.

## 9. What the first organisation pilot looks like

1. Install the platform on a controlled network (S2 plus deployment skeleton).
2. Import their CATIA Magic model and run the fidelity gate; publish the evidence.
3. Run a real review in the workbench: coverage, orphans, matrices, reports.
4. Answer a real decision question end to end: bind the programme product, supplier
   and cost data to the model, ask which candidates comply and which requirement
   excludes the most, and hand the decision maker an answer with its evidence.
5. Let the copilot propose changes; gate and review them as normal change.
6. Coexist: keep authoring in CATIA where they must, sync both ways, prove each
   cycle with the gate.
7. Cut over project by project, never in one risky step.

