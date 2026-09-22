# STPA / STAMP vocabulary and completeness check

**Status:** shipped (T1-T3). The design note is
[2026-09-21-modelwrite-stpa-and-safety.md](superpowers/specs/2026-09-21-modelwrite-stpa-and-safety.md).

## The honest boundary

**The analysis is AUTHORED; the check is COMPUTED.** Modelwrite does not perform STPA, does not
infer hazards from a design, and will never claim to. It holds the control structure and the
analysis as a versioned OKF model, checks completeness and internal consistency, names everything
missing, and reports each finding's basis — what it was computed over and what the model did not
carry. This sentence is on the STPA completeness screen.

## The vocabulary as a declared domain pack

The authoritative, machine-readable declaration is
[`sample/stpa/stpa-vocabulary.json`](../../sample/stpa/stpa-vocabulary.json). Every STPA concept
is an existing OKF node kind carrying a declared stereotype, and every STPA relationship is an
existing OKF edge kind. No engine change is required to hold the vocabulary.

| Stereotype | OKF node kind | STPA meaning |
|---|---|---|
| Loss | block | a system-level loss to prevent |
| Hazard | block | a system state that leads to a loss |
| SystemConstraint | requirement | a system-level safety constraint |
| Controller | block | a component that issues control actions |
| ControlledProcess | block | the process under control |
| ControlAction | signal | a command the controller issues |
| Feedback | signal | information the process returns |
| UnsafeControlAction | block | a control action unsafe in a context |
| LossScenario | block | a causal scenario explaining a UCA |
| CausalFactor | block | a factor contributing to a scenario |

The four UCA types are carried as the **second stereotype** of an UnsafeControlAction node:
`not-provided`, `provided`, `wrong-timing-or-order`, `stopped-too-soon-or-applied-too-long`.

## The edge mapping

| STPA relationship | OKF edge |
|---|---|
| controller issues control action | `triggers` Controller → ControlAction |
| control action directs process | `triggers` ControlAction → ControlledProcess |
| process returns feedback | `triggers` ControlledProcess → Feedback |
| feedback reports to controller | `triggers` Feedback → Controller |
| hazard mitigated by constraint | `dependency` Hazard → SystemConstraint, label `Mitigate` |
| element satisfies constraint | `dependency` element → SystemConstraint, label `Satisfy` (existing coverage) |
| UCA analyses a control action | `reference` UCA → ControlAction |
| scenario explains a UCA | `reference` LossScenario → UCA |

Control action and feedback are signal nodes so the UCA analysis and the loss scenarios can
reference them by id. The control loop is therefore the four-node cycle
Controller → ControlAction → ControlledProcess → Feedback → Controller, all `triggers` edges.

## The five checks

1. **Unanalysed control actions** — every ControlAction must carry a UCA for each of the four types.
2. **Control loops with no feedback** — a Controller with a ControlAction and no Feedback path
   (the same directed-degree reading the orphan/isolation detector uses).
3. **Hazards with no constraints, constraints reaching no element** — the second half reuses the
   platform's requirement coverage, filtered to SystemConstraint requirements.
4. **UCAs with no LossScenario**.
5. **Trend across baselines** — the same counts per commit on a branch, computed directly (no
   cached trend path).

## Method coverage: three states, never two

Every check above is **relational** — it tests a relationship BETWEEN STPA elements. A model
that carries none of them therefore returns no findings, and no findings used to read as
"complete". That is a vacuous truth, and for a safety engineer it is a false assurance: a check
that passes because the thing it checks is not there.

Above the relational checks sits a method-coverage layer that verifies each STPA stage was
**performed at all**, and answers with one of three states (`StpaVerdict` in
`engine/graph/src/stpa.rs`):

- **Not started** — the model lacks the elements the method needs. The page names the missing
  stage (Losses, Hazards, SystemConstraints, the control structure, the UCA enumeration) and
  marks each check that has no subject as "cannot be evaluated yet". It never renders a bare
  zero and never wears the pass state.
- **Started, gaps found** — the stages were performed; the relational findings are named.
- **Performed, no gaps** — the only state in which a clean result is shown, and it names the
  counts it was computed over (losses, hazards, constraints, controllers, control actions,
  UCAs, loss scenarios) so a reader can see the coverage is real.

The verdict is computed, never stored: it is a function of the method gaps, and the gaps are a
function of the elements the model carries, so no report can claim a clean result over an empty
stage. Worked fixtures: `sample/stpa/no-hazards.json` (a complete model with no Hazard — zero
relational findings, reported as **not started**) and `sample/stpa/no-stpa-elements.json` (a
model with no STPA vocabulary at all).
The checks live in `engine/graph/src/stpa.rs`; the screen is `server/src/ui/stpa.rs` at
`/ui/projects/:project/stpa`; the control-structure view is a third diagram type
(`?view=control`). Worked fixtures: `sample/stpa/fire-suppression-defective.json` (each check
fires) and `sample/stpa/fire-suppression-correct.json` (silent).
