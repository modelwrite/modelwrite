# The low-friction entry: bring your model, ask it to onboard and check it

**Status:** design, from the owner's direction: *"make it as easy as possible for users to use it... this must be the GenAI LLM-enabled interface from the start. Bring your model, ask it to onboard it and check it, low friction."*

## The problem with the front door today

The workbench opens on **a list of projects and an empty "New project" text box**. For the person this product is for - an engineer with a 36 MB Cameo export and a review next month - that front door asks them to do the hard parts themselves: know which binding to pick, start a project, upload, read a 50,784-row loss report, and decide what to accept. Every one of those steps is where they give up.

**The front door should be a drop zone and a question.**

## The flow

1. **DROP.** The first thing on the page is a large target: *"Drop your SysML model here, or point at a file."* Accepted: XMI, `.mdzip` (the Cameo container), `.sysml`, and a Capella project folder. No project creation step, no format choice.
2. **IT IDENTIFIES AND ONBOARDS.** The platform detects the format, picks the binding, retains the artifact content-addressed, runs the import, and reports - **in plain language, generated from the engine's own data**: *"This is a MagicDraw export, 362 blocks, 633 relationships, 7 requirements. It round-trips exactly. 48,553 things are outside what I carry; 2,828 are dropped identifiers and the rest are construct types I do not map yet. Here are the five biggest classes."*
3. **IT ASKS ONE QUESTION.** *"Import the 362 blocks and 633 relationships, accepting those 48,553 named losses? You can see every one first."* One button. The long list stays available and never disappears.
4. **IT CHECKS IT.** Immediately after import: the health view, in words. *"7 requirements, none covered. 369 nodes in 153 disconnected groups, 134 of them isolated. Your model has no traceability links."* Then: *"Want me to look at why?"*
5. **ASK.** A persistent ask box: *"Which requirements have no verification?"*, *"What is orphaned?"*, *"Where is the pump?"*, *"Is my fire-suppression controller's feedback complete?"* Every answer carries **the evidence and the command that produced it**.

## The one rule that makes this safe

**THE LLM AUTHORS AND INTERPRETS. THE ENGINE MEASURES.** This is already the platform's ruling and it becomes load-bearing here:

- every number the assistant speaks comes from an engine computation, **never from the model's own reasoning**;
- every answer states **what it was computed over** and **what the model did not carry** (the basis rule, unchanged);
- the assistant may say *"I don't know"* and it must never invent a count;
- an answer that cannot cite a measurement is labelled **interpretation, not measurement**.

A chat interface over a model platform is exactly where a confident-sounding wrong number would do the most damage. The whole product exists to prevent that, so the assistant is the LAST place to relax it - and if the interface makes the caveats feel like friction, the interface is wrong, not the rule.

## What this changes in the product

| Today | Becomes |
|---|---|
| Projects list, then a New project box | **Drop a model, or ask a question** |
| Pick a binding from a dropdown | **Detected; the dropdown stays for the expert** |
| Read a 50,784-row loss report | **A plain-language summary with the five biggest classes, and the full list behind it** |
| Click through six pages to learn what is wrong | **Told, in three sentences, with the links to the evidence** |
| Type a question in a page called Assist | **Ask from anywhere, with answers carrying their basis** |

## Slices

| | Deliverable |
|---|---|
| **E1** | The drop zone front door: drop -> detect -> retain -> import -> plain-language loss summary -> one-button accept |
| **E2** | The post-import health briefing: the engine's findings rendered as prose, with the evidence behind every sentence |
| **E3** | The persistent ask box, wired to the engine's computable questions first (coverage, gaps, STPA completeness, where-is-X) - NOT free-form reasoning over the document |
| **E4** | The refuse-to-guess path: a question the engine cannot answer is labelled as interpretation, and the UI shows which questions ARE computable |

**E1 is the product.** Most of the machinery exists: the streaming import, the loss report, the acceptance path, the health view, the STPA checks. What is missing is the FLOW that makes a stranger able to use them without reading documentation.
