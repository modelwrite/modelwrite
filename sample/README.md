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
