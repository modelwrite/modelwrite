# Modelwrite Platform Capability Roadmap

> **For agentic workers:** this is the programme plan. Each slice gets its own
> bite-sized implementation plan (superpowers:writing-plans) at the moment the
> slice starts, and is executed with superpowers:subagent-driven-development plus
> test-driven-development. Slice 1 already has one.

**Goal:** build modelwrite into an organisation-grade open-source MBSE platform
that a company can run its systems engineering on, migrate off CATIA Magic onto
with provable zero-loss fidelity, and make market, supplier and cost decisions
with — not merely document models.

**Spec:** docs/superpowers/specs/2026-09-17-modelwrite-platform-design.md (v2)

**Slice 1 plan:** docs/superpowers/plans/2026-09-17-modelwrite-p0-p1-engine-core.md

## Programme rules (apply to every slice)

1. **The engine is the contract.** Every service, app and connector builds on the
   OKF crate and the gate; nothing bypasses them.
2. **Nothing ships unproven.** A slice is done when its acceptance criteria run
   green in CI and its evidence records are committed in docs/evidence/.
3. **The corpus gates every slice.** The coffee-machine corpus stays the fixed
   proof fixture; each slice adds its own fixtures (larger models, broken models,
   migration pairs, datasets) rather than weakening existing ones.
4. **Legacy parity is tracked explicitly.** The CATIA replacement backlog table in
   the design spec is the definition of done for the replacement claim; parity
   status lives in docs/conformance/parity.md and is updated every slice.
5. **Licence split holds.** Engine, server and analytics service:
   AGPL-3.0-or-later. Interchange, connectors and client libraries: Apache-2.0.
   Business functions: private repository.
6. **AI never marks its own work correct.** Every AI-proposed change goes through
   the model-edit API, the rule packs, and the gate; every analytical answer goes
   through the binding set and carries its provenance.
7. **Determinism is evidence.** A gate run or an analysis given the same inputs
   reproduces byte for byte; that is what makes the result citable.
8. **Each slice ships a usable increment.** If an organisation cannot do something
   new with it, the slice is not finished.

---

## Slice 1 — Foundations and engine core

**Status:** planned in detail at
docs/superpowers/plans/2026-09-17-modelwrite-p0-p1-engine-core.md (11 tasks).

**Capability delivered:** the OKF 1.0 standard, the Rust engine (types,
validation, hashing, semantic diff, graph analysis, the round-trip fidelity gate),
the C ABI, the MCP server, the judge harness, CI, and the evidence annex seeded
with the first recorded gate runs.

**Why it comes first:** OKF and the gate are what make migration, collaboration,
analytics and AI trustworthy. Everything later is a client of this contract.

**Note on framing:** the corpus port in that plan is the proof fixture, not the
product. The platform capabilities below are the product.

---

## Slice 2 — Repository and collaboration

**Capability delivered:** a team runs a shared, versioned, permissioned and audited
model repository, and installs it on their own network. This is the Teamwork Cloud
replacement.

**Workstreams:**

1. **Service skeleton** — a crate group under server/: an axum-based HTTP and
   WebSocket API, configuration, health and metrics endpoints, structured errors,
   an OpenAPI description committed to docs/api/.
2. **Model store** — OKF documents persisted per project with a commit graph: every
   save is a commit carrying parent, author, message, timestamp and the canonical
   OKF hash. Snapshots are content-addressed; the storage abstraction covers
   PostgreSQL (teams) and SQLite (single user and edge).
3. **Versioning and baselines** — branches, tags and named baselines; semantic diff
   between any two commits using the engine diff; three-way merge for concurrent
   branches with conflict reporting at element and edge granularity; revert of a
   commit or an element.
4. **Concurrency control** — element-level and project-level locks with leases and
   expiry; optimistic check-and-set on commit; server-side conflict rejection with
   actionable messages.
5. **Identity and access** — OIDC and LDAP integration, local accounts for
   air-gapped installs, roles (viewer, modeller, reviewer, approver, admin),
   project and branch scoping, token-based CLI and CI access.
6. **Audit and evidence** — append-only audit log for every commit, lock,
   permission change and gate run; server-side gate execution that writes evidence
   records on demand and on every protected-branch change.
7. **Deployment skeleton** — deploy/docker-compose.yml for a single-node install, a
   Helm chart for Kubernetes, and an air-gapped bundle script that packages images,
   migrations and the offline AI assets.
8. **CLI** — a modelwrite command line for login, project and branch operations,
   pull, commit, diff, gate and evidence, so CI and scripts never need a browser.

**Interfaces produced:** REST and WebSocket API for projects, commits, branches,
baselines, locks, users and roles, gate runs and evidence; the modelwrite CLI;
server-side gate execution with the same evidence schema as mw-gate.

**Acceptance criteria (proven, not claimed):**

- Two clients commit to the same project concurrently; the second either merges
  cleanly or is rejected with an element-level conflict report; a test drives both
  paths.
- A branch, merge, baseline and revert cycle on the corpus is exercised in an
  integration test; every commit hash and the final OKF hash are asserted.
- The gate runs server-side on a protected branch and refuses a commit that loses
  elements, orphans nodes or breaks the graph; the refusal writes evidence.
- Role checks are tested: a reviewer cannot commit, a modeller cannot change
  permissions, an admin can.
- A docker compose up on a clean machine serves the API and the corpus, and the CLI
  completes a login, commit and gate cycle against it.

**New fixtures:** a multi-branch merge scenario (two divergent OKF snapshots with a
resolvable conflict and one unresolvable), committed under sample/scenarios/.

**Indicative size:** the largest single slice; roughly comparable to the engine
core plus its own integration test suite.

---
## Slice 3 — Authoring workbench

**Capability delivered:** engineers build and review models in the browser without a
legacy licence. This is the MagicDraw and Cameo authoring surface replacement.

**Workstreams:**

1. **Workbench shell** — apps/web: project picker, tree, tabbed views, command
   palette, keyboard-first navigation, offline-tolerant client that talks to the
   repository API from Slice 2.
2. **Structure authoring** — block definition tree and diagram, parts, ports and
   interfaces, drag-to-create with live validation; rename and move operations that
   preserve the graph (no dangling references, ever).
3. **Requirements workspace** — requirement capture with ids and text, satisfy,
   refine, verify and allocate links, coverage warnings as you type, bulk import from
   Excel and ReqIF via the interchange layer.
4. **Behaviour authoring** — state machine editor (states, transitions, triggers,
   effects, entry and do behaviours) and activity editor with swim-lanes and control
   flow, wired to the twin engine for execution.
5. **Parametrics** — constraint blocks and value properties, a solver-backed
   parametric view, and requirement verification status computed from results.
6. **Matrices and tables** — dependency and traceability matrices, generic tables,
   instance tables; editable cells that write back through the repository API.
7. **Patterns library** — engine/patterns crate plus a catalogue UI: reusable
   architecture patterns (redundancy, sensing and control, safety interlock, power
   distribution) that instantiate into a model as named elements with parameter
   substitution, and that the rules engine can check for conformance.
8. **Rules and live validation** — engine/rules crate: declarative rules (traceability
   coverage, orphan elements, naming conventions, pattern conformance, interface
   completeness) that run in the editor, in the CLI and in CI, with severities and
   waivers that carry justification.
9. **SysML v2 textual editing** — engine/sysml2 crate: parse and emit the documented
   SysML v2 subset, round-trip it to OKF with the same fidelity gate used for legacy
   imports, and show textual and graphical views of the same model side by side.
10. **Simulation** — the twin engine productised: run a state machine with scenarios
    and fault injection, solve parametrics, produce time-series traces and instance
    tables, and attach verification evidence to requirements.

**Interfaces produced:** the workbench application; engine/rules rule specification;
engine/patterns pattern specification; engine/sysml2 parse and emit API; simulation
run and trace format.

**Acceptance criteria:**

- A modeller creates a new model from the patterns library in the browser, links
  requirements to structure, and commits it; the server stores it as OKF and the
  gate passes.
- Rename and move operations on the corpus are exercised by tests that assert zero
  dangling references and an unchanged element count.
- The SysML v2 subset round-trips the corpus representation: emit text, parse it
  back, and gate for equality, with coverage of the subset documented in
  docs/conformance/sysml2.md.
- A simulation scenario runs the coffee-machine state machine, produces a trace and
  marks requirement verification status; the trace is committed as evidence.

**New fixtures:** a SysML v2 textual encoding of the corpus and an expected trace for
one simulation scenario, both under sample/.

**Indicative size:** large; the workbench is the visible product and should be built
view by view, each view shipping on its own.

---

## Slice 4 — Interoperability and migration (seamless, both directions)

**Capability delivered:** an organisation moves off CATIA Magic incrementally, and
keeps exchanging data with the tools it still owns, with zero-loss proof at every
step. This is the seamless import and expose capability.

**Workstreams:**

1. **CATIA Magic kit** — importers/magic-groovy-kit: the export and import macros from
   the collateral, hardened and productised: session-wrapped writes, file-verified
   results, association ends handled correctly, profile packages excluded, state
   entry and do behaviours traversed properly. Delivered as a versioned macro bundle
   with a compatibility matrix per CATIA release.
2. **Teamwork Cloud adapter** — read models and write changes through the TWC API, so
   teams that cannot leave TWC immediately still get the gate and the workbench on
   top of their existing repository.
3. **Watch-folder and agent sync** — a local agent that watches mdzip exports,
   produces OKF, runs the gate, commits to the repository, and can push accepted
   edits back into CATIA through the macro kit. This is how coexistence works in
   practice.
4. **ReqIF, Excel and CSV** — bidirectional requirements exchange with id preservation
   and a diff report, so requirements tools and spreadsheets stay in step with the
   model.
5. **XMI import** — IBM Rhapsody and Sparx Enterprise Architect, covering the second
   tier of legacy estates.
6. **SysML v2 API and JSON** — expose the platform through the standardised API so
   other v2 tools interoperate without a bespoke connector.
7. **Migration service** — a guided, resumable migration: assess a legacy model,
   report gaps and fidelity, migrate in stages, and publish a signed fidelity report
   per stage; the judge harness scores each stage so an organisation can audit the
   migration rather than trust it.
8. **Exit guarantee** — documented, tested export of an entire repository to OKF,
   SysML v2 and ReqIF, so no customer is ever locked in. The exit path is a feature,
   not an afterthought.

**Acceptance criteria:**

- The corpus round-trips in both directions: CATIA export to OKF to CATIA and back to
  OKF, with the gate proving element, edge and attribute equality at each hop.
- The macro path and the file-based path both have recorded evidence runs in
  docs/evidence/, including a deliberate failure case.
- A requirements round-trip through ReqIF and through Excel preserves every
  requirement id and text, asserted by test.
- A full repository export to OKF, SysML v2 and ReqIF is produced and re-imported
  cleanly, proving the exit guarantee.

**New fixtures:** legacy import and export pairs, a ReqIF requirements file, an Excel
requirements workbook, and an XMI sample model, all under sample/imports/.

**Indicative size:** large; the CATIA kit and the migration service carry the
commercial weight of the replacement claim.

---

## Slice 5 — AI automation

**Capability delivered:** organisations with too few modellers get the platform to do
the skilled repetition — drafting, repairing, reviewing, explaining — and to answer
questions asked in plain language across the model and the bound data together.

**Workstreams:**

1. **Model-edit API** — a narrow, auditable instruction set (create element, add
   relationship, apply pattern, retype, retrace, rename safely) used by both the
   copilot and human scripting; every instruction is validated by the rules engine
   and scored by the gate before it lands.
2. **Copilot service** — ml/: retrieval grounded in OKF, tool calls into the
   model-edit API, local model serving by default (air-gapped friendly) with optional
   customer endpoints; every proposal is a draft commit a human reviews, never a
   silent edit.
3. **Agent rule packs** — agents/: extend the existing invariants with
   modelling-specific rules, and publish the modelwrite MCP server so general coding
   agents can do MBSE tasks with the gate as their scorer.
4. **ML quality models** — trained on labelled corpora (starting with the
   coffee-machine before and after states): model smell and gap detection,
   completeness scoring, similarity search across an organisation model corpus, and
   duplicate requirement clustering.
5. **Recommendation engine** — suggest missing relationships, likely satisfying blocks
   for uncovered requirements, candidate patterns for a subsystem shape, and impact
   sets for a proposed change.
6. **Ask the model** — grounded natural-language question answering over the model
   itself: retrieval over OKF, answers cited to element ids and the model commit, and
   refusal with a reason when the model does not contain the answer. Question answering
   across external structured and unstructured data belongs to the analytics workstream
   (Slice 7), which owns the bindings that make such answers citable.
7. **Review automation** — generate a review pack for a branch: what changed, what
   broke coverage, which patterns are violated, what the gate found, what the copilot
   suggests; post it to the repository as a review artefact.
8. **Trade studies and variants** — define variant parameters and measures of
   effectiveness, run scored comparisons using the simulation service against measures
   defined in the model, and record the decision with its evidence. External market and
   cost data joins these studies in Slice 7, where the bindings live.
9. **The shipped MCP agent** — agent/: an MCP client of the modelwrite server with a
   skill-pack system, a provider abstraction (local model first, customer endpoint or
   approved cloud optionally), step, token and wall-clock budgets, and a replayable run
   log. It ships inside the platform so an organisation gets AI-generated MBSE without
   building its own harness, and so third-party agents can bind to the same contract.
10. **Generation skills** — pattern-driven synthesis: intake requirements from a
    workbook, ReqIF or document set; select and instantiate patterns from the pattern
    library; name and trace consistently; iterate against the gate until it passes.
    Patterns are the generation substrate, which is what keeps generated structure sound
    rather than invented.
11. **Repair skills** — turn gate and rule findings into a work queue and drive the
    model to a passing state, explaining each change and escalating when a finding needs
    a human decision rather than guessing one.
12. **Agent evaluation harness** — a labelled task set (generate, repair, review) with
    published pass criteria and results, so agent capability is measured the way the
    gate measures fidelity. Ambiguity handling is part of the task set: the agent must
    ask rather than invent.

**Acceptance criteria:**

- The copilot drafts a change against the corpus (for example tracing uncovered
  requirements to candidate blocks); the proposal appears as a draft commit; the gate
  scores it; a rejected proposal leaves the model unchanged.
- The quality models are trained and evaluated on a held-out split with published
  precision and recall in docs/evidence/, not asserted informally.
- An MCP client (a coding agent) completes a modelling task end to end using only the
  published tools, and its result is gated.
- A plain-language question about the model (for example which requirements are still
  uncovered) is answered with citations to element ids and the model commit; a question
  the model cannot answer is refused with the reason.
- A variant trade study produces a scored comparison with a recorded decision.
- **AI-generated MBSE, end to end:** from a requirements workbook plus a stated pattern
  set, the shipped agent produces a corpus-scale model with zero human edits that passes
  validation, the rules at target coverage, graph integration (no orphans, one component)
  and the round-trip gate; the run replays deterministically from its log and its
  evidence is committed.
- Given the corrupted corpus fixture, the agent repairs it to a passing state and
  explains each repair in the review pack.
- The agent asks rather than invents when a requirement is ambiguous, demonstrated by a
  case in the evaluation set.
- The published tool manifest is sufficient for an externally authored skill pack to run
  without modifying the agent.

**New fixtures:** a labelled before and after model pair for smell training, a variant
pair for the trade study, a question set with expected answers for model question
answering, and an agent task set (a small requirements workbook, its expected
gate-passing model, and the corrupted corpus for repair), under sample/ and
sample/agent/.

**Indicative size:** large. Start with the model-edit API, the MCP tool contract and the
agent runtime, then the generation and repair skills, then the ML models once labelled
data exists.

---

## Slice 6 — Governance, assurance and operations

**Capability delivered:** a regulated organisation can run, audit and certify the
platform, and install it on a controlled network.

**Workstreams:**

1. **CI model gates** — a container and action that runs validation, rules, the gate
   and evidence generation on every model change in the customer pipeline, with
   configurable severity thresholds and waivers.
2. **Baselines and approvals** — formal baselines with sign-off, change requests that
   reference evidence, electronic approval records, and immutable release snapshots of
   the model plus its evidence bundle.
3. **Reporting** — the report wizard equivalent: templated Word, HTML and PDF reports
   straight from the model (requirement coverage, traceability matrices, interface
   lists, verification status) plus analytics packs from Slice 5, generated in CI so
   documents never drift from the model.
4. **Conformance matrix** — docs/conformance/: honest status of SysML v2 subset
   coverage, OKF coverage of legacy constructs, and CATIA parity against the
   field-guide backlog table.
5. **Air-gapped and hardened deployment** — offline install bundle, no telemetry,
   signed release artefacts, SBOM, dependency and container scanning, backup and
   restore procedures, upgrade and store migration.
6. **Identity and enterprise integration** — SSO behaviour documented and tested,
   service accounts, audit export to a customer SIEM.
7. **Performance and scale evidence** — published measurements on a large synthetic
   model (hundreds of thousands of elements, thousands of commits, multi-gigabyte
   datasets), recorded in the evidence annex in the spirit of the bylazora benchmarks.
8. **Operations documentation** — runbooks for install, upgrade, backup, restore and
   incident response, plus a pilot playbook.

**Acceptance criteria:**

- A model change in a customer-style pipeline fails the build when it breaks coverage
  or the graph, and the failing run publishes evidence.
- A baseline with approvals is created, modified afterwards, and proven immutable by
  test.
- The report generator produces a requirement coverage report from the corpus that
  matches the computed coverage exactly (no hand-written numbers).
- An air-gapped install completes from the bundle on a machine with no network,
  including the local AI and analytics assets.
- Scale measurements are published with the hardware, dataset and command used to
  reproduce them.

**Indicative size:** medium, but it is the slice that makes enterprise adoption
possible; it should follow the first successful pilot rather than precede it.

---

## Slice 7 — Analytics and data fusion (the long-horizon workstream)

**Capability delivered:** the organisation brings its external data into the platform,
binds it to the model, and asks decision questions across both: which products meet the
specification, is there anything on the market that does, and which requirements are
the most expensive.

**Framing, deliberately different from the other slices:** this is not a single
bounded slice. Market, cost and supplier analytics is a much larger conversation about
ingesting many additional data sources — structured and unstructured — into the
analytics engine, and it grows with every source an organisation adds. It is placed
last because it is the longest-horizon workstream, and it should be designed as its own
programme (its own spec and its own plans) rather than squeezed into this roadmap. The
workstreams below are the shape of that programme, not a fixed scope.

**Two halves, and the second is the harder one:**

- **Structured analytics:** the tabular world — product catalogues, price lists, ERP
  and PLM exports, telemetry and test data, cost models. Bindings, SQL and deterministic
  query packs make these answers citable today.
- **Unstructured analytics:** the world that holds most of an organisation knowledge —
  standards and regulations, specifications, test reports, supplier documents, drawings
  and models, meeting records, emails, maintenance histories. This needs ingestion,
  extraction (text, tables, figures, OCR for scans), entity and relation extraction,
  embedding and vector search, and a link from extracted concepts back to model
  elements. It is where the largest gains and the largest risks live, because an
  ungrounded answer over documents is exactly the confident nonsense this platform
exists to replace.

**Workstreams:**

1. **Data fabric and dataset registry** — data/: connectors for CSV, Excel, JSON,
   REST, Parquet, PostgreSQL and SQL Server; a registry carrying owner, licence,
   classification, refresh policy and content hash per dataset; versioned imports
   with a reproducible manifest; signed dataset bundles for air-gapped installs.
2. **Semantic bindings** — extend OKF to 1.1 with a bindings section: requirement
   threshold to dataset column, block or part to supplier entity, value property to
   measured field, measure of effectiveness to derived column. Bindings are
   validated against element ids, diffed and gated like model content, and the
   validator rejects a binding to a missing element. The workbench gains a binding
   editor and a coverage view: which requirements are measurable against real data,
   and which are untestable claims.
3. **Analysis engine** — a service wrapping DuckDB: SQL over registered datasets plus
   the model export, exposed as named query packs, with results tied to dataset
   versions and the model commit hash.
4. **Decision analyses** — the analysis packs themselves: compliance matrix,
   feasibility gap (nothing on the market meets it, and by how much), cost drivers
   (exclusion count, price premium, cost of relaxing each requirement), sensitivity
   and what-if, make versus buy, and trade studies scored against measures of
   effectiveness defined in the model.
5. **Dashboards and report packs** — interactive compliance and sensitivity views in
   the workbench, a cost-driver Pareto, supplier gap analysis, and generated packs
   for design reviews and procurement.
6. **Provenance and evidence** — every analysis records the model commit, dataset
   versions, bindings used and the executed query; results are re-runnable and saved
   as evidence records, so a decision answer is a citation rather than an opinion.
7. **Data governance** — dataset access inherits repository roles; licence and
   classification travel with the data; exports respect classification.
8. **Unstructured ingestion and extraction** — pipelines for documents: PDF, Office,
   HTML, images and scans, CAD-side metadata; text and table extraction, OCR for
   scans, figure and caption capture; every extracted fragment keeps its source
   document, page and offsets so it can be cited. Extraction runs offline and its
   output is versioned like any other dataset.
9. **Knowledge extraction and linking** — entities, requirements mentions, standards
   clauses, part numbers, test results and supplier claims extracted from documents and
   mapped onto model elements, producing a provenance-carrying link graph rather than a
   pile of text. Human confirmation is part of the flow: proposed links are reviewed,
   and accepted ones become model content or bindings.
10. **Retrieval and grounded question answering** — embedding and vector search over
    extracted content combined with structured queries, so a question can be answered
    from a spread of datasets plus the model, with every claim cited to its document,
    page and element. Retrieval quality is measured (recall against a labelled question
    set) and published, and an answer with no supporting source is refused.

**Interfaces produced:** the dataset registry and connector API; the OKF 1.1 bindings
section; the query pack format; the analysis result and provenance schema; analytics
views in the workbench.

**Acceptance criteria:**

- The corpus requirements are bound to a real product dataset (the public
  espresso-machine data from the collateral) and the compliance matrix reproduces the
  collateral result: which machines pass, and which requirement excludes the most.
- The cost-driver analysis answers for the corpus which requirement is the most
  expensive to hold, with the excluded-candidate count and the price delta as its
  evidence; the run is committed as an evidence record.
- Relaxing the binding constraint threshold moves the cheapest conforming option,
  demonstrated by test rather than by inspection.
- A binding to a deleted requirement fails validation and blocks the commit.
- Two runs of the same analysis with the same model commit and dataset versions
  produce byte-identical results.
- An air-gapped install imports a dataset bundle and runs the same analyses.
- A standard or regulation in document form and a test report are ingested, extracted
  and linked to the requirements they bear on; a question answered from them cites the
  document, page and model element, and the link is reviewable by a human.
- Retrieval quality is measured against a labelled question set and published, not
  asserted; a question with no supporting source is refused with the reason.

**New fixtures:** a products dataset (CSV and Parquet), a supplier price list, a cost
model, a bindings file for the corpus, and a small document corpus (a standard extract,
a specification and a test report) with a labelled question set, under sample/analytics/.

**Indicative size:** unbounded by nature. The structured half is medium; the
unstructured half is a programme in its own right, and should be specified, sized and
planned separately before any of it is built.

---
## Sequencing

| Order | Slice | Depends on | Rough size | Ships to whom |
|---|---|---|---|---|
| 1 | Slice 1 foundations and engine core | — | large, planned | Maintainers and CI |
| 2 | Slice 2 repository and collaboration | Slice 1 | largest | A team on a shared server |
| 3 | Slice 3 authoring workbench | Slice 2 | large | Modellers without a legacy licence |
| 4 | Slice 4 interoperability and migration | Slice 1, Slice 2 | large | An organisation with an existing CATIA estate |
| 5 | Slice 5 AI automation, including the shipped MCP agent | Slice 3 | large | Under-resourced teams: AI-generated models, repair and review, and anyone with a question |
| 6 | Slice 6 governance and operations | Slice 2 to Slice 5 | medium | Regulated enterprise |
| 7 | Slice 7 analytics and data fusion | everything above; structured half needs Slice 2, unstructured half needs its own spec | unbounded | Programme, engineering, procurement and supplier decisions |

Two ordering decisions worth stating plainly:

- **Interoperability (Slice 4) comes before AI (Slice 5)** because migration and
  coexistence are what unblock a real organisation, and they need only the engine and
  the repository. AI then multiplies a platform the organisation is already using.
- **Analytics (Slice 7) comes last, deliberately.** Market, cost and supplier analytics
  is not a bounded feature: it is the workstream that keeps ingesting new data sources,
  structured and unstructured, for as long as the organisation keeps finding them. Its
  structured half needs the repository and the engine; its unstructured half (documents,
  standards, reports, scans, drawings) needs a design conversation of its own, with its
  own spec, before any code is written. Placing it last does not reduce its importance —
  it gives it the room it needs instead of forcing it into a slice shape it does not
  fit.

## Definition of done for the replacement claim

An organisation can leave CATIA Magic when, for the work it actually does:

1. every capability in the parity table of the design spec has a working equivalent,
   or a documented, accepted gap;
2. the migration of its models is proven by a signed fidelity report;
3. its reviews, approvals and reports run inside modelwrite;
4. its engineers can author the model without a legacy licence;
5. its market, supplier, cost and trade-off questions are answered inside the platform
   with citations, not in a parallel spreadsheet (this is the Slice 7 horizon, and it is
   the last thing to arrive);
6. its data can leave again, in the open standard, at any time.

That is the standard the roadmap is held to; nothing less counts as a replacement.

## Open decisions before Slice 2 starts

1. Confirm SysML v2 as the primary authoring and interchange standard, with SysML v1
   and CATIA arriving through OKF mapping.
2. Confirm PostgreSQL plus a Rust service as the team repository store, with SQLite
   for single-user installs.
3. Confirm the browser-first workbench as the authoring surface.
4. Confirm element locking plus branch and merge before any real-time co-editing.
5. Confirm the slice order above: interoperability before AI, and analytics last as the
   long-horizon workstream with its own spec.
6. Confirm the first pilot target (which organisation, which model, which controlled
   network) so Slices 2, 4 and 5 can be shaped around it.

**Confirmed on 2026-09-17 (structured analytics foundations, to be honoured when Slice 7
starts):**

7. Analytics stack: embedded DuckDB over Parquet and registered datasets, with
   PostgreSQL optional for shared catalogues and no separate warehouse server.
   — confirmed.
8. Bindings as model content (an OKF 1.1 bindings section, versioned and gated) rather
   than an external mapping file. — confirmed.
9. First external data sources: product catalogues, price lists and supplier data, then
   ERP and PLM exports and test and telemetry data. — confirmed.
10. Unstructured data (documents, standards, reports, scans, drawings, correspondence)
    is in scope for Slice 7, and gets its own spec, question set and evaluation before
    implementation. — agreed in principle; the detailed design conversation is open.

