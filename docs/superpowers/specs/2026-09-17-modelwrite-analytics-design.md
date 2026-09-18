# Analytics and data fusion - the questions the model cannot answer alone

**Status:** slice 7 specification. Written before any code, as required: analytics comes last, and it answers questions that cross the boundary of a single model.

## What this is for

Everything before this slice answers questions about ONE model: is it valid, what changed, can it be merged, did the migration lose anything, has it been checked. The questions an organisation actually spends money on are not like that:

- **Which products meet which specifications?** A requirement set is per-project. A portfolio question spans projects, and often spans the supplier's documents too.
- **Which requirements have no provider?** Coverage exists inside a model (the graph tells us a requirement has no satisfying element); the real question is which requirements have no WORK, no owner, no cost and no date.
- **Which requirements are the most expensive?** Cost lives in an ERP, a spreadsheet, a contract. Nothing in a model knows what a part costs.
- **What is changing in the market that affects us?** Standards get revised, components go end-of-life, suppliers are acquired. That is unstructured text, not a model.

So the slice fuses THREE kinds of source, and the design principle is that they keep their own identity:

1. **Models** (OKF, from this platform): structured, provable, versioned, with an audit trail.
2. **Structured records** (spreadsheets, ERP and PLM exports, CSVs): tabular, unversioned, of unknown provenance.
3. **Unstructured text** (specifications, supplier documents, standards, news): the least reliable and often the most current.

## The ruling that shapes everything

**A FUSED ANSWER MUST BE TRACEABLE TO ITS SOURCES, AND ITS CONFIDENCE MUST BE ITS WEAKEST LINK.**

An answer that mixes a proven model with a scraped price and reports one number is a lie by arithmetic. Every figure carries: which source it came from, when that source was read, how trustworthy the source is, and whether it was measured or asserted. A total built from one measured value and three estimated ones is an ESTIMATE, and must say so.

The second half matters as much: **analytics never writes to a model.** It reads, and it produces findings. A finding that should change a model becomes a PROPOSAL, which Slice 5 already has the shape for - so the analytics slice answers questions without becoming a second way to change engineering data.

## The three questions, made precise

**Q1 - Portfolio compliance.** Given several projects and a specification (a standard, a customer requirement set, a regulation), which projects' models satisfy it and which do not? The answer must distinguish three very different states, and never blur them:
- **Satisfied**: a requirement is present and coverage proves it is met.
- **Not satisfied**: a requirement is present and the graph shows it unmet.
- **Unknown**: the requirement is NOT in the model at all. This is the most common answer and the most dangerous one to render as a blank, because a missing requirement and a met requirement look identical in a naive report.

**Q2 - Cost and effort.** Which requirements are expensive, and which have no estimate? Requires joining requirements to cost data from outside the model. Every joined figure is labelled with its source and its date, because a cost from three years ago is an estimate about the past.

**Q3 - Change and risk.** What in the outside world affects this model? End-of-life components, revised standards, supplier changes. This is the unstructured half, and it is where an agent earns its place - extraction from text is exactly the work no engineer has time for. Every extracted claim is a FINDING with its source quoted, never a fact.

## Architecture

- **A source registry**: every external source declared with its kind, its location, when it was read, and its trust level. A source that is not registered cannot be queried.
- **A dataset layer**: external data as versioned, content-addressed SNAPSHOTS, so an answer can be reproduced a year later and a changed spreadsheet does not silently change history. This reuses the repository's own content addressing: a snapshot is a blob plus a record of where it came from.
- **A query layer over models plus datasets**, which returns findings with provenance rather than bare numbers.
- **An agent-assisted extraction path** for the unstructured half, producing findings with quoted evidence, reviewed by a human exactly as Slice 5's proposals are.

## What this slice must NOT do

- **Not a BI tool.** No chart builder, no dashboards as an end in themselves. The output is an ANSWER with its evidence, and a person decides what to do with it.
- **Not a second write path.** Findings that imply a model change become proposals.
- **Not a place where confidence is averaged.** The weakest link governs.
- **Not dependent on a network at query time.** Snapshots are local; a source is refreshed deliberately, not sampled live.

## Open questions for the slice's own tasks

- How is a specification from outside expressed so it can be checked against a model? The most plausible answer is that an external requirement set is IMPORTED through the existing binding machinery into a model-like form, so compliance is then an ordinary graph question - which would reuse Slice 4 rather than invent a second comparison engine.
- What is the smallest useful cost join, given that no two organisations store cost the same way? A named-column mapping declared per source is the likely answer.
