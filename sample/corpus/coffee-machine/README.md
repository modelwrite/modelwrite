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
