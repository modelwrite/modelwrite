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
