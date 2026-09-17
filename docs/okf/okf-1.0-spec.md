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
| exportedAt | string | no (expected: exporters emit it) | Export timestamp. Exporters SHOULD emit ISO-8601; legacy exports may be free text. |
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

These spellings are the vocabulary the platform uses on dependency
edges; they are checked by the coverage analysis, not by document
validation.

## 4. Section item shapes

Element: id (required, opaque, stable), name, kind, stereotypes[],
attributes[] (each with name, type, aggregation, default), documentation.

Requirement: everything in Element plus reqId (required, non-empty) and
reqText.

State: id (required), name, entry, doActivity, exit. The entry, doActivity
and exit fields reference behaviour names, not ids.

StateMachine: name plus regions[]; each region holds states[].

Activity: name, partitions[] (each an object with a name and an optional
represents field naming the element the lane represents), nodes[] (each with
id, type, name, partition) and edges[] (each with type, source, target,
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
exportedAt. exportedAt is expected of every export and emitted by every
exporter, but it is not a schema-required field; the runtime validator
does not reject a document that omits it.
