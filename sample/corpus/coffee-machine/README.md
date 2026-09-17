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

## Known finding in this export

Validation reports two warnings for this corpus: two dependency edges labelled
Satisfy reference a source element that the exporter never emitted as a node, so
they point at nothing in the graph. Exactly 2 of the 165 edges are affected, and
the finding was surfaced by the engine while implementing the okf crate.

The edges are kept exactly as exported: the corpus is the verbatim record of what
the legacy tool produced, and repairing it by hand would destroy the evidence.
The test suite pins the count at two, so the finding cannot silently change, and
the graph health metrics report it as a model-health defect rather than hiding it.

This is the first real model-health defect modelwrite found in the reference
material, and it is the kind of thing a diagram-by-diagram review does not catch.
