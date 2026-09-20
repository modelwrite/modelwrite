# Analyses and simulations — what we can build, and what we must not claim

## The ask

A user asks for a simulation to be run over a model; it is LLM-assisted; it comes back with findings and gaps; and over time there is a **library** of what has been run.

## What already exists, which this builds on rather than replaces

- **The gate** is already an analysis: it runs over a revision, produces a verdict, and writes a **deterministic evidence record**.
- **Checks are already attached to a commit** (`GET /projects/:project/commits/:hash/checks`), newest first, with the evidence. That is the seed of the library.
- **The analytics slice** already fuses external records and reports findings with weakest-link confidence.
- **The agent loop** already has the trust shape: a `Reasoner` proposes, a human accepts, and provenance names both.
- **The compositional gate (S1)** already checks across models at pinned revisions.

So the missing pieces are not a new engine — they are a **frame**: a definition, a run, findings, and a library.

## Rulings

1. **AN ANALYSIS IS A RECORD, NOT A SIDE EFFECT.** Every run is stored against the revision it examined, with the definition it ran, its findings, and its evidence. *Cost if wrong:* storage. *Benefit:* the library, and the ability to answer "what did we know about this revision in March?"
2. **THE LLM AUTHORS AND INTERPRETS; THE ENGINE MEASURES.** Findings are produced by deterministic code over the model. The LLM may propose an analysis definition and may narrate findings — and any narration is **labelled as interpretation, beside the measured findings, never instead of them.** *Cost if wrong:* less magic. *Benefit:* a finding is reproducible and checkable, which is this platform's entire promise.
3. **AN ANALYSIS DEFINITION IS REVIEWED LIKE A MODEL CHANGE.** LLM proposes → a human accepts → the accepted definition is committed and addressable, and a run names the definition version that produced it.
4. **FINDINGS ARE ADDRESSABLE AND DIFFABLE.** Two runs can be compared, so "did this get better?" is a **diff**, not a feeling. This falls out of running against commits.

## The honest scoping: analysis vs simulation

These are different things and the product must not blur them.

**Analysis (buildable now):** checks over the model's structure, coverage, traceability, composition and cost. Examples: every requirement is covered; every requirement has a verifying activity; no block is orphaned; every platform reference resolves at its pinned revision; the stand's process has no step that can run before payment approval.

**Model-level simulation (buildable, a real tranche):** executing the model's OWN behaviour — walking the activity/process graph, checking reachability, finding dead ends and cycles, and doing **timing arithmetic** where activities carry durations (the cafe-stand's "coffee and toastie within 4 minutes of approval" is arithmetic over a process, not physics). Deterministic, reproducible, and provable.

**Numeric or physical simulation (NOT buildable here, and must not be claimed):** solving fluid, thermal or structural behaviour requires solvers and executables this platform does not have. If an organisation needs that, the honest answer is that it is a different tool, and the platform's role is to hold the model and the requirements the solver is checked against.

**A definition that cannot be expressed in the bounded vocabulary must be REFUSED with a reason, never silently approximated.** That is the same rule as the binding's unmappable elements: say what cannot be done rather than pretending.

## The frame

- **AnalysisDefinition** — id, name, version, the check vocabulary it uses, its parameters, and its provenance (built-in, or proposed by an agent and accepted by a human).
- **AnalysisRun** — definition version + project + pinned revision + when + who/what ran it + the findings + an evidence record in the existing deterministic format.
- **Finding** — severity, subject (the element or requirement id), a one-line statement, the measured evidence behind it, and optionally an interpretation clearly marked as such.
- **Library** — runs over time per model, filterable, each addressable, two runs diffable.

## Slices

| Slice | Deliverable |
|---|---|
| **A1** | The frame: definition, run, findings, storage; two BUILT-IN analyses (the round-trip gate; requirement coverage) prove it end to end |
| **A2** | The library: runs over time per model, each addressable, two runs diffable |
| **A3** | The bounded check vocabulary + LLM-authored definitions (proposed, validated, refused when not expressible, accepted by a human) |
| **A4** | Model-level simulation: walk the process graph, find unreachable steps, dead ends and cycles, and do timing arithmetic over a process |
| **A5** | The narrative: findings summarised by the LLM, labelled as interpretation, always beside the measured findings |

## Acceptance criteria

- A run is reproducible: the same definition over the same revision produces the same findings and the same evidence bytes.
- Every finding names its subject and the measurement behind it; nothing is asserted that was not measured.
- The library lists runs over time, and two runs can be diffed.
- An analysis the vocabulary cannot express is **refused with a reason**.
- An LLM-authored definition is a proposal a human accepts, and the run records whose definition it was.
