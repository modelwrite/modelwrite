# MIL-STD-882 mapping (STPA to MIL-STD-882E)

**Status:** documentation. Slice T5 of the STPA design note (the design note calls T5 the peace treaty). This is the declared mapping, under the interoperability discipline in [2026-09-17-modelwrite-standards-interoperability.md](../superpowers/specs/2026-09-17-modelwrite-standards-interoperability.md). It has not yet been promoted to a machine-readable matrix.

A defence programme that must produce MIL-STD-882 artefacts reads this platform's STPA vocabulary and asks a fair question: does using it mean abandoning the standard it is accountable to. The answer is no, and this document is the proof. It maps every STPA concept to its MIL-STD-882 counterpart, and every 882 concept back to STPA, with a verdict on each row. Nothing is mapped silently. That rule is the platform's house rule, and it is the whole reason this table has value.

## Which revision, and why it matters

This document maps against **MIL-STD-882E** (11 May 2012), the current revision, which supersedes MIL-STD-882D (10 February 2000). Revisions differ, and they differ in the places this table touches: the wording of the hazard and mishap definitions, the severity and probability category definitions and their thresholds, and the set of hazard analysis tasks. A programme on an earlier revision must re-check each row against its own purchased copy. Where this document is unsure of a standard's detail, it says so rather than printing an unchecked claim.

## The house rule

Three verdicts, and nothing silent.

- `exact`: the target expresses the same thing. The mapping is mechanical.
- `lossy`: the target expresses something close, and the difference matters. The difference is named in the row.
- `unmappable`: the target has no counterpart. The concept stays in its own vocabulary, and the table says so.

## STPA to MIL-STD-882

The STPA meanings are from [../stpa-vocabulary.md](../stpa-vocabulary.md).

| STPA concept | STPA meaning | Nearest 882 concept | Verdict | Reason (for anything not exact) |
|---|---|---|---|---|
| Loss | a system-level loss to prevent | Mishap | `lossy` | Same enumerated outcomes (death, injury, damage to equipment or property, environmental damage) but not the same thing. A Loss is a value that is lost, and it includes loss of mission, which has no 882 mishap home. A Mishap is the unplanned event, not the value. Event frame and value frame are close, not identical. |
| Hazard | a system state that leads to a loss | Hazard | `exact` | Both name a condition or state from which harm can follow. STPA scopes it to a system state tied to a loss; 882 says condition tied to a mishap. That is a difference in where each method draws its boundary, not a different concept. |
| SystemConstraint | a system-level safety constraint | Safety requirement | `lossy` | Same function: constrain the system so the hazard cannot happen. Not the same object. A SystemConstraint is a system-level must-never or must-always statement. A 882 safety requirement is derived through the mitigation order of precedence and includes warnings, procedures and training, which STPA does not count as system constraints. One constraint can expand into several requirements. |
| Controller | a component that issues control actions | (none) | `unmappable` | 882 has no control-structure vocabulary. The nearest term, system or subsystem component, carries no controller or control-loop meaning. |
| ControlledProcess | the process under control | (none) | `unmappable` | 882 models a system and its functions, not a process under control in a feedback loop. |
| ControlAction | a command the controller issues | Function (in Functional Hazard Analysis), at a stretch | `unmappable` | A control action is not a function. Its whole safety meaning lives in the four-way unsafe-control-action analysis, which 882 does not have. Mapping it to function would be exactly the silent loss this table exists to prevent. |
| Feedback | information the process returns | (none) | `unmappable` | 882 has monitoring, instrumentation and safety devices, but not feedback as a control-loop signal. The loop semantics have no 882 home. |
| UnsafeControlAction | a control action unsafe in a context | Hazard cause | `lossy` | 882 hazard analyses do record causes, so there is a real landing spot. But 882 has no construct for a control action unsafe in a context, and the four-way enumeration (not-provided, provided, wrong-timing-or-order, stopped-too-soon-or-applied-too-long) has no counterpart. Only a cause survives; the control framing is dropped. |
| LossScenario | a causal scenario explaining a UCA | Hazard cause chain / mishap description | `lossy` | 882 hazard analyses capture causes and effects, which is where a scenario's substance sits. But 882 has no first-class scenario object, and the tie from a scenario to a specific UCA is STPA-specific. The narrative survives; the structure does not. |
| CausalFactor | a factor contributing to a scenario | Hazard cause / contributing factor | `lossy` | A single 882 cause line item is a fair home. The loss: STPA causal factors deliberately include non-failure causes (a flawed requirement, an incorrect mental model, missing feedback, a coordination failure), and 882's cause taxonomy is failure-centric. |

## MIL-STD-882 to STPA

| 882 concept | 882 meaning | Nearest STPA concept | Verdict | Reason (for anything not exact) |
|---|---|---|---|---|
| Hazard | a real or potential condition that could lead to a mishap | Hazard | `exact` | Mirror of the STPA row. One concept, two framings of its boundary. |
| Mishap | an unplanned event or series of events resulting in death, injury, occupational illness, damage to or loss of equipment or property, or damage to the environment | Loss | `lossy` | A mishap is an event; a Loss is a value lost. The sets do not coincide: loss of mission is a STPA loss with no mishap home, and occupational illness is a mishap outcome that is not a canonical STPA loss. |
| Severity | the magnitude of potential consequences of a mishap (categories 1 to 4) | (none) | `unmappable` | STPA has no severity scale. It ranks hazards by the losses they lead to and the constraints they violate, not by a category. A severity category is a 882-assigned judgement with no STPA home. |
| Probability | the likelihood that a hazard will cause a mishap (lettered levels) | (none) | `unmappable` | STPA has no probability. STAMP treats an accident as the product of interacting factors and argues that the likelihood of a rare high-severity event cannot be estimated reliably. This is a genuine disagreement, not a gap the mapping can bridge. |
| Risk index (risk assessment code) | the severity-probability pair from the risk assessment matrix | (none) | `unmappable` | Risk index is severity and probability combined. Both inputs are unmappable, so the index is unmappable. STPA has no single-number or coded risk expression. |
| Safety requirement | a requirement derived to eliminate or control a hazard | SystemConstraint, plus refined requirements | `lossy` | Same function, same asymmetry in reverse. A 882 safety requirement can become a STPA constraint, but 882's warning, procedure and training mitigations, and its order-of-precedence derivation, are not system constraints. |
| Verification | confirming a safety requirement is met (analysis, demonstration, inspection, test) | (none) | `unmappable` | STPA produces constraints and refined requirements but defines no verification method taxonomy and no verification closure. A programme verifies a STPA constraint with 882's methods; STPA itself says nothing about how. |

## Verdict counts

Across the two tables, deduplicating the three mirrored pairs (Hazard, Loss-Mishap, SystemConstraint-safety requirement):

- `exact`: 1
- `lossy`: 5
- `unmappable`: 8

14 concepts mapped in total. The asymmetry is the finding: the two models share their ends (hazards, and the requirements or constraints that prevent them) and share almost nothing in the middle. 882 carries risk language STPA does not have; STPA carries control language 882 does not have.

## Where the two genuinely disagree

These are two different accident models. The table does not pretend otherwise, and neither should a programme.

MIL-STD-882 is failure-based. It starts from a hazard, a condition that could lead to a mishap, assigns the mishap a severity and a probability, computes a risk, and drives mitigations through an order of precedence: eliminate the hazard by design, reduce the risk by design, add safety devices, add warnings, add procedures and training. Its unit of analysis is the failure event and its likelihood.

STPA is control-based. It starts from a loss, a value to protect, derives hazards as system states, builds a control structure, and asks which control actions, in which contexts, could be unsafe, and what causal scenarios explain them. Its unit of analysis is the control loop and the constraints on it. An accident, in this model, is not a component failure; it is control that was inadequate, missing, or misapplied, and it can happen with no component failed at all.

The sharpest disagreement is probability. 882 requires a probability for every hazard, because risk is severity and probability together. STAMP's stated position is that the probability of a rare high-severity accident is not estimable, and that concentrating on a number draws effort away from understanding the causal structure that prevents the accident. Those two positions cannot be reconciled inside a mapping table, and this document does not try. A programme that must report a risk assessment code reports one; STPA does not supply it, and the table says so rather than inventing a number.

The smaller disagreements follow from the same root. 882's hazard is a condition leading to a mishap; STPA's hazard is a system state leading to a loss. 882 models the system and its failures; STPA models the control structure and its constraints. 882 closes the loop with verification of safety requirements; STPA closes the loop with refined requirements and leaves verification to whatever method the programme already uses.

What they share is the ends. Both name hazards. Both derive requirements or constraints to prevent them. Both demand that the requirement reach the design and be traced. That overlap is exactly the rows marked exact or lossy. The rest is two methods looking at the same system and drawing different pictures.

## What a programme does with this

A programme that must produce 882 artefacts keeps producing them. Nothing in the deliverable set changes: the hazard analyses, the risk assessment matrix, the hazard tracking system, the safety requirements and their verification all stay as the standard requires. Modelwrite holds those artefacts as a versioned model and checks them for the same coherence it checks any model: a hazard with no mitigation, a safety requirement with no verification, a mitigation that reaches no element in the design.

If the programme also runs STPA, or brings in an STPA or Capella model, the two vocabularies sit in the same repository and are queried together. The table above is the published boundary between them. What crosses is marked exact or lossy, and the loss is named. What does not cross is marked unmappable and stays in its own vocabulary. The programme reads the table. It is never asked to convert 882 into STPA or STPA into 882, and the platform takes no side between them. It states the mapping, with the verdicts, and leaves the choice to the programme.

## Claims to check against the purchased copy

This document maps against MIL-STD-882E and paraphrases its definitions. Read these from the programme's own purchased copy before this table is used in a safety case:

- The exact wording of the hazard and mishap definitions. The definitions above are the standard's long-standing wording, paraphrased.
- The severity category definitions and their thresholds (personnel, dollar, environmental). These changed between 882D and 882E, and the dollar figures must not be taken from this document.
- The probability level set and wording. 882D uses five lettered levels (Frequent through Improbable); 882E's exact level set, and how an eliminated hazard is recorded, must be checked.
- The risk assessment matrix layout, the risk level names, and the risk acceptance authority for each level. Acceptance authority is set by programme policy, not by this document.
- The task numbers of the hazard analysis tasks. This document names them (Preliminary Hazard Analysis, System Requirements Hazard Analysis, Subsystem Hazard Analysis, System Hazard Analysis, Operating and Support Hazard Analysis, Functional Hazard Analysis) but cites no task numbers, because the numbering differs between revisions.
- The order of precedence wording. The five steps are stated in substance above; the standard's own wording governs.
