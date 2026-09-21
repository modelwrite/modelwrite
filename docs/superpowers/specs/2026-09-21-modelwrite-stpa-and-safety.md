# STPA / STAMP: what Modelwrite can honestly offer

**Status:** design note, written in response to external defence-safety feedback. Not yet built.
**Trigger:** *"SysML view of the world it's all a bit old-school... When I went to USC on a System Safety Course we had Lockheed Martin, Northrup Grumman guys and they were all about just doing the MIL-STDs as opposed to Nancy's STPA. Not sure you can plug this gap."*

## The gap, restated precisely

Two things are being said, and they need separating:

1. **A methodology gap.** Defence safety practice leans on prescriptive standards (MIL-STD-882 and its children) while the state of the art - Leveson's **STAMP** accident model and **STPA** analysis technique - treats safety as a **control problem** rather than a failure problem. The two produce different artefacts and different conversations.
2. **A tooling gap.** STPA work today lives in documents and spreadsheets, or in a plugin (MIT's STAMP Tools has a **Capella** plugin). Neither gives you a defensible answer to *"is the analysis complete, and what changed since the last baseline?"*

**Modelwrite cannot close (1) - that is a culture and policy question, and no tool closes it.** It can close (2), and the shape of (2) is exactly what this platform already does.

## What STPA actually produces (and why it is a graph)

STPA derives, in order: **losses** -> **hazards** -> **system-level constraints** -> a **control structure** (controllers, controlled processes, **control actions**, **feedback**) -> **unsafe control actions (UCAs)**, enumerated four ways per control action (not provided / provided / too early-too-late-wrong-order / stopped-too-soon-applied-too-long) -> **loss scenarios** (why the UCA could occur) -> refined constraints and requirements.

**That is a typed graph with containment, references and traceability.** It is not a document. Every arrow in it is a claim that can be checked:
- a controller issues a control action -> an edge
- a controlled process returns feedback -> an edge
- a hazard is mitigated by a constraint -> an edge
- a constraint becomes a requirement -> an edge, and the requirement's coverage is already computed by this platform

## What Modelwrite already has that maps onto it

| STPA needs | Modelwrite has |
|---|---|
| typed elements and edges | OKF with `NODE_KINDS` and `EDGE_KINDS` (contains, triggers, uses, dependency, reference, subject, effect, transition...) |
| a control structure diagram | the layered diagram renderer with orthogonal routing and no box crossings |
| constraint -> requirement traceability | **requirement coverage** and traceability, already computed by the engine |
| an analysis record that is reviewable and diffable | **commits, provenance, diffs, the gate** |
| findings that are addressable | the analyses frame (analyses are records; findings are addressable and diffable) |
| honest reporting of what was NOT analysed | **the basis rule** - what it was computed over, what was not carried |

## THE DIFFERENTIATING CAPABILITY: completeness checking

This is the part no document, spreadsheet or plugin gives you, and it is **computable**:

1. **UNANALYSED CONTROL ACTIONS.** Every control action in the control structure must have a UCA analysis covering the four types. A control action with three of four, or none, is a **named, countable gap**.
2. **CONTROL LOOPS WITH NO FEEDBACK.** A controller with a control action and **no feedback path** is the classic STPA defect - and it is structurally the same shape as the **orphan / isolated group / dangling link** this platform already detects and now displays on one screen. *The gap-detector you asked for on models is, applied to a control structure, an STPA completeness check.*
3. **HAZARDS WITH NO CONSTRAINTS, CONSTRAINTS WITH NO ELEMENTS.** Every hazard the analysis names must be constrained; every constraint must reach the design - otherwise it is an aspiration.
4. **UCAs WITH NO SCENARIO.** "It could happen" without a causal scenario is an assertion, and the platform will say so.
5. **TREND ACROSS BASELINES.** Coverage, unanalysed actions, feedback gaps and churn as a line towards a design review - the "model health by baseline" analytic, applied to the safety argument.

**That is the offer: not "we do STPA", but "we show your STPA is complete and consistent, and prove what changed between revisions."** A safety case needs exactly that, and it is the one thing a document cannot do.

## The honest boundary - and it is the whole reason this is credible

**The analysis is AUTHORED; the platform CHECKS it.** Modelwrite does not perform STPA, does not infer hazards from a design, and will never claim to. It:
- holds the control structure and the analysis as a versioned model,
- **checks completeness and internal consistency** (the five checks above),
- names everything that is missing,
- and reports each finding's basis: what it was computed over, and what the model did not carry.

**A platform that implied it had performed a safety analysis would be dangerous.** The basis rule - the same rule that stops a coverage number appearing without its caveat - is what makes this usable in a safety argument rather than a liability in one.

## Interoperability: do not take a side

- **MIL-STD-882 and STPA are both supported as vocabularies**, with a mapping table under the existing interoperability discipline: every mapping verdict is `exact`, `lossy` or `unmappable`, and **nothing is mapped silently**. A programme that must produce 882 artefacts gets 882 artefacts; a programme that wants STPA gets STPA; a programme doing both gets **the mapping between them, stated**.
- **Read Capella / Arcadia**, because MIT's STAMP Tools plugin lives there and a great deal of STPA work is done in it. A Capella binding is the interoperability route: **keep doing STPA in Capella, and bring the model here for the completeness check, the traceability and the baseline trend.** That is a stronger position than competing with the plugin.
- The Capella samples are already in the corpus (`sample/examples/arcadia-capella` - EPL-2.0, fetched not bundled).

## Slices

| Slice | Deliverable |
|---|---|
| **T1** | **The STPA vocabulary as a declared domain pack**: Loss, Hazard, SystemConstraint, Controller, ControlledProcess, ControlAction, Feedback, UCA (with the four types), LossScenario, CausalFactor - as stereotypes with a documented mapping, no engine change |
| **T2** | **The control-structure view**: a diagram type showing controllers, control actions and feedback loops, with the existing gap detector applied (no feedback = flagged) |
| **T3** | **The completeness check**: the five checks as engine-computed findings with counts and names, on one screen, trended by baseline |
| **T4** | **The Capella binding**: read an Arcadia/STPA model so a team can keep their tool and get the check |
| **T5** | **The 882 mapping table** under the interoperability rules, with its verdicts |

**T1-T3 are the product. T4 is the adoption route. T5 is the peace treaty.**

## What this says to the feedback

*"Not sure you can plug this gap."* **Not the methodology gap - nobody plugs that with software.** But the gap underneath it, the one that stops STPA being adopted in a defence programme, is **defensibility**: an STPA result that cannot be shown complete, cannot be traced to the design, and cannot be compared with last year's baseline is an argument, not evidence. **Making it evidence is what this platform is for.** And it does it without asking a Lockheed or Northrop safety engineer to abandon the standards they are accountable to.
