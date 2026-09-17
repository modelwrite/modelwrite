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
