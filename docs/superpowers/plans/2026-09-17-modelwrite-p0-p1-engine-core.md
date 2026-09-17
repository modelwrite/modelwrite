# Modelwrite Phase 0 + Phase 1 (Engine Core) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (- [ ]) syntax for tracking.

**Goal:** Ship the modelwrite engine core: OKF 1.0 types, validation, hashing and diffing; graph analysis; the round-trip fidelity gate with evidence recording; the C ABI; the MCP server; and the judge harness — all proven against the coffee-machine corpus.

**Architecture:** A Rust workspace under engine/ with five crates (okf, graph, gate, capi, mcp) plus a dev-only test-support crate. Everything speaks OKF JSON; the gate is the only authority on correctness. Python is used only for the judge harness and sample scripts, stdlib only.

**Tech Stack:** Rust stable (edition 2021, rust-version 1.75), serde/serde_json, petgraph, sha2/hex, anyhow; Python 3.12+ stdlib; GitHub Actions CI on Ubuntu.

**Spec:** docs/superpowers/specs/2026-09-17-modelwrite-design.md (Sections 4.1 OKF, 4.1 engine crates, 4.2 repo structure, and Phase 0 + Phase 1 of 4.3)

**Scope note:** this plan is **Slice 1 (engine core) of the platform capability roadmap** at docs/superpowers/plans/2026-09-17-modelwrite-capability-roadmap.md. That roadmap builds modelwrite into an organisation-grade open-source MBSE platform that a company can run its systems engineering on and migrate off CATIA Magic onto. The platform design is docs/superpowers/specs/2026-09-17-modelwrite-platform-design.md.

The coffee-machine corpus in this plan is the **proof fixture, not the product**: it proves the OKF contract and the gate that every later capability (repository, workbench, interoperability, analytics, AI, governance) depends on. Those later slices get their own plans.

**Forward compatibility note:** the OKF types in Task 4 do not deny unknown fields, so later additive sections (for example the OKF 1.1 analytics bindings section defined in Slice 5, which binds model elements to external datasets) parse cleanly and are ignored by this version. Do not add `deny_unknown_fields` to any type in this plan.

## Global Constraints

- Rust stable is installed in Task 1 via rustup; edition 2021; rust-version 1.75; workspace dependencies pinned in the root Cargo.toml.
- Every engine source file starts with the line: // SPDX-License-Identifier: AGPL-3.0-or-later
- Python code (judge/, sample/scripts/) is stdlib only; no pip installs; invoked as python.
- Conventional commits (feat:, fix:, docs:, test:, ci:, chore:); commit at the end of each task exactly as directed.
- collateral/ is read-only source material. Canonical copies live under sample/ from Task 3 onward. Never edit anything inside collateral/.
- Binary names: mw-gate, mw-mcp. Library names: okf, graph, gate, capi, mcp, test_support. Package names: mw-okf, mw-graph, mw-gate, mw-capi, mw-mcp, mw-test-support.
- Corpus numbers pinned in tests: 99 graph nodes, 165 graph edges, 25 requirements, 49 blocks, 9 signals, 8 activities, 1 connected component, 0 isolated nodes.
- The local machine is Windows; commands are PowerShell with forward-slash paths. CI runs on Ubuntu. Rust tests spawn no child processes (sandbox-safe); only the judge smoke steps in Task 10 spawn the gate binary.
- No external network services are required at build or test time.
- Repo naming follows design section 5 assumption 7 (GitHub org modelwrite). If the real org or remote URL differs, adjust only the git remote step in Task 1 and the repository field in the root Cargo.toml.

---

### Task 1: Repository scaffold

**Files:**
- Create: .gitignore
- Create: .gitattributes
- Create: LICENSE (AGPL-3.0-or-later full text)
- Create: CODE_OF_CONDUCT.md
- Create: CONTRIBUTING.md
- Create: SECURITY.md
- Create: NOTICE.md
- Create: README.md (stub; completed in Task 11)
- Create: agents/CLAUDE.md
- Create: docs/okf/ (empty directory; filled by Task 2)
- Create: docs/evidence/ (empty directory; filled by Task 6)
- Create: sample/ (empty directory; filled by Task 3)

**Interfaces:**
- Consumes: nothing.
- Produces: a git repository at the repo root with main as the default branch; the AGPL licence text; the rule pack in agents/CLAUDE.md that every later agent task must respect; a working Rust toolchain.

- [ ] **Step 1: Install the Rust toolchain (rustup, stable, minimal profile)**

Run in PowerShell from the repo root:

```powershell
Invoke-WebRequest -Uri https://win.rustup.rs -OutFile "$env:TEMP\rustup-init.exe"
& "$env:TEMP\rustup-init.exe" -y --default-toolchain stable --profile minimal
$env:Path = "$env:USERPROFILE\.cargo\bin;" + $env:Path
```

If the download is blocked, fall back to: winget install --id Rustlang.Rustup -e --accept-source-agreements --accept-package-agreements

- [ ] **Step 2: Verify the toolchain**

Run: cargo --version and rustc --version
Expected: both print a stable version (1.75 or newer).

- [ ] **Step 3: Initialise the repository**

```powershell
git init -b main
```

Expected: Initialized empty Git repository in ... If .git already exists, skip this step. Note: do not add a remote yet; when the GitHub remote URL is confirmed, run git remote add origin URL as a separate commit-free step.

- [ ] **Step 4: Write .gitignore**

```
/target/
**/__pycache__/
*.pyc
.cache/
.DS_Store
Thumbs.db
```

- [ ] **Step 5: Write .gitattributes**

```
*.mdzip binary
*.zip binary
```

- [ ] **Step 6: Write LICENSE (AGPL-3.0-or-later)**

Download the verbatim licence text:

```powershell
Invoke-WebRequest -Uri https://www.gnu.org/licenses/agpl-3.0.txt -OutFile LICENSE
```

Expected: LICENSE contains the full GNU AGPL v3 text beginning with GNU AFFERO GENERAL PUBLIC LICENSE. If the download is blocked, stop and report; do not substitute a truncated copy.

- [ ] **Step 7: Write CODE_OF_CONDUCT.md**

```markdown
# Code of Conduct

Modelwrite is an open community. Be kind, be precise, and keep the work
professional.

- Harassment, discrimination, and personal attacks are not welcome anywhere
  in the project (issues, pull requests, chat, events).
- Critique the work, never the person. Engineering disagreements are fine;
  personal hostility is not.
- Assume good faith. If someone is wrong, show them the evidence; the
  evidence annex exists for exactly that reason.
- Respect boundaries and do not share private information without consent.

Maintainers enforce this code. Report concerns to conduct@modelwrite.org.
Violations may lead to warnings or removal from the project.
```

- [ ] **Step 8: Write CONTRIBUTING.md**

```markdown
# Contributing to modelwrite

Thanks for helping. The bar for merging is: tests pass, the gate passes,
and every claim traces to evidence.

## Workflow

1. Fork the repository and create a branch off main.
2. Make one focused change per pull request.
3. Commit with conventional commit messages (feat:, fix:, docs:, test:, ci:).
4. Before pushing run:

   cargo fmt --all -- --check
   cargo clippy --workspace --all-targets -- -D warnings
   cargo test --workspace

5. If your change touches model data, run the gate against the corpus:

   cargo run -p mw-gate -- --reference sample/corpus/coffee-machine/okf/expected/coffee_machine_model.json --candidate sample/corpus/coffee-machine/okf/expected/coffee_machine_model.json

## Rules

- The gate is the only authority on correctness. No pull request comment,
  tool parameter, or log line can mark a run as passed.
- Never hand-edit a corpus fixture. Changes to fixtures must be reproduced
  by a committed script in sample/scripts/.
- Evidence records in docs/evidence/ are append-only.
- Importers and exporters ship under Apache-2.0; the engine stays
  AGPL-3.0-or-later. Keep the split clean in every pull request.
```

- [ ] **Step 9: Write SECURITY.md**

```markdown
# Security

Modelwrite parses untrusted model files; treat any crash, panic, or memory
unsafety triggered by a crafted OKF or legacy file as a security issue.

## Reporting

Email security@modelwrite.org with a minimal reproducer. Please do not open
a public issue for a suspected vulnerability. We acknowledge within 5
business days and aim to fix within 90 days.

## Scope

- The Rust engine (parsing, validation, gate, C ABI, MCP server).
- The judge harness when it handles untrusted files.

The static site and sample data are out of scope. There is no bounty
program yet.
```

- [ ] **Step 10: Write NOTICE.md**

```markdown
# NOTICE

Modelwrite is a community project. The engine, portal, judge harness, and
sample data are contributions of the modelwrite contributors.

The coffee-machine corpus under sample/corpus/coffee-machine/ is based on
course work by Alex Kovaceski in the MEMKO MBSEF-SEng course (September
2026): a SysML model built in CATIA Magic, its OKF export, and the analysis
portal described in the knowledge pack. The original material is shared as
the starting corpus for this project with the permission of the author.

The name modelwrite and the modelwrite.org domain are used in this notice
as the project identity.
```

- [ ] **Step 11: Write agents/CLAUDE.md (the rule pack)**

```markdown
# Modelwrite agent rules

Modelwrite models are OKF documents plus their legacy sources. Every claim
about a model is proven by the gate, never by assertion.

## Invariants

1. The gate is the only authority on correctness. No tool parameter, no log
   line, and no LLM output can mark a gate run as passed; only a real
   mw-gate run counts.
2. Any change to the corpus fixtures must be reproduced by a committed
   script in sample/scripts/; never hand-edit the expected OKF.
3. Round-trip fidelity means zero loss: element sets, edge sets, and
   attributes must match exactly between reference and candidate OKF.
4. Graph health is non-negotiable: a candidate with isolated nodes or more
   than one connected component fails the gate.
5. OKF ids are opaque and stable. Never regenerate or renumber ids when
   transforming a model; a missing id is data loss.
6. Traceability labels are capitalised exactly: Satisfy, Refine, Verify,
   Allocate. Dependency edges carry them in the label field.
7. State entry/do behaviour references point at activity names, and the
   activity section keeps every activity the state machine reaches.
8. Evidence goes to docs/evidence/ and is committed; never rewrite an
   existing evidence record.
```

- [ ] **Step 12: Write README.md stub**

```markdown
# Modelwrite

**The model you can prove.**

Modelwrite is an open-source MBSE platform that turns legacy SysML models
into portable, provable data — the Open Knowledge Format (OKF) — and
automates the skilled work of building and reviewing models with grounded
AI.

Status: engine core under construction. See docs/superpowers/plans/ for the
implementation plan and docs/superpowers/specs/ for the design.

Quickstart once the engine lands:

cargo build --workspace
cargo test --workspace

Licence: AGPL-3.0-or-later (engine). See NOTICE.md for attribution.
```

- [ ] **Step 13: Verify and commit**

```powershell
Get-ChildItem -Name
git status --short
git add -A
git commit -m "chore: scaffold modelwrite repository"
```

Expected: the listing shows the ten created files plus docs/ and sample/; the commit lands on main.

---
### Task 2: OKF 1.0 specification and JSON Schema

**Files:**
- Create: docs/okf/okf-1.0-spec.md
- Create: docs/okf/okf-1.0.schema.json

**Interfaces:**
- Consumes: nothing.
- Produces: the normative OKF 1.0 field names, kind enums and validation rules that every later task implements verbatim. The Rust validator in Task 4 is the runtime conformance checker; this document and the schema are the readable descriptions.

- [ ] **Step 1: Write docs/okf/okf-1.0-spec.md**

```markdown
# OKF 1.0 — Open Knowledge Format

Status: normative, v1.0-draft

Normative description of the modelwrite interchange format. The Rust
validator in engine/okf is the runtime conformance checker; this document
and okf-1.0.schema.json are the human and tool readable descriptions.

## 1. Purpose

OKF is a single self-describing JSON document that captures a systems
model: its elements, its requirements, its state machine and activities,
and a graph of every relationship. One OKF file drives every downstream
consumer: portals, analysis, AI, and the gate.

## 2. Top level

| Field | Type | Required | Meaning |
|---|---|---|---|
| okf | string | no | Version marker; exporters emit 1.0. Absent means legacy export. |
| project | string | yes | Model or project name. |
| exportedAt | string | yes | Export timestamp. Exporters SHOULD emit ISO-8601; legacy exports may be free text. |
| provenance | object | no | sourceTool, exporter, exporterVersion strings. |
| summary | object | yes | Counts used as a sanity cross-check (mismatches are warnings). |
| structure | Element[] | no | Blocks and interface blocks. |
| interfaces | Element[] | no | Ports and interfaces (empty in the reference corpus). |
| signals | Element[] | no | Signals and events. |
| requirements | Requirement[] | no | Requirements with reqId and reqText. |
| stateMachine | object | yes | name plus regions of states. |
| activities | Activity[] | no | Activity diagrams: nodes and control edges. |
| graph | object | yes | Every element as a node and every relationship as an edge. |

## 3. Element kinds and edge kinds

Node kinds: activity, actor, block, interface, requirement, signal, state,
stateMachine, usecase.

Edge kinds: association, behavior, contains, dependency, effect,
generalization, include, part, reference, subject, transition, triggers,
uses.

Traceability lives on dependency edges in the label field with exactly
these spellings: Satisfy, Refine, Verify, Allocate (capitalised).

## 4. Section item shapes

Element: id (required, opaque, stable), name, kind, stereotypes[],
attributes[] (each with name, type, aggregation, default), documentation.

Requirement: everything in Element plus reqId (required, non-empty) and
reqText.

State: id (required), name, entry, doActivity, exit. The entry, doActivity
and exit fields reference behaviour names, not ids.

StateMachine: name plus regions[]; each region holds states[].

Activity: name, partitions[] (swim-lane names), nodes[] (each with id,
type, name, partition) and edges[] (each with type, source, target,
guard). Control edges may reference node ids or be empty.

Graph node: id (required), kind, name, stereotypes[].

Graph edge: source, target, kind (required), label.

## 5. Graph rules

- Every node has an id unique within the graph.
- Every edge endpoint must reference an existing node id.
- Node and edge kinds must come from the tables above.
- Element ids are unique across all sections.

## 6. Summary cross-check

The summary counts (blocks, requirements, interfaces, signals, activities,
graphNodes, graphEdges) are compared with the actual section sizes; a
mismatch is a warning, never a failure.

## 7. Validation rules (normative list)

1. The okf marker is empty or 1.0.
2. project is non-empty.
3. graph and stateMachine are present.
4. No duplicate element ids; no empty ids.
5. Every requirement has a non-empty reqId.
6. The graph has at least one node; all kinds come from the tables.
7. Every edge endpoint resolves to a node; all kinds come from the tables.
8. Summary counts match section sizes (warning only).
9. A legacy document without the okf marker produces a warning, not an
   error.

## 8. Hashing

The canonical hash of an OKF snapshot is sha256 over the serialized
document with the structural field order of the Rust types. It is a
fingerprint of the snapshot, not a semantic identity: two semantically
equal files that differ in array order hash differently. The gate uses
semantic diffing for equality and hashes for evidence records.

## 9. Conformance

The reference corpus fixture
(sample/corpus/coffee-machine/okf/expected/coffee_machine_model.json) is a
legacy export: no okf marker and a free text exportedAt. It is conformant
with one warning. New exporters MUST emit okf 1.0 and an ISO-8601
exportedAt.
```

- [ ] **Step 2: Write docs/okf/okf-1.0.schema.json**

```json
{
  "$schema": "http://json-schema.org/draft-07/schema#",
  "$id": "https://modelwrite.org/okf/1.0/schema.json",
  "title": "OKF 1.0",
  "type": "object",
  "required": ["project", "summary", "stateMachine", "graph"],
  "properties": {
    "okf": {"type": "string", "enum": ["", "1.0"]},
    "project": {"type": "string"},
    "exportedAt": {"type": "string"},
    "provenance": {
      "type": "object",
      "properties": {
        "sourceTool": {"type": "string"},
        "exporter": {"type": "string"},
        "exporterVersion": {"type": "string"}
      }
    },
    "summary": {
      "type": "object",
      "properties": {
        "blocks": {"type": "integer"},
        "requirements": {"type": "integer"},
        "interfaces": {"type": "integer"},
        "signals": {"type": "integer"},
        "activities": {"type": "integer"},
        "graphNodes": {"type": "integer"},
        "graphEdges": {"type": "integer"}
      }
    },
    "structure": {"type": "array", "items": {"$ref": "#/definitions/element"}},
    "interfaces": {"type": "array", "items": {"$ref": "#/definitions/element"}},
    "signals": {"type": "array", "items": {"$ref": "#/definitions/element"}},
    "requirements": {"type": "array", "items": {"$ref": "#/definitions/requirement"}},
    "stateMachine": {
      "type": "object",
      "required": ["name"],
      "properties": {
        "name": {"type": "string"},
        "regions": {
          "type": "array",
          "items": {
            "type": "object",
            "properties": {
              "states": {"type": "array", "items": {"$ref": "#/definitions/state"}}
            }
          }
        }
      }
    },
    "activities": {"type": "array", "items": {"$ref": "#/definitions/activity"}},
    "graph": {
      "type": "object",
      "required": ["nodes", "edges"],
      "properties": {
        "nodes": {"type": "array", "items": {"$ref": "#/definitions/graphNode"}},
        "edges": {"type": "array", "items": {"$ref": "#/definitions/graphEdge"}}
      }
    }
  },
  "definitions": {
    "attribute": {
      "type": "object",
      "required": ["name"],
      "properties": {
        "name": {"type": "string"},
        "type": {"type": "string"},
        "aggregation": {"type": "string"},
        "default": {"type": "string"}
      }
    },
    "element": {
      "type": "object",
      "required": ["id"],
      "properties": {
        "id": {"type": "string"},
        "name": {"type": "string"},
        "kind": {"type": "string"},
        "stereotypes": {"type": "array", "items": {"type": "string"}},
        "attributes": {"type": "array", "items": {"$ref": "#/definitions/attribute"}},
        "documentation": {"type": "string"}
      }
    },
    "requirement": {
      "type": "object",
      "required": ["id", "reqId"],
      "properties": {
        "id": {"type": "string"},
        "name": {"type": "string"},
        "kind": {"type": "string"},
        "stereotypes": {"type": "array", "items": {"type": "string"}},
        "attributes": {"type": "array", "items": {"$ref": "#/definitions/attribute"}},
        "documentation": {"type": "string"},
        "reqId": {"type": "string"},
        "reqText": {"type": "string"}
      }
    },
    "state": {
      "type": "object",
      "required": ["id"],
      "properties": {
        "id": {"type": "string"},
        "name": {"type": "string"},
        "entry": {"type": ["string", "null"]},
        "doActivity": {"type": ["string", "null"]},
        "exit": {"type": ["string", "null"]}
      }
    },
    "activityNode": {
      "type": "object",
      "required": ["id"],
      "properties": {
        "id": {"type": "string"},
        "type": {"type": "string"},
        "name": {"type": "string"},
        "partition": {"type": ["string", "null"]}
      }
    },
    "activityEdge": {
      "type": "object",
      "properties": {
        "type": {"type": "string"},
        "source": {"type": "string"},
        "target": {"type": "string"},
        "guard": {"type": "string"}
      }
    },
    "activity": {
      "type": "object",
      "properties": {
        "name": {"type": "string"},
        "partitions": {"type": "array", "items": {"type": "string"}},
        "nodes": {"type": "array", "items": {"$ref": "#/definitions/activityNode"}},
        "edges": {"type": "array", "items": {"$ref": "#/definitions/activityEdge"}}
      }
    },
    "graphNode": {
      "type": "object",
      "required": ["id"],
      "properties": {
        "id": {"type": "string"},
        "kind": {"type": "string"},
        "name": {"type": "string"},
        "stereotypes": {"type": "array", "items": {"type": "string"}}
      }
    },
    "graphEdge": {
      "type": "object",
      "required": ["source", "target", "kind"],
      "properties": {
        "source": {"type": "string"},
        "target": {"type": "string"},
        "kind": {"type": "string"},
        "label": {"type": "string"}
      }
    }
  }
}
```

- [ ] **Step 3: Verify both files parse**

```powershell
python -c "import json,pathlib; json.loads(pathlib.Path('docs/okf/okf-1.0.schema.json').read_text(encoding='utf-8')); print('schema parses')"
python -c "import json,pathlib; d=json.loads(pathlib.Path('docs/okf/okf-1.0.schema.json').read_text(encoding='utf-8')); assert d['required']==['project','summary','stateMachine','graph']; print('required fields pinned')"
```

Expected: schema parses and required fields pinned, both printed.

- [ ] **Step 4: Commit**

```powershell
git add docs/okf
git commit -m "docs: add OKF 1.0 spec and JSON schema"
```

---
### Task 3: Port the coffee-machine corpus into sample/

**Files:**
- Create: sample/corpus/coffee-machine/okf/expected/coffee_machine_model.json (copied fixture)
- Create: sample/corpus/coffee-machine/legacy/CoffeeMachine-SysML-Model.mdzip (copied fixture)
- Create: sample/demo/CoffeeMachine-Showcase-standalone.html (copied fixture)
- Create: sample/README.md
- Create: sample/corpus/coffee-machine/README.md

**Interfaces:**
- Consumes: the read-only sources in collateral/.
- Produces: the canonical fixture paths that every later task and test reads. Paths are fixed from now on: sample/corpus/coffee-machine/okf/expected/coffee_machine_model.json is the reference OKF.

- [ ] **Step 1: Expand the app archive to a temporary directory**

```powershell
$tmp = "C:/Users/alexk/projects/modelwrite/.tmp-corpus"
New-Item -ItemType Directory -Force -Path $tmp | Out-Null
Expand-Archive -Path "collateral/CoffeeMachine-App.zip" -DestinationPath "$tmp/app"
Get-ChildItem "$tmp/app" -Name
```

Expected: the listing shows coffee_machine_model.json, index.html, READ-ME.txt, serve.ps1, upload_server.ps1.

- [ ] **Step 2: Copy the three fixtures into sample/**

```powershell
New-Item -ItemType Directory -Force -Path "sample/corpus/coffee-machine/okf/expected" | Out-Null
Copy-Item "$tmp/app/coffee_machine_model.json" "sample/corpus/coffee-machine/okf/expected/coffee_machine_model.json"
New-Item -ItemType Directory -Force -Path "sample/corpus/coffee-machine/legacy" | Out-Null
Copy-Item "collateral/CoffeeMachine-SysML-Model.mdzip" "sample/corpus/coffee-machine/legacy/CoffeeMachine-SysML-Model.mdzip"
New-Item -ItemType Directory -Force -Path "sample/demo" | Out-Null
Copy-Item "collateral/CoffeeMachine-Showcase-standalone.html" "sample/demo/CoffeeMachine-Showcase-standalone.html"
Remove-Item $tmp -Recurse -Force
```

- [ ] **Step 3: Pin the corpus numbers**

```powershell
python -c "import json,pathlib; d=json.loads(pathlib.Path('sample/corpus/coffee-machine/okf/expected/coffee_machine_model.json').read_text(encoding='utf-8')); assert d['summary']['graphNodes']==99 and d['summary']['graphEdges']==165, d['summary']; assert len(d['requirements'])==25 and d['summary']['blocks']==49 and d['summary']['signals']==9 and d['summary']['activities']==8; print('corpus pinned: 99 nodes, 165 edges, 25 requirements')"
```

Expected: corpus pinned: 99 nodes, 165 edges, 25 requirements.

- [ ] **Step 4: Write sample/README.md**

```markdown
# Sample

The coffee-machine corpus: a complete SysML model and its OKF export,
shared as the regression fixture for the whole project.

- corpus/coffee-machine/okf/expected/ — the exported model (OKF), the
  reference snapshot the gate must accept unchanged.
- corpus/coffee-machine/okf/corrupted/ — a deliberately broken copy the
  gate must reject (created by scripts/make_corrupted.py in Task 6).
- corpus/coffee-machine/legacy/ — the CATIA Magic project (.mdzip).
- demo/ — the standalone portal showcase from the knowledge pack.
- scripts/ — fixture generators; never hand-edit a fixture.

The corpus originates from the MEMKO MBSEF-SEng coffee machine course work
by Alex Kovaceski (September 2026) and is shared with the permission of
the author. See NOTICE.md.
```

- [ ] **Step 5: Write sample/corpus/coffee-machine/README.md**

```markdown
# Coffee machine corpus

Model: bean-to-cup coffee machine, SysML v1, built in CATIA Magic
(Magic Systems of Systems Architect 2026x) during the MEMKO MBSEF-SEng
course.

Known-good numbers (pinned by tests and by the evidence annex):

- graph nodes: 99
- graph edges: 165
- requirements: 25
- blocks: 49
- signals: 9
- activities: 8
- connected components: 1
- isolated nodes: 0

Workflow: edit the legacy model in CATIA Magic, re-run the exporter, and
place the fresh export at okf/expected/coffee_machine_model.json. The gate
CI job proves the refresh loses nothing.
```

- [ ] **Step 6: Commit**

```powershell
git add sample
git commit -m "chore: port coffee-machine corpus into sample/"
```

---

### Task 4: engine/okf — types, validation, hashing, diffing

**Files:**
- Create: Cargo.toml (workspace root; members okf and test-support)
- Create: engine/test-support/Cargo.toml
- Create: engine/test-support/src/lib.rs
- Create: engine/okf/Cargo.toml
- Create: engine/okf/src/lib.rs
- Create: engine/okf/src/types.rs
- Create: engine/okf/src/validate.rs
- Create: engine/okf/src/hash.rs
- Create: engine/okf/src/diff.rs
- Test: engine/okf/tests/okf_validation.rs
- Test: engine/okf/tests/okf_diff.rs

**Interfaces:**
- Consumes: the corpus fixture at sample/corpus/coffee-machine/okf/expected/coffee_machine_model.json (via test_support).
- Produces:
  - okf::types::OkfRoot and its item types (Element, Requirement, State, Region, StateMachine, Activity, ActivityNode, ActivityEdge, GraphNode, GraphEdge, Graph, Summary, Provenance, Attribute).
  - okf::validate::validate(root: &OkfRoot) -> ValidationReport with fields valid: bool, errors: Vec<String>, warnings: Vec<String>.
  - okf::hash::canonical_hash(root: &OkfRoot) -> String (sha256 hex).
  - okf::diff::diff(reference: &OkfRoot, candidate: &OkfRoot) -> DiffReport with fields equal, missing_elements, extra_elements, missing_edges, extra_edges, changed_attributes (all Vec<String>).
  - test_support::load_okf_expected() -> String, test_support::load_okf_broken() -> String, test_support::okf_expected() -> PathBuf, test_support::okf_broken() -> PathBuf.

- [ ] **Step 1: Write the workspace root Cargo.toml**

```toml
[workspace]
resolver = "2"
members = ["engine/okf", "engine/test-support"]

[workspace.package]
edition = "2021"
rust-version = "1.75"
license = "AGPL-3.0-or-later"
repository = "https://github.com/modelwrite/modelwrite"

[workspace.dependencies]
anyhow = "1.0"
hex = "0.4"
petgraph = "0.6"
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
sha2 = "0.10"
```

- [ ] **Step 2: Write engine/test-support/Cargo.toml**

```toml
[package]
name = "mw-test-support"
version = "0.1.0"
edition.workspace = true
rust-version.workspace = true
license.workspace = true
publish = false

[lib]
name = "test_support"

[dependencies]
```

- [ ] **Step 3: Write engine/test-support/src/lib.rs**

```rust
// SPDX-License-Identifier: AGPL-3.0-or-later
use std::path::PathBuf;

/// The repository root, two levels above this crate (engine/test-support).
pub fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

pub fn okf_expected() -> PathBuf {
    repo_root().join("sample/corpus/coffee-machine/okf/expected/coffee_machine_model.json")
}

pub fn okf_broken() -> PathBuf {
    repo_root().join("sample/corpus/coffee-machine/okf/corrupted/coffee_machine_model.json")
}

pub fn load_okf_expected() -> String {
    std::fs::read_to_string(okf_expected()).expect("corpus fixture must exist")
}

pub fn load_okf_broken() -> String {
    std::fs::read_to_string(okf_broken()).expect("corrupted fixture must exist")
}
```

- [ ] **Step 4: Write engine/okf/Cargo.toml**

```toml
[package]
name = "mw-okf"
version = "0.1.0"
edition.workspace = true
rust-version.workspace = true
license.workspace = true
publish = false

[lib]
name = "okf"

[dependencies]
serde.workspace = true
serde_json.workspace = true
sha2.workspace = true
hex.workspace = true

[dev-dependencies]
mw-test-support = { path = "../test-support" }
```

- [ ] **Step 5: Write engine/okf/src/lib.rs**

```rust
// SPDX-License-Identifier: AGPL-3.0-or-later
pub mod diff;
pub mod hash;
pub mod types;
pub mod validate;
```

- [ ] **Step 6: Write engine/okf/src/types.rs**

```rust
// SPDX-License-Identifier: AGPL-3.0-or-later
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Attribute {
    pub name: String,
    #[serde(rename = "type", default)]
    pub attr_type: String,
    #[serde(default)]
    pub aggregation: String,
    #[serde(default)]
    pub default: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Element {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub stereotypes: Vec<String>,
    #[serde(default)]
    pub attributes: Vec<Attribute>,
    #[serde(default)]
    pub documentation: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Requirement {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub stereotypes: Vec<String>,
    #[serde(default)]
    pub attributes: Vec<Attribute>,
    #[serde(default)]
    pub documentation: String,
    #[serde(rename = "reqId", default)]
    pub req_id: String,
    #[serde(rename = "reqText", default)]
    pub req_text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct State {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub entry: Option<String>,
    #[serde(rename = "doActivity", default)]
    pub do_activity: Option<String>,
    #[serde(default)]
    pub exit: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Region {
    #[serde(default)]
    pub states: Vec<State>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StateMachine {
    pub name: String,
    #[serde(default)]
    pub regions: Vec<Region>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ActivityNode {
    pub id: String,
    #[serde(rename = "type", default)]
    pub node_type: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub partition: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ActivityEdge {
    #[serde(rename = "type", default)]
    pub edge_type: String,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub target: String,
    #[serde(default)]
    pub guard: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Activity {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub partitions: Vec<String>,
    #[serde(default)]
    pub nodes: Vec<ActivityNode>,
    #[serde(default)]
    pub edges: Vec<ActivityEdge>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GraphNode {
    pub id: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub stereotypes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GraphEdge {
    pub source: String,
    pub target: String,
    pub kind: String,
    #[serde(default)]
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Graph {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Summary {
    #[serde(default)]
    pub blocks: u64,
    #[serde(default)]
    pub requirements: u64,
    #[serde(default)]
    pub interfaces: u64,
    #[serde(default)]
    pub signals: u64,
    #[serde(default)]
    pub activities: u64,
    #[serde(rename = "graphNodes", default)]
    pub graph_nodes: u64,
    #[serde(rename = "graphEdges", default)]
    pub graph_edges: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Provenance {
    #[serde(rename = "sourceTool", default)]
    pub source_tool: String,
    #[serde(default)]
    pub exporter: String,
    #[serde(rename = "exporterVersion", default)]
    pub exporter_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OkfRoot {
    #[serde(default)]
    pub okf: String,
    pub project: String,
    #[serde(rename = "exportedAt", default)]
    pub exported_at: String,
    #[serde(default)]
    pub summary: Summary,
    #[serde(default)]
    pub structure: Vec<Element>,
    #[serde(default)]
    pub interfaces: Vec<Element>,
    #[serde(default)]
    pub signals: Vec<Element>,
    #[serde(default)]
    pub requirements: Vec<Requirement>,
    #[serde(default)]
    pub state_machine: Option<StateMachine>,
    #[serde(default)]
    pub activities: Vec<Activity>,
    #[serde(default)]
    pub graph: Option<Graph>,
    #[serde(default)]
    pub provenance: Option<Provenance>,
}
```

---
- [ ] **Step 7: Write engine/okf/src/validate.rs**

```rust
// SPDX-License-Identifier: AGPL-3.0-or-later
use std::collections::HashSet;

use serde::Serialize;

use crate::types::OkfRoot;

pub const NODE_KINDS: &[&str] = &[
    "activity", "actor", "block", "interface", "requirement", "signal", "state",
    "stateMachine", "usecase",
];

pub const EDGE_KINDS: &[&str] = &[
    "association", "behavior", "contains", "dependency", "effect", "generalization",
    "include", "part", "reference", "subject", "transition", "triggers", "uses",
];

#[derive(Debug, Clone, Serialize)]
pub struct ValidationReport {
    pub valid: bool,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

pub fn validate(root: &OkfRoot) -> ValidationReport {
    let mut report = ValidationReport {
        valid: true,
        errors: Vec::new(),
        warnings: Vec::new(),
    };

    if !root.okf.is_empty() && root.okf != "1.0" {
        report.errors.push(format!("okf marker must be empty or 1.0, found {}", root.okf));
    } else if root.okf.is_empty() {
        report.warnings.push("legacy OKF without version marker; exporters should emit okf 1.0".to_string());
    }
    if root.project.is_empty() {
        report.errors.push("project name is empty".to_string());
    }
    if root.graph.is_none() {
        report.errors.push("graph section is missing".to_string());
    }
    if root.state_machine.is_none() {
        report.errors.push("stateMachine section is missing".to_string());
    }

    let mut seen: HashSet<String> = HashSet::new();
    let mut add_id = |id: &str, section: &str, errors: &mut Vec<String>| {
        if id.is_empty() {
            errors.push(format!("{}: empty element id", section));
        } else if !seen.insert(id.to_string()) {
            errors.push(format!("duplicate element id {} in {}", id, section));
        }
    };
    for el in &root.structure {
        add_id(&el.id, "structure", &mut report.errors);
    }
    for el in &root.interfaces {
        add_id(&el.id, "interfaces", &mut report.errors);
    }
    for el in &root.signals {
        add_id(&el.id, "signals", &mut report.errors);
    }
    for r in &root.requirements {
        add_id(&r.id, "requirements", &mut report.errors);
        if r.req_id.trim().is_empty() {
            report.errors.push(format!("requirement {} has an empty reqId", r.id));
        }
    }
    if let Some(sm) = &root.state_machine {
        if sm.name.is_empty() {
            report.errors.push("stateMachine has an empty name".to_string());
        }
        for region in &sm.regions {
            for s in &region.states {
                add_id(&s.id, "stateMachine", &mut report.errors);
            }
        }
    }

    if let Some(graph) = &root.graph {
        if graph.nodes.is_empty() {
            report.errors.push("graph has no nodes".to_string());
        }
        let node_ids: HashSet<&str> = graph.nodes.iter().map(|n| n.id.as_str()).collect();
        for n in &graph.nodes {
            if !NODE_KINDS.contains(&n.kind.as_str()) {
                report.errors.push(format!("unknown node kind {} on node {}", n.kind, n.id));
            }
        }
        for e in &graph.edges {
            if !EDGE_KINDS.contains(&e.kind.as_str()) {
                report.errors.push(format!("unknown edge kind {} from {} to {}", e.kind, e.source, e.target));
            }
            if !node_ids.contains(e.source.as_str()) {
                report.errors.push(format!("dangling edge endpoint {} (source of a {} edge)", e.source, e.kind));
            }
            if !node_ids.contains(e.target.as_str()) {
                report.errors.push(format!("dangling edge endpoint {} (target of a {} edge)", e.target, e.kind));
            }
        }
    }

    let sm = &root.summary;
    if let Some(graph) = &root.graph {
        if sm.graph_nodes as usize != graph.nodes.len() || sm.graph_edges as usize != graph.edges.len() {
            report.warnings.push(format!("summary graph counts ({}/{}) do not match graph section ({}/{})", sm.graph_nodes, sm.graph_edges, graph.nodes.len(), graph.edges.len()));
        }
    }
    if sm.blocks as usize != root.structure.len() {
        report.warnings.push(format!("summary blocks {} do not match structure length {}", sm.blocks, root.structure.len()));
    }
    if sm.requirements as usize != root.requirements.len() {
        report.warnings.push(format!("summary requirements {} do not match requirements length {}", sm.requirements, root.requirements.len()));
    }
    if sm.signals as usize != root.signals.len() {
        report.warnings.push(format!("summary signals {} do not match signals length {}", sm.signals, root.signals.len()));
    }
    if sm.activities as usize != root.activities.len() {
        report.warnings.push(format!("summary activities {} do not match activities length {}", sm.activities, root.activities.len()));
    }
    if sm.interfaces as usize != root.interfaces.len() {
        report.warnings.push(format!("summary interfaces {} do not match interfaces length {}", sm.interfaces, root.interfaces.len()));
    }

    report.valid = report.errors.is_empty();
    report
}
```

- [ ] **Step 8: Write engine/okf/src/hash.rs**

```rust
// SPDX-License-Identifier: AGPL-3.0-or-later
use sha2::{Digest, Sha256};

use crate::types::OkfRoot;

/// Content fingerprint of an OKF snapshot: sha256 over the canonical
/// serialization. Field order is fixed by the type definitions, so a given
/// snapshot always hashes to the same value. It is a fingerprint, not a
/// semantic identity; the gate uses diffing for equality.
pub fn canonical_hash(root: &OkfRoot) -> String {
    let bytes = serde_json::to_vec(root).expect("OkfRoot serialization cannot fail");
    let digest = Sha256::digest(&bytes);
    hex::encode(digest)
}
```

- [ ] **Step 9: Write engine/okf/src/diff.rs**

```rust
// SPDX-License-Identifier: AGPL-3.0-or-later
use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::types::OkfRoot;

#[derive(Debug, Clone, Serialize)]
pub struct DiffReport {
    pub equal: bool,
    pub missing_elements: Vec<String>,
    pub extra_elements: Vec<String>,
    pub missing_edges: Vec<String>,
    pub extra_edges: Vec<String>,
    pub changed_attributes: Vec<String>,
}

/// Every element id across all sections, plus every graph node id.
pub fn element_ids(root: &OkfRoot) -> BTreeSet<String> {
    let mut ids = BTreeSet::new();
    for el in &root.structure {
        ids.insert(el.id.clone());
    }
    for el in &root.interfaces {
        ids.insert(el.id.clone());
    }
    for el in &root.signals {
        ids.insert(el.id.clone());
    }
    for r in &root.requirements {
        ids.insert(r.id.clone());
    }
    if let Some(sm) = &root.state_machine {
        for region in &sm.regions {
            for s in &region.states {
                ids.insert(s.id.clone());
            }
        }
    }
    if let Some(graph) = &root.graph {
        for n in &graph.nodes {
            ids.insert(n.id.clone());
        }
    }
    ids
}

/// Every relationship as a source|target|kind|label key.
pub fn edge_keys(root: &OkfRoot) -> BTreeSet<String> {
    let mut keys = BTreeSet::new();
    if let Some(graph) = &root.graph {
        for e in &graph.edges {
            keys.insert(format!("{}|{}|{}|{}", e.source, e.target, e.kind, e.label));
        }
    }
    keys
}

/// id -> canonical JSON of the section item, used for attribute equality.
pub fn attribute_keys(root: &OkfRoot) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    for el in &root.structure {
        if let Ok(v) = serde_json::to_string(el) {
            map.insert(el.id.clone(), v);
        }
    }
    for el in &root.interfaces {
        if let Ok(v) = serde_json::to_string(el) {
            map.insert(el.id.clone(), v);
        }
    }
    for el in &root.signals {
        if let Ok(v) = serde_json::to_string(el) {
            map.insert(el.id.clone(), v);
        }
    }
    for r in &root.requirements {
        if let Ok(v) = serde_json::to_string(r) {
            map.insert(r.id.clone(), v);
        }
    }
    if let Some(sm) = &root.state_machine {
        for region in &sm.regions {
            for s in &region.states {
                if let Ok(v) = serde_json::to_string(s) {
                    map.insert(s.id.clone(), v);
                }
            }
        }
    }
    if let Some(graph) = &root.graph {
        for n in &graph.nodes {
            if let Ok(v) = serde_json::to_string(n) {
                map.insert(n.id.clone(), v);
            }
        }
    }
    map
}

pub fn diff(reference: &OkfRoot, candidate: &OkfRoot) -> DiffReport {
    let ref_ids = element_ids(reference);
    let cand_ids = element_ids(candidate);
    let ref_edges = edge_keys(reference);
    let cand_edges = edge_keys(candidate);
    let ref_attrs = attribute_keys(reference);
    let cand_attrs = attribute_keys(candidate);

    let mut missing_elements: Vec<String> = ref_ids.difference(&cand_ids).cloned().collect();
    let mut extra_elements: Vec<String> = cand_ids.difference(&ref_ids).cloned().collect();
    let mut missing_edges: Vec<String> = ref_edges.difference(&cand_edges).cloned().collect();
    let mut extra_edges: Vec<String> = cand_edges.difference(&ref_edges).cloned().collect();
    let mut changed_attributes: Vec<String> = Vec::new();
    for (id, ref_value) in &ref_attrs {
        if let Some(cand_value) = cand_attrs.get(id) {
            if cand_value != ref_value {
                changed_attributes.push(id.clone());
            }
        }
    }
    missing_elements.sort();
    extra_elements.sort();
    missing_edges.sort();
    extra_edges.sort();
    changed_attributes.sort();

    let equal = missing_elements.is_empty()
        && extra_elements.is_empty()
        && missing_edges.is_empty()
        && extra_edges.is_empty()
        && changed_attributes.is_empty();
    DiffReport {
        equal,
        missing_elements,
        extra_elements,
        missing_edges,
        extra_edges,
        changed_attributes,
    }
}
```

- [ ] **Step 10: Write engine/okf/tests/okf_validation.rs**

```rust
// SPDX-License-Identifier: AGPL-3.0-or-later
use okf::{hash, types::OkfRoot, validate};

fn expected() -> OkfRoot {
    serde_json::from_str(&test_support::load_okf_expected()).expect("corpus fixture parses")
}

#[test]
fn corpus_fixture_validates() {
    let report = validate::validate(&expected());
    assert!(report.valid, "unexpected errors: {:?}", report.errors);
    assert_eq!(report.errors, Vec::<String>::new());
}

#[test]
fn corpus_fixture_has_expected_graph() {
    let root = expected();
    let graph = root.graph.as_ref().expect("graph present");
    assert_eq!(graph.nodes.len(), 99);
    assert_eq!(graph.edges.len(), 165);
    assert_eq!(root.requirements.len(), 25);
}

#[test]
fn rejects_duplicate_element_ids() {
    let mut root = expected();
    if root.structure.len() >= 2 {
        let dup = root.structure[1].id.clone();
        root.structure[0].id = dup;
    }
    let report = validate::validate(&root);
    assert!(!report.valid);
    assert!(report.errors.iter().any(|e| e.contains("duplicate element id")));
}

#[test]
fn rejects_dangling_edge_endpoint() {
    let mut root = expected();
    if let Some(graph) = root.graph.as_mut() {
        let target = graph.nodes[0].id.clone();
        graph.edges.push(okf::types::GraphEdge {
            source: "missing-node".into(),
            target,
            kind: "part".into(),
            label: String::new(),
        });
    }
    let report = validate::validate(&root);
    assert!(!report.valid);
    assert!(report.errors.iter().any(|e| e.contains("missing-node")));
}

#[test]
fn rejects_unknown_edge_kind() {
    let mut root = expected();
    if let Some(graph) = root.graph.as_mut() {
        let target = graph.nodes[0].id.clone();
        graph.edges.push(okf::types::GraphEdge {
            source: "missing-node".into(),
            target,
            kind: "not-a-kind".into(),
            label: String::new(),
        });
    }
    let report = validate::validate(&root);
    assert!(!report.valid);
    assert!(report.errors.iter().any(|e| e.contains("unknown edge kind")));
}

#[test]
fn rejects_empty_requirement_id() {
    let mut root = expected();
    root.requirements[0].req_id = String::new();
    let report = validate::validate(&root);
    assert!(!report.valid);
    assert!(report.errors.iter().any(|e| e.contains("empty reqId")));
}

#[test]
fn canonical_hash_is_stable() {
    let a = hash::canonical_hash(&expected());
    let b = hash::canonical_hash(&expected());
    assert_eq!(a, b);
    assert_eq!(a.len(), 64);
}
```

- [ ] **Step 11: Write engine/okf/tests/okf_diff.rs**

```rust
// SPDX-License-Identifier: AGPL-3.0-or-later
use okf::{diff, types::OkfRoot};

fn expected() -> OkfRoot {
    serde_json::from_str(&test_support::load_okf_expected()).expect("corpus fixture parses")
}

#[test]
fn identical_models_diff_equal() {
    let r = expected();
    assert!(diff::diff(&r, &r).equal);
}

#[test]
fn removed_requirement_is_missing_element() {
    let mut candidate = expected();
    candidate.requirements.pop();
    let d = diff::diff(&expected(), &candidate);
    assert!(!d.equal);
    assert_eq!(d.missing_elements.len(), 1);
}

#[test]
fn removed_edge_is_missing_edge() {
    let mut candidate = expected();
    candidate.graph.as_mut().expect("graph present").edges.pop();
    let d = diff::diff(&expected(), &candidate);
    assert!(!d.equal);
    assert_eq!(d.missing_edges.len(), 1);
}

#[test]
fn renamed_element_is_changed_attribute() {
    let mut candidate = expected();
    candidate.structure[0].name.push_str(" X");
    let d = diff::diff(&expected(), &candidate);
    assert!(!d.equal);
    assert_eq!(d.changed_attributes.len(), 1);
}
```

- [ ] **Step 12: Run the tests**

Run: cargo test -p mw-okf
Expected: all 13 tests pass. If corpus_fixture_validates fails, stop and report the validation errors verbatim; do not weaken the validator to make the corpus pass.

- [ ] **Step 13: Format and lint**

Run: cargo fmt --all
Run: cargo clippy -p mw-okf --all-targets -- -D warnings
Expected: no output from fmt; clippy exits 0 with no warnings.

- [ ] **Step 14: Commit**

```powershell
git add Cargo.toml engine
git commit -m "feat: add okf crate with types, validation, hashing and diffing"
```

---
### Task 5: engine/graph — graph analysis and coverage

**Files:**
- Create: engine/graph/Cargo.toml
- Create: engine/graph/src/lib.rs
- Test: engine/graph/tests/graph_stats.rs
- Modify: Cargo.toml (add engine/graph to workspace members)

**Interfaces:**
- Consumes: okf::types::OkfRoot; test_support::load_okf_expected().
- Produces:
  - graph::graph_stats(root: &OkfRoot) -> GraphStats with fields node_count: usize, edge_count: usize, isolated: Vec<String>, component_count: usize, component_sizes: Vec<usize> (descending).
  - graph::requirement_coverage(root: &OkfRoot) -> CoverageReport with fields total, satisfied, refined, verified, allocated, covered (usize) and uncovered: Vec<String> (sorted).

- [ ] **Step 1: Write engine/graph/Cargo.toml**

```toml
[package]
name = "mw-graph"
version = "0.1.0"
edition.workspace = true
rust-version.workspace = true
license.workspace = true
publish = false

[lib]
name = "graph"

[dependencies]
mw-okf = { path = "../okf" }
petgraph.workspace = true
serde.workspace = true

[dev-dependencies]
mw-test-support = { path = "../test-support" }
serde_json.workspace = true
```

- [ ] **Step 2: Add engine/graph to the workspace members in the root Cargo.toml**

Change the members line to:

```toml
members = ["engine/okf", "engine/test-support", "engine/graph"]
```

- [ ] **Step 3: Write engine/graph/src/lib.rs**

```rust
// SPDX-License-Identifier: AGPL-3.0-or-later
use std::collections::{HashMap, HashSet};

use okf::types::OkfRoot;
use petgraph::unionfind::UnionFind;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct GraphStats {
    pub node_count: usize,
    pub edge_count: usize,
    pub isolated: Vec<String>,
    pub component_count: usize,
    pub component_sizes: Vec<usize>,
}

pub fn graph_stats(root: &OkfRoot) -> GraphStats {
    let graph = root.graph.as_ref().expect("graph required; validate first");
    let node_count = graph.nodes.len();
    let edge_count = graph.edges.len();
    let index: HashMap<&str, usize> = graph
        .nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (n.id.as_str(), i))
        .collect();
    let mut uf = UnionFind::new(node_count);
    let mut degree = vec![0usize; node_count];
    for e in &graph.edges {
        if let (Some(&a), Some(&b)) = (index.get(e.source.as_str()), index.get(e.target.as_str())) {
            degree[a] += 1;
            degree[b] += 1;
            uf.union(a, b);
        }
    }
    let isolated: Vec<String> = graph
        .nodes
        .iter()
        .enumerate()
        .filter(|(i, _)| degree[*i] == 0)
        .map(|(_, n)| n.id.clone())
        .collect();
    let mut sizes: HashMap<usize, usize> = HashMap::new();
    for i in 0..node_count {
        *sizes.entry(uf.find(i)).or_insert(0) += 1;
    }
    let mut component_sizes: Vec<usize> = sizes.into_values().collect();
    component_sizes.sort_unstable_by(|a, b| b.cmp(a));
    let component_count = component_sizes.len();
    GraphStats {
        node_count,
        edge_count,
        isolated,
        component_count,
        component_sizes,
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct CoverageReport {
    pub total: usize,
    pub satisfied: usize,
    pub refined: usize,
    pub verified: usize,
    pub allocated: usize,
    pub covered: usize,
    pub uncovered: Vec<String>,
}

/// Requirement coverage read from dependency edges whose label is exactly
/// Satisfy, Refine, Verify or Allocate.
pub fn requirement_coverage(root: &OkfRoot) -> CoverageReport {
    let req_ids: HashSet<&str> = root.requirements.iter().map(|r| r.id.as_str()).collect();
    let graph = root.graph.as_ref().expect("graph required; validate first");
    let mut satisfied = 0usize;
    let mut refined = 0usize;
    let mut verified = 0usize;
    let mut allocated = 0usize;
    let mut covered: HashSet<&str> = HashSet::new();
    for e in &graph.edges {
        if e.kind != "dependency" {
            continue;
        }
        let s = e.source.as_str();
        let t = e.target.as_str();
        match e.label.as_str() {
            "Satisfy" => {
                if req_ids.contains(t) {
                    satisfied += 1;
                    covered.insert(t);
                }
            }
            "Refine" => {
                if req_ids.contains(t) {
                    refined += 1;
                    covered.insert(t);
                }
            }
            "Verify" => {
                if req_ids.contains(t) {
                    verified += 1;
                    covered.insert(t);
                }
            }
            "Allocate" => {
                if req_ids.contains(t) || req_ids.contains(s) {
                    allocated += 1;
                    covered.insert(t);
                    covered.insert(s);
                }
            }
            _ => {}
        }
    }
    let mut uncovered: Vec<String> = root
        .requirements
        .iter()
        .filter(|r| !covered.contains(r.id.as_str()))
        .map(|r| r.id.clone())
        .collect();
    uncovered.sort();
    CoverageReport {
        total: req_ids.len(),
        satisfied,
        refined,
        verified,
        allocated,
        covered: covered.len(),
        uncovered,
    }
}
```

- [ ] **Step 4: Write engine/graph/tests/graph_stats.rs**

```rust
// SPDX-License-Identifier: AGPL-3.0-or-later
use graph::{graph_stats, requirement_coverage};
use okf::types::OkfRoot;

fn expected() -> OkfRoot {
    serde_json::from_str(&test_support::load_okf_expected()).expect("corpus fixture parses")
}

fn okf(text: &str) -> OkfRoot {
    serde_json::from_str(text).expect("test OKF parses")
}

const TINY: &str = r#"{
  "project": "tiny",
  "exportedAt": "2026-09-17T00:00:00Z",
  "summary": {},
  "stateMachine": {"name": "tiny sm", "regions": []},
  "requirements": [
    {"id": "r1", "name": "R1", "kind": "requirement", "stereotypes": ["Requirement"], "attributes": [], "documentation": "", "reqId": "1.1", "reqText": "covered"},
    {"id": "r2", "name": "R2", "kind": "requirement", "stereotypes": ["Requirement"], "attributes": [], "documentation": "", "reqId": "1.2", "reqText": "uncovered"}
  ],
  "graph": {
    "nodes": [
      {"id": "p1", "kind": "block", "name": "P1"},
      {"id": "r1", "kind": "requirement", "name": "R1"},
      {"id": "r2", "kind": "requirement", "name": "R2"}
    ],
    "edges": [
      {"source": "p1", "target": "r1", "kind": "dependency", "label": "Satisfy"}
    ]
  }
}"#;

#[test]
fn corpus_is_fully_integrated() {
    let stats = graph_stats(&expected());
    assert_eq!(stats.node_count, 99);
    assert_eq!(stats.edge_count, 165);
    assert!(stats.isolated.is_empty(), "isolated: {:?}", stats.isolated);
    assert_eq!(stats.component_count, 1);
    assert_eq!(stats.component_sizes, vec![99]);
}

#[test]
fn isolated_node_is_detected() {
    let stats = graph_stats(&okf(TINY));
    assert_eq!(stats.isolated, vec!["r2"]);
}

#[test]
fn two_components_are_counted() {
    let stats = graph_stats(&okf(TINY));
    assert_eq!(stats.component_count, 2);
    assert_eq!(stats.component_sizes, vec![2, 1]);
}

#[test]
fn coverage_counts_traceability() {
    let cov = requirement_coverage(&okf(TINY));
    assert_eq!(cov.total, 2);
    assert_eq!(cov.satisfied, 1);
    assert_eq!(cov.covered, 1);
    assert_eq!(cov.uncovered, vec!["r2"]);
}
```

- [ ] **Step 5: Run the tests**

Run: cargo test -p mw-graph
Expected: all 4 tests pass. If corpus_is_fully_integrated fails, stop and report the actual isolated nodes and component sizes; do not change the assertions to match.

- [ ] **Step 6: Format, lint and commit**

```powershell
cargo fmt --all
cargo clippy -p mw-graph --all-targets -- -D warnings
git add Cargo.toml engine/graph
git commit -m "feat: add graph crate with integration and coverage analysis"
```

---

### Task 6: engine/gate — the round-trip fidelity gate

**Files:**
- Create: engine/gate/Cargo.toml
- Create: engine/gate/src/lib.rs
- Create: engine/gate/src/main.rs (binary mw-gate)
- Create: sample/scripts/make_corrupted.py
- Create: sample/corpus/coffee-machine/okf/corrupted/coffee_machine_model.json (generated)
- Create: docs/evidence/README.md
- Create: docs/evidence/2026-09-17-roundtrip-self-pass.json (generated)
- Create: docs/evidence/2026-09-17-roundtrip-corrupted-fail.json (generated)
- Test: engine/gate/tests/gate.rs
- Modify: Cargo.toml (add engine/gate to workspace members)

**Interfaces:**
- Consumes: okf::{validate, diff, hash}; graph::{graph_stats, requirement_coverage}; test_support.
- Produces:
  - gate::run(reference: &OkfRoot, candidate: &OkfRoot, strict_coverage: bool) -> GateOutcome with fields passed: bool, failures: Vec<String>, evidence: serde_json::Value.
  - gate::write_evidence(path: &std::path::Path, evidence: &serde_json::Value) -> anyhow::Result<()>.
  - Binary mw-gate with flags: --reference FILE, --candidate FILE, --evidence FILE (optional), --json (print evidence JSON to stdout), --strict-coverage (optional). Exit code 0 on pass, 1 on gate failure, 2 on usage or I/O errors.

- [ ] **Step 1: Write engine/gate/Cargo.toml**

```toml
[package]
name = "mw-gate"
version = "0.1.0"
edition.workspace = true
rust-version.workspace = true
license.workspace = true
publish = false

[lib]
name = "gate"

[[bin]]
name = "mw-gate"
path = "src/main.rs"

[dependencies]
mw-okf = { path = "../okf" }
mw-graph = { path = "../graph" }
anyhow.workspace = true
serde_json.workspace = true

[dev-dependencies]
mw-test-support = { path = "../test-support" }
```

- [ ] **Step 2: Add engine/gate to the workspace members in the root Cargo.toml**

```toml
members = ["engine/okf", "engine/test-support", "engine/graph", "engine/gate"]
```

- [ ] **Step 3: Write engine/gate/src/lib.rs**

```rust
// SPDX-License-Identifier: AGPL-3.0-or-later
use graph::{graph_stats, requirement_coverage};
use okf::{diff, hash, types::OkfRoot, validate};
use serde_json::json;

pub struct GateOutcome {
    pub passed: bool,
    pub failures: Vec<String>,
    pub evidence: serde_json::Value,
}

/// The provable gate: round-trip fidelity plus graph health, with evidence
/// recorded for every run. The evidence is deterministic: given the same
/// inputs, the record reproduces exactly.
pub fn run(reference: &OkfRoot, candidate: &OkfRoot, strict_coverage: bool) -> GateOutcome {
    let mut failures: Vec<String> = Vec::new();

    let d = diff::diff(reference, candidate);
    if !d.equal {
        failures.push(format!(
            "roundtrip: {} missing elements, {} extra elements, {} missing edges, {} extra edges, {} changed attributes",
            d.missing_elements.len(),
            d.extra_elements.len(),
            d.missing_edges.len(),
            d.extra_edges.len(),
            d.changed_attributes.len()
        ));
    }

    let v = validate::validate(candidate);
    if !v.valid {
        for e in &v.errors {
            failures.push(format!("validation: {}", e));
        }
    }

    let stats = graph_stats(candidate);
    if !stats.isolated.is_empty() {
        failures.push(format!("integration: {} isolated nodes", stats.isolated.len()));
    }
    if stats.component_count != 1 {
        failures.push(format!("integration: {} connected components", stats.component_count));
    }

    let cov = requirement_coverage(candidate);
    if strict_coverage && !cov.uncovered.is_empty() {
        failures.push(format!("coverage: {} uncovered requirements", cov.uncovered.len()));
    }

    let evidence = json!({
        "gateVersion": env!("CARGO_PKG_VERSION"),
        "okfVersion": "1.0",
        "referenceHash": hash::canonical_hash(reference),
        "candidateHash": hash::canonical_hash(candidate),
        "roundtrip": {
            "equal": d.equal,
            "missingElements": d.missing_elements,
            "extraElements": d.extra_elements,
            "missingEdges": d.missing_edges,
            "extraEdges": d.extra_edges,
            "changedAttributes": d.changed_attributes
        },
        "integration": {
            "isolated": stats.isolated,
            "componentCount": stats.component_count,
            "componentSizes": stats.component_sizes
        },
        "coverage": {
            "total": cov.total,
            "satisfied": cov.satisfied,
            "refined": cov.refined,
            "verified": cov.verified,
            "allocated": cov.allocated,
            "covered": cov.covered,
            "uncovered": cov.uncovered
        },
        "strictCoverage": strict_coverage,
        "validationErrors": v.errors,
        "validationWarnings": v.warnings,
        "passed": failures.is_empty(),
        "failures": failures
    });

    GateOutcome {
        passed: failures.is_empty(),
        failures,
        evidence,
    }
}

pub fn write_evidence(path: &std::path::Path, evidence: &serde_json::Value) -> anyhow::Result<()> {
    let text = serde_json::to_string_pretty(evidence)?;
    std::fs::write(path, text)?;
    Ok(())
}
```

- [ ] **Step 4: Write engine/gate/src/main.rs**

```rust
// SPDX-License-Identifier: AGPL-3.0-or-later
use std::path::PathBuf;
use std::process::ExitCode;

use okf::types::OkfRoot;

fn main() -> ExitCode {
    let mut reference: Option<PathBuf> = None;
    let mut candidate: Option<PathBuf> = None;
    let mut evidence: Option<PathBuf> = None;
    let mut json_output = false;
    let mut strict_coverage = false;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--reference" => reference = args.next().map(PathBuf::from),
            "--candidate" => candidate = args.next().map(PathBuf::from),
            "--evidence" => evidence = args.next().map(PathBuf::from),
            "--json" => json_output = true,
            "--strict-coverage" => strict_coverage = true,
            _ => {
                eprintln!("unknown argument: {}", a);
                return ExitCode::from(2);
            }
        }
    }
    let (Some(reference), Some(candidate)) = (reference, candidate) else {
        eprintln!("usage: mw-gate --reference FILE --candidate FILE [--evidence FILE] [--json] [--strict-coverage]");
        return ExitCode::from(2);
    };
    let read = |p: &PathBuf| -> anyhow::Result<OkfRoot> {
        let text = std::fs::read_to_string(p)?;
        Ok(serde_json::from_str(&text)?)
    };
    let (reference, candidate) = match (read(&reference), read(&candidate)) {
        (Ok(r), Ok(c)) => (r, c),
        (Err(e), _) | (_, Err(e)) => {
            eprintln!("failed to read input: {:#}", e);
            return ExitCode::from(2);
        }
    };
    let outcome = gate::run(&reference, &candidate, strict_coverage);
    if let Some(path) = &evidence {
        if let Err(e) = gate::write_evidence(path, &outcome.evidence) {
            eprintln!("failed to write evidence: {:#}", e);
            return ExitCode::from(2);
        }
    }
    if json_output {
        println!("{}", serde_json::to_string(&outcome.evidence).expect("evidence serializes"));
    } else if outcome.passed {
        println!("GATE PASS");
    } else {
        println!("GATE FAIL");
        for f in &outcome.failures {
            println!("- {}", f);
        }
    }
    if outcome.passed {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}
```

---
- [ ] **Step 5: Write sample/scripts/make_corrupted.py**

```python
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Produce the corrupted OKF fixture the gate must reject.

Deterministic breakage, applied to the reference corpus:
- every edge touching requirement 3.1.5 is removed, isolating that node;
- requirement 1.1 is deleted entirely (a lost element).
"""
import json
import sys
from pathlib import Path


def main() -> int:
    src = Path("sample/corpus/coffee-machine/okf/expected/coffee_machine_model.json")
    dst = Path("sample/corpus/coffee-machine/okf/corrupted/coffee_machine_model.json")
    data = json.loads(src.read_text(encoding="utf-8"))
    target = next(r["id"] for r in data["requirements"] if r["reqId"] == "3.1.5")
    before = len(data["graph"]["edges"])
    data["graph"]["edges"] = [
        e for e in data["graph"]["edges"]
        if e["source"] != target and e["target"] != target
    ]
    removed = before - len(data["graph"]["edges"])
    data["requirements"] = [r for r in data["requirements"] if r["reqId"] != "1.1"]
    dst.parent.mkdir(parents=True, exist_ok=True)
    dst.write_text(json.dumps(data, indent=2), encoding="utf-8")
    print(f"wrote {dst}: removed {removed} edges around REQ 3.1.5, deleted REQ 1.1")
    return 0


if __name__ == "__main__":
    sys.exit(main())
```

- [ ] **Step 6: Generate the corrupted fixture**

```powershell
python sample/scripts/make_corrupted.py
```

Expected: the script prints how many edges it removed around REQ 3.1.5 (a positive number) and that it deleted REQ 1.1. The corrupted file uses the same filename in a different directory so the judge harness can pair it with the reference.

- [ ] **Step 7: Write engine/gate/tests/gate.rs**

```rust
// SPDX-License-Identifier: AGPL-3.0-or-later
use okf::types::OkfRoot;

fn expected() -> OkfRoot {
    serde_json::from_str(&test_support::load_okf_expected()).expect("corpus fixture parses")
}

fn okf(text: &str) -> OkfRoot {
    serde_json::from_str(text).expect("test OKF parses")
}

const TINY_GATE: &str = r#"{
  "project": "tiny",
  "exportedAt": "2026-09-17T00:00:00Z",
  "summary": {},
  "stateMachine": {"name": "tiny sm", "regions": []},
  "requirements": [
    {"id": "r1", "name": "R1", "kind": "requirement", "stereotypes": ["Requirement"], "attributes": [], "documentation": "", "reqId": "1.1", "reqText": "covered"},
    {"id": "r2", "name": "R2", "kind": "requirement", "stereotypes": ["Requirement"], "attributes": [], "documentation": "", "reqId": "1.2", "reqText": "uncovered"}
  ],
  "graph": {
    "nodes": [
      {"id": "p1", "kind": "block", "name": "P1"},
      {"id": "r1", "kind": "requirement", "name": "R1"},
      {"id": "r2", "kind": "requirement", "name": "R2"}
    ],
    "edges": [
      {"source": "p1", "target": "r1", "kind": "dependency", "label": "Satisfy"},
      {"source": "p1", "target": "r2", "kind": "reference", "label": ""}
    ]
  }
}"#;

#[test]
fn self_roundtrip_passes() {
    let r = expected();
    let outcome = gate::run(&r, &r, false);
    assert!(outcome.passed, "unexpected failures: {:?}", outcome.failures);
    assert_eq!(outcome.evidence["passed"], true);
    assert_eq!(outcome.evidence["referenceHash"], outcome.evidence["candidateHash"]);
}

#[test]
fn corrupted_corpus_fails() {
    let reference = expected();
    let candidate: OkfRoot =
        serde_json::from_str(&test_support::load_okf_broken()).expect("broken fixture parses");
    let outcome = gate::run(&reference, &candidate, false);
    assert!(!outcome.passed);
    assert!(outcome.failures.iter().any(|f| f.contains("missing elements")));
    assert!(outcome.failures.iter().any(|f| f.contains("isolated")));
}

#[test]
fn strict_coverage_fails_on_uncovered() {
    let r = okf(TINY_GATE);
    let lenient = gate::run(&r, &r, false);
    assert!(lenient.passed, "unexpected failures: {:?}", lenient.failures);
    let strict = gate::run(&r, &r, true);
    assert!(!strict.passed);
    assert!(strict.failures.iter().any(|f| f.contains("uncovered requirements")));
}
```

- [ ] **Step 8: Run the tests**

Run: cargo test -p mw-gate
Expected: all 3 tests pass.

- [ ] **Step 9: Record the first two evidence files**

```powershell
cargo run -p mw-gate -- --reference sample/corpus/coffee-machine/okf/expected/coffee_machine_model.json --candidate sample/corpus/coffee-machine/okf/expected/coffee_machine_model.json --evidence docs/evidence/2026-09-17-roundtrip-self-pass.json
cargo run -p mw-gate -- --reference sample/corpus/coffee-machine/okf/expected/coffee_machine_model.json --candidate sample/corpus/coffee-machine/okf/corrupted/coffee_machine_model.json --evidence docs/evidence/2026-09-17-roundtrip-corrupted-fail.json
```

Expected: the first command prints GATE PASS and exits 0; the second prints GATE FAIL with the roundtrip and integration failure lines and exits 1 (the non-zero exit is correct and expected here). Both evidence files exist; the first contains "passed": true, the second "passed": false.

- [ ] **Step 10: Write docs/evidence/README.md**

```markdown
# Evidence annex

Every public claim about modelwrite traces to a recorded gate run in this
directory. Records are append-only: never rewrite an existing record.

Naming: YYYY-MM-DD-<subject>.json

Each record is the deterministic output of mw-gate --evidence: given the
same reference and candidate, the record reproduces exactly (hashes and
all). Dates appear only in filenames, never inside records.

Index:

- 2026-09-17-roundtrip-self-pass.json — the corpus compared with itself:
  the gate passes. Proves the reference OKF is a fixed point of the gate.
- 2026-09-17-roundtrip-corrupted-fail.json — the corpus compared with the
  corrupted fixture: the gate fails on missing elements and isolated
  nodes. Proves the gate detects loss and fragmentation.
```

- [ ] **Step 11: Format, lint and commit**

```powershell
cargo fmt --all
cargo clippy -p mw-gate --all-targets -- -D warnings
git add Cargo.toml engine/gate sample docs/evidence
git commit -m "feat: add round-trip fidelity gate with evidence records"
```

---
### Task 7: engine/capi — the C ABI

**Files:**
- Create: engine/capi/Cargo.toml
- Create: engine/capi/src/lib.rs
- Create: engine/include/modelwrite.h
- Test: engine/capi/tests/capi.rs
- Modify: Cargo.toml (add engine/capi to workspace members)

**Interfaces:**
- Consumes: okf::{types, validate}; gate::run.
- Produces: four exported C symbols, stable from now on:
  - const char *modelwrite_version(void)
  - char *modelwrite_validate(const unsigned char *json, size_t len) — returns a JSON ValidationReport string.
  - char *modelwrite_gate(const unsigned char *reference, size_t reference_len, const unsigned char *candidate, size_t candidate_len) — returns the JSON gate evidence string.
  - void modelwrite_free_string(char *ptr) — frees any string returned above.
  All returned strings are owned by the caller and must be released with modelwrite_free_string.

- [ ] **Step 1: Write engine/capi/Cargo.toml**

```toml
[package]
name = "mw-capi"
version = "0.1.0"
edition.workspace = true
rust-version.workspace = true
license.workspace = true
publish = false

[lib]
name = "capi"
crate-type = ["cdylib", "staticlib", "rlib"]

[dependencies]
mw-okf = { path = "../okf" }
mw-gate = { path = "../gate" }
serde_json.workspace = true

[dev-dependencies]
mw-test-support = { path = "../test-support" }
serde_json.workspace = true
```

- [ ] **Step 2: Add engine/capi to the workspace members in the root Cargo.toml**

```toml
members = ["engine/okf", "engine/test-support", "engine/graph", "engine/gate", "engine/capi"]
```

- [ ] **Step 3: Write engine/capi/src/lib.rs**

```rust
// SPDX-License-Identifier: AGPL-3.0-or-later
//! C ABI for the modelwrite engine. All returned strings are owned by the
//! caller and must be released with modelwrite_free_string.
use std::ffi::{c_char, CStr, CString};

fn err_string(message: &str) -> *mut c_char {
    let payload = serde_json::json!({ "error": message }).to_string();
    CString::new(payload).expect("message has no NUL").into_raw()
}

/// Version string; deliberately leaked, never freed.
#[no_mangle]
pub extern "C" fn modelwrite_version() -> *const c_char {
    let s = CString::new(format!("modelwrite {}", env!("CARGO_PKG_VERSION"))).expect("version has no NUL");
    s.into_raw() as *const c_char
}

/// Validate one OKF document; returns a JSON ValidationReport string.
#[no_mangle]
pub extern "C" fn modelwrite_validate(json: *const u8, len: usize) -> *mut c_char {
    if json.is_null() || len == 0 {
        return err_string("empty input");
    }
    let bytes = unsafe { std::slice::from_raw_parts(json, len) };
    let text = match std::str::from_utf8(bytes) {
        Ok(t) => t,
        Err(_) => return err_string("input is not UTF-8"),
    };
    let root = match serde_json::from_str::<okf::types::OkfRoot>(text) {
        Ok(r) => r,
        Err(e) => return err_string(&format!("parse error: {}", e)),
    };
    let report = okf::validate::validate(&root);
    let payload = serde_json::to_string(&report).expect("report serializes");
    CString::new(payload).expect("payload has no NUL").into_raw()
}

/// Run the round-trip gate; returns the JSON evidence string.
#[no_mangle]
pub extern "C" fn modelwrite_gate(
    reference: *const u8,
    reference_len: usize,
    candidate: *const u8,
    candidate_len: usize,
) -> *mut c_char {
    if reference.is_null() || reference_len == 0 || candidate.is_null() || candidate_len == 0 {
        return err_string("empty input");
    }
    let parse = |ptr: *const u8, len: usize| -> Option<okf::types::OkfRoot> {
        let bytes = unsafe { std::slice::from_raw_parts(ptr, len) };
        let text = std::str::from_utf8(bytes).ok()?;
        serde_json::from_str(text).ok()
    };
    let (Some(reference), Some(candidate)) = (parse(reference, reference_len), parse(candidate, candidate_len)) else {
        return err_string("parse error: inputs must be OKF JSON documents");
    };
    let outcome = gate::run(&reference, &candidate, false);
    let payload = serde_json::to_string(&outcome.evidence).expect("evidence serializes");
    CString::new(payload).expect("payload has no NUL").into_raw()
}

/// Free a string returned by this library.
#[no_mangle]
pub extern "C" fn modelwrite_free_string(ptr: *mut c_char) {
    if !ptr.is_null() {
        drop(unsafe { CString::from_raw(ptr) });
    }
}
```

- [ ] **Step 4: Write engine/include/modelwrite.h**

```c
/* SPDX-License-Identifier: AGPL-3.0-or-later */
#ifndef MODELWRITE_H
#define MODELWRITE_H

#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Version string; never free it. */
const char *modelwrite_version(void);

/* Validate one OKF JSON document; returns a JSON report string.
   The caller owns the result and must free it with modelwrite_free_string. */
char *modelwrite_validate(const unsigned char *json, size_t len);

/* Run the round-trip fidelity gate; returns the JSON evidence string.
   The caller owns the result and must free it with modelwrite_free_string. */
char *modelwrite_gate(const unsigned char *reference, size_t reference_len,
                      const unsigned char *candidate, size_t candidate_len);

/* Free a string returned by this library. */
void modelwrite_free_string(char *ptr);

#ifdef __cplusplus
}
#endif

#endif
```

- [ ] **Step 5: Write engine/capi/tests/capi.rs**

```rust
// SPDX-License-Identifier: AGPL-3.0-or-later
use std::ffi::CStr;

#[test]
fn version_returns_string() {
    let ptr = capi::modelwrite_version();
    let s = unsafe { CStr::from_ptr(ptr) }.to_string_lossy().into_owned();
    assert!(s.starts_with("modelwrite "));
}

#[test]
fn validate_returns_json_report() {
    let okf = test_support::load_okf_expected();
    let ptr = capi::modelwrite_validate(okf.as_ptr(), okf.len());
    let s = unsafe { CStr::from_ptr(ptr) }.to_string_lossy().into_owned();
    let v: serde_json::Value = serde_json::from_str(&s).expect("JSON report");
    assert!(v["valid"].as_bool().unwrap());
    capi::modelwrite_free_string(ptr);
}

#[test]
fn gate_returns_evidence() {
    let a = test_support::load_okf_expected();
    let b = a.clone();
    let ptr = capi::modelwrite_gate(a.as_ptr(), a.len(), b.as_ptr(), b.len());
    let s = unsafe { CStr::from_ptr(ptr) }.to_string_lossy().into_owned();
    let v: serde_json::Value = serde_json::from_str(&s).expect("JSON evidence");
    assert!(v["passed"].as_bool().unwrap());
    capi::modelwrite_free_string(ptr);
}

#[test]
fn validate_rejects_garbage() {
    let garbage = "not json";
    let ptr = capi::modelwrite_validate(garbage.as_ptr(), garbage.len());
    let s = unsafe { CStr::from_ptr(ptr) }.to_string_lossy().into_owned();
    let v: serde_json::Value = serde_json::from_str(&s).expect("JSON error report");
    assert!(v["error"].is_string());
    capi::modelwrite_free_string(ptr);
}
```

- [ ] **Step 6: Run the tests**

Run: cargo test -p mw-capi
Expected: all 4 tests pass.

- [ ] **Step 7: Format, lint and commit**

```powershell
cargo fmt --all
cargo clippy -p mw-capi --all-targets -- -D warnings
git add Cargo.toml engine/capi engine/include
git commit -m "feat: add C ABI for validation and the gate"
```

---

### Task 8: engine/mcp — the MCP server

**Files:**
- Create: engine/mcp/Cargo.toml
- Create: engine/mcp/src/lib.rs
- Create: engine/mcp/src/main.rs (binary mw-mcp)
- Test: engine/mcp/tests/mcp.rs
- Modify: Cargo.toml (add engine/mcp to workspace members)

**Interfaces:**
- Consumes: okf::{types, validate}; graph::graph_stats; gate::run.
- Produces:
  - mcp::handle_request(line: &str) -> String — one JSON-RPC 2.0 request (a single line) in, one JSON-RPC response line out; an empty String for notifications.
  - Binary mw-mcp: reads JSON-RPC lines from stdin, writes responses to stdout. Tools: okf.validate, graph.stats, gate.run.

- [ ] **Step 1: Write engine/mcp/Cargo.toml**

```toml
[package]
name = "mw-mcp"
version = "0.1.0"
edition.workspace = true
rust-version.workspace = true
license.workspace = true
publish = false

[lib]
name = "mcp"

[[bin]]
name = "mw-mcp"
path = "src/main.rs"

[dependencies]
mw-okf = { path = "../okf" }
mw-graph = { path = "../graph" }
mw-gate = { path = "../gate" }
serde_json.workspace = true

[dev-dependencies]
mw-test-support = { path = "../test-support" }
serde_json.workspace = true
```

- [ ] **Step 2: Add engine/mcp to the workspace members in the root Cargo.toml**

```toml
members = ["engine/okf", "engine/test-support", "engine/graph", "engine/gate", "engine/capi", "engine/mcp"]
```

- [ ] **Step 3: Write engine/mcp/src/lib.rs**

```rust
// SPDX-License-Identifier: AGPL-3.0-or-later
use serde_json::{json, Value};

fn tool_error(message: &str) -> Value {
    json!({ "content": [{ "type": "text", "text": message }], "isError": true })
}

fn tool_ok(text: String) -> Value {
    json!({ "content": [{ "type": "text", "text": text }], "isError": false })
}

fn call_tool(msg: &Value) -> Value {
    let params = msg.get("params");
    let name = params.and_then(|p| p.get("name")).and_then(Value::as_str).unwrap_or("");
    let args = params.and_then(|p| p.get("arguments")).cloned().unwrap_or_else(|| json!({}));
    match name {
        "okf.validate" => {
            let Some(okf) = args.get("okf").and_then(Value::as_str) else {
                return tool_error("missing okf argument");
            };
            match serde_json::from_str::<okf::types::OkfRoot>(okf) {
                Err(e) => tool_error(&format!("parse error: {}", e)),
                Ok(root) => {
                    let report = okf::validate::validate(&root);
                    tool_ok(serde_json::to_string_pretty(&report).expect("report serializes"))
                }
            }
        }
        "graph.stats" => {
            let Some(okf) = args.get("okf").and_then(Value::as_str) else {
                return tool_error("missing okf argument");
            };
            match serde_json::from_str::<okf::types::OkfRoot>(okf) {
                Err(e) => tool_error(&format!("parse error: {}", e)),
                Ok(root) => {
                    let stats = graph::graph_stats(&root);
                    tool_ok(serde_json::to_string_pretty(&stats).expect("stats serialize"))
                }
            }
        }
        "gate.run" => {
            let reference = args.get("reference").and_then(Value::as_str);
            let candidate = args.get("candidate").and_then(Value::as_str);
            let strict = args.get("strictCoverage").and_then(Value::as_bool).unwrap_or(false);
            let (Some(reference), Some(candidate)) = (reference, candidate) else {
                return tool_error("missing reference or candidate argument");
            };
            let parse = |s: &str| serde_json::from_str::<okf::types::OkfRoot>(s);
            match (parse(reference), parse(candidate)) {
                (Err(e), _) | (_, Err(e)) => tool_error(&format!("parse error: {}", e)),
                (Ok(reference), Ok(candidate)) => {
                    let outcome = gate::run(&reference, &candidate, strict);
                    tool_ok(serde_json::to_string_pretty(&outcome.evidence).expect("evidence serializes"))
                }
            }
        }
        _ => tool_error(&format!("unknown tool: {}", name)),
    }
}

pub fn handle_request(line: &str) -> String {
    let msg: Value = match serde_json::from_str(line) {
        Ok(m) => m,
        Err(e) => {
            return json!({
                "jsonrpc": "2.0",
                "id": null,
                "error": { "code": -32700, "message": format!("parse error: {}", e) }
            })
            .to_string();
        }
    };
    let id = msg.get("id").cloned().unwrap_or(Value::Null);
    let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
    if method.is_empty() {
        return json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": { "code": -32600, "message": "method missing" }
        })
        .to_string();
    }
    if id.is_null() {
        return String::new();
    }
    let result = match method {
        "initialize" => json!({
            "protocolVersion": "2024-11-05",
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "modelwrite-mcp", "version": env!("CARGO_PKG_VERSION") }
        }),
        "tools/list" => json!({ "tools": [
            {
                "name": "okf.validate",
                "description": "Validate an OKF JSON document",
                "inputSchema": {
                    "type": "object",
                    "properties": { "okf": { "type": "string" } },
                    "required": ["okf"]
                }
            },
            {
                "name": "graph.stats",
                "description": "Graph health of an OKF document",
                "inputSchema": {
                    "type": "object",
                    "properties": { "okf": { "type": "string" } },
                    "required": ["okf"]
                }
            },
            {
                "name": "gate.run",
                "description": "Round-trip fidelity gate between two OKF documents",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "reference": { "type": "string" },
                        "candidate": { "type": "string" },
                        "strictCoverage": { "type": "boolean" }
                    },
                    "required": ["reference", "candidate"]
                }
            }
        ] }),
        "tools/call" => call_tool(&msg),
        _ => {
            return json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32601, "message": format!("method not found: {}", method) }
            })
            .to_string();
        }
    };
    json!({ "jsonrpc": "2.0", "id": id, "result": result }).to_string()
}
```

- [ ] **Step 4: Write engine/mcp/src/main.rs**

```rust
// SPDX-License-Identifier: AGPL-3.0-or-later
use std::io::{BufRead, Write};

fn main() {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let response = mcp::handle_request(&line);
        if response.is_empty() {
            continue;
        }
        if writeln!(out, "{}", response).is_err() {
            break;
        }
        let _ = out.flush();
    }
}
```

- [ ] **Step 5: Write engine/mcp/tests/mcp.rs**

```rust
// SPDX-License-Identifier: AGPL-3.0-or-later
use serde_json::{json, Value};

fn call(payload: Value) -> Value {
    let out = mcp::handle_request(&payload.to_string());
    serde_json::from_str(&out).expect("valid JSON-RPC response")
}

#[test]
fn initialize_returns_server_info() {
    let resp = call(json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {} }));
    assert_eq!(resp["id"], 1);
    assert_eq!(resp["result"]["serverInfo"]["name"], "modelwrite-mcp");
    assert_eq!(resp["result"]["protocolVersion"], "2024-11-05");
}

#[test]
fn tools_list_has_three_tools() {
    let resp = call(json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {} }));
    let names: Vec<&str> = resp["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, vec!["okf.validate", "graph.stats", "gate.run"]);
}

#[test]
fn validate_tool_reports_invalid() {
    let resp = call(json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "tools/call",
        "params": { "name": "okf.validate", "arguments": { "okf": "{\"project\":\"x\"}" } }
    }));
    let text = resp["result"]["content"][0]["text"].as_str().unwrap();
    let report: Value = serde_json::from_str(text).unwrap();
    assert_eq!(report["valid"], false);
}

#[test]
fn gate_tool_runs_roundtrip() {
    let a = test_support::load_okf_expected();
    let resp = call(json!({
        "jsonrpc": "2.0",
        "id": 4,
        "method": "tools/call",
        "params": { "name": "gate.run", "arguments": { "reference": a, "candidate": a } }
    }));
    let text = resp["result"]["content"][0]["text"].as_str().unwrap();
    let evidence: Value = serde_json::from_str(text).unwrap();
    assert!(evidence["passed"].as_bool().unwrap());
}

#[test]
fn unknown_method_returns_error() {
    let resp = call(json!({ "jsonrpc": "2.0", "id": 5, "method": "nope", "params": {} }));
    assert_eq!(resp["error"]["code"], -32601);
}
```

- [ ] **Step 6: Run the tests**

Run: cargo test -p mw-mcp
Expected: all 5 tests pass.

- [ ] **Step 7: Smoke test the binary over stdin**

```powershell
"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/list\",\"params\":{}}" | cargo run -p mw-mcp
```

Expected: one JSON line listing okf.validate, graph.stats and gate.run. If piping into the binary is blocked in the current environment, skip this step (the unit tests already cover handle_request) and note it in the commit message.

- [ ] **Step 8: Format, lint and commit**

```powershell
cargo fmt --all
cargo clippy -p mw-mcp --all-targets -- -D warnings
git add Cargo.toml engine/mcp
git commit -m "feat: add MCP server exposing validation, graph stats and the gate"
```

---
### Task 9: CI workflow

**Files:**
- Create: .github/workflows/ci.yml

**Interfaces:**
- Consumes: the workspace build, the corpus fixture, and (once Task 10 lands) the judge unit tests.
- Produces: a green CI pipeline on every push and pull request. Note: CI first runs when the repository is pushed to GitHub; locally it is verified by inspection only.

- [ ] **Step 1: Write .github/workflows/ci.yml**

```yaml
name: ci
on:
  push:
  pull_request:

jobs:
  engine:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: clippy, rustfmt
      - run: cargo fmt --all -- --check
      - run: cargo clippy --workspace --all-targets -- -D warnings
      - run: cargo test --workspace
      - run: cargo run -p mw-gate -- --reference sample/corpus/coffee-machine/okf/expected/coffee_machine_model.json --candidate sample/corpus/coffee-machine/okf/expected/coffee_machine_model.json
      - run: python -m unittest discover -s judge/tests -p "test_*.py" -v
```

- [ ] **Step 2: Verify the file is present and commit**

```powershell
Get-Content .github/workflows/ci.yml | Select-Object -First 5
git add .github
git commit -m "ci: add engine test and gate pipeline"
```

Expected: the first five lines match the file above. Note for the executor: the judge test step will fail until Task 10 creates judge/tests; complete Task 10 before pushing to GitHub.

---

### Task 10: judge/ — the gate-as-judge harness

**Files:**
- Create: judge/judge.py
- Test: judge/tests/test_judge.py

**Interfaces:**
- Consumes: the mw-gate binary (built by cargo build --workspace).
- Produces: python judge/judge.py --ref-dir DIR --candidate-dir DIR [--gate-bin PATH] — pairs OKF files with matching filenames across the two directories, runs the gate on each pair with --json, prints one PASS/FAIL line per pair, exits 0 when all pass and 1 when any fails, 2 when no pairs are found.

- [ ] **Step 1: Write judge/judge.py**

```python
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Gate-as-judge harness: run mw-gate over pairs of OKF files in two directories.

Standalone scoring for model migrations, or the scoring step in an agent
loop. Usage:

    python judge/judge.py --ref-dir DIR --candidate-dir DIR [--gate-bin PATH]
"""
import argparse
import json
import subprocess
import sys
from pathlib import Path


def find_pairs(ref_dir: Path, cand_dir: Path) -> dict[str, tuple[Path, Path]]:
    pairs: dict[str, tuple[Path, Path]] = {}
    for ref in sorted(ref_dir.glob("*.json")):
        cand = cand_dir / ref.name
        if cand.is_file():
            pairs[ref.stem] = (ref, cand)
    return pairs


def run_gate(gate_bin: Path, reference: Path, candidate: Path) -> dict:
    proc = subprocess.run(
        [str(gate_bin), "--reference", str(reference), "--candidate", str(candidate), "--json"],
        capture_output=True,
        text=True,
    )
    try:
        payload = json.loads(proc.stdout.strip())
        return payload
    except json.JSONDecodeError:
        return {
            "passed": False,
            "failures": [],
            "parseError": True,
            "raw": proc.stdout.strip(),
            "exitCode": proc.returncode,
            "stderr": proc.stderr.strip(),
        }


def render_table(results: list[tuple[str, dict]]) -> str:
    lines = []
    for name, payload in results:
        if payload.get("parseError"):
            lines.append(f"FAIL  {name}  (gate did not return JSON: {payload.get('stderr', '')})")
        elif payload.get("passed"):
            lines.append(f"PASS  {name}")
        else:
            reasons = "; ".join(payload.get("failures", []))
            lines.append(f"FAIL  {name}  ({reasons})")
    return "\n".join(lines)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Gate-as-judge harness for OKF migrations")
    parser.add_argument("--ref-dir", required=True, type=Path)
    parser.add_argument("--candidate-dir", required=True, type=Path)
    parser.add_argument("--gate-bin", default="mw-gate", type=Path)
    args = parser.parse_args(argv)
    pairs = find_pairs(args.ref_dir, args.candidate_dir)
    if not pairs:
        print("no matching OKF pairs found", file=sys.stderr)
        return 2
    results: list[tuple[str, dict]] = []
    for name, (ref, cand) in pairs.items():
        results.append((name, run_gate(args.gate_bin, ref, cand)))
    print(render_table(results))
    return 0 if all(payload.get("passed") for _, payload in results) else 1


if __name__ == "__main__":
    sys.exit(main())
```

- [ ] **Step 2: Write judge/tests/test_judge.py**

```python
# SPDX-License-Identifier: AGPL-3.0-or-later
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import judge


class JudgeTests(unittest.TestCase):
    def test_find_pairs(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            ref = root / "ref"
            cand = root / "cand"
            ref.mkdir()
            cand.mkdir()
            (ref / "a.json").write_text("{}", encoding="utf-8")
            (ref / "b.json").write_text("{}", encoding="utf-8")
            (cand / "a.json").write_text("{}", encoding="utf-8")
            pairs = judge.find_pairs(ref, cand)
            self.assertEqual(set(pairs), {"a"})

    def test_render_table(self):
        out = judge.render_table([
            ("a", {"passed": True, "failures": []}),
            ("b", {"passed": False, "failures": ["integration: 2 connected components"]}),
        ])
        self.assertIn("PASS  a", out)
        self.assertIn("FAIL  b", out)
        self.assertIn("2 connected components", out)


if __name__ == "__main__":
    unittest.main()
```

- [ ] **Step 3: Run the unit tests**

Run: python -m unittest discover -s judge/tests -p "test_*.py" -v
Expected: 2 tests pass.

- [ ] **Step 4: Smoke test against the corpus (spawns the gate binary)**

```powershell
cargo build --workspace
python judge/judge.py --ref-dir sample/corpus/coffee-machine/okf/expected --candidate-dir sample/corpus/coffee-machine/okf/expected --gate-bin target/debug/mw-gate.exe
python judge/judge.py --ref-dir sample/corpus/coffee-machine/okf/expected --candidate-dir sample/corpus/coffee-machine/okf/corrupted --gate-bin target/debug/mw-gate.exe
```

Expected: the first run prints PASS coffee_machine_model and exits 0; the second prints FAIL coffee_machine_model and exits 1.

- [ ] **Step 5: Commit**

```powershell
git add judge
git commit -m "feat: add gate-as-judge harness"
```

---

### Task 11: Wrap-up — README, evidence index, full verification

**Files:**
- Modify: README.md (replace the stub with the full quickstart)

**Interfaces:**
- Consumes: everything built in Tasks 1 through 10.
- Produces: a repository that passes the complete verification sequence below; the final commit of Phase 0 + Phase 1.

- [ ] **Step 1: Replace README.md with the full quickstart**

```markdown
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

## Quickstart

Requirements: Rust (stable, via rustup) and Python 3.12+ for the judge
harness.

cargo build --workspace
cargo test --workspace

Run the gate on the corpus:

cargo run -p mw-gate -- --reference sample/corpus/coffee-machine/okf/expected/coffee_machine_model.json --candidate sample/corpus/coffee-machine/okf/expected/coffee_machine_model.json

Expected output: GATE PASS.

## Licence

The engine is AGPL-3.0-or-later. Importers and exporters will ship under
Apache-2.0 as they land. See NOTICE.md for attribution.
```

- [ ] **Step 2: Run the complete verification sequence**

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
python -m unittest discover -s judge/tests -p "test_*.py" -v
cargo run -p mw-gate -- --reference sample/corpus/coffee-machine/okf/expected/coffee_machine_model.json --candidate sample/corpus/coffee-machine/okf/expected/coffee_machine_model.json
```

Expected: every command exits 0; the final line is GATE PASS. Record the command output in the task completion report.

- [ ] **Step 3: Final review of the working tree**

```powershell
git status --short
git log --oneline
```

Expected: a clean tree and one commit per task (11 commits). If anything is uncommitted, commit it with an appropriate conventional message before finishing.

- [ ] **Step 4: Commit the wrap-up**

```powershell
git add README.md
git commit -m "docs: add quickstart README"
```

Note for the executor: pushing to the GitHub remote happens once the remote URL is confirmed; CI (Task 9) first runs on that push.

---

## Completion criteria

- [ ] cargo test --workspace passes on Windows locally.
- [ ] cargo fmt --all -- --check and cargo clippy --workspace --all-targets -- -D warnings pass.
- [ ] The gate passes on the corpus self-comparison and fails on the corrupted fixture; both evidence records are committed in docs/evidence/.
- [ ] judge.py returns 0 on the self-comparison and 1 on the corrupted comparison.
- [ ] All 11 task commits are on main; the tree is clean.

