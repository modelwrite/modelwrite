# Evidence annex

Every public claim about modelwrite traces to a recorded run in this
directory. Records are append-only: never rewrite an existing record.

Naming: YYYY-MM-DD-<subject>.json, except the fixed-name
analytics-transport-equivalence.json (its name is the contract CI diffs).

Each record is the deterministic output of the tool named inside it: given
the same inputs it reproduces exactly (hashes and all). The roundtrip
records are mw-gate --evidence over a reference and a candidate; the
analytics record is server/tests/equivalence.rs over a scratch store and a
live server. Dates appear only in filenames, never inside records.

Index:

- 2026-09-17-roundtrip-self-pass.json — the corpus compared with itself:
  the gate passes. Proves the reference OKF is a fixed point of the gate.
- 2026-09-17-roundtrip-corrupted-fail.json — the corpus compared with the
  corrupted fixture: the gate fails on missing elements and isolated
  nodes. Proves the gate detects loss and fragmentation.
- analytics-transport-equivalence.json — REST, the CLI (in both --db and
  --server modes), MCP and Python read every mw-analytics-schema@1 table
  for the sample corpus and for one large real import; the rows are
  identical. It records the CLI --server route divergence and the Python
  float64 representation difference rather than hiding them, and injects a
  deliberate divergence that the checker detects, so the record proves it
  can fail.
