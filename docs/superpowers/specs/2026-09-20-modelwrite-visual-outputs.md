# Visual outputs: diagrams, documents and packs

## The ask

Turn the models into things that can be **used in presentations and reviews** - images of the model, a document that assembles itself, and evidence packs that can be handed to somebody who was not in the room.

## What we already have, which makes this cheap

The diagrams are **deterministic server-rendered SVG**. That is not a rendering detail; it is the whole reason this is achievable without a design tool:
- SVG is **vector**, so it scales to a projector or a printed A3 page without loss.
- It is **text**, so it diffs, versions and content-addresses like everything else here.
- It is **generated from the model**, so a picture can never disagree with the model it claims to show.
- The same model renders **byte-identically** every time - already a tested property.

## Rulings

1. **AN EXPORT IS GENERATED FROM THE COMMIT, NOT FROM THE SCREEN.** A diagram or document exported for revision `abc123` shows revision `abc123`, and says so on its face. *Cost if wrong:* exports take a parameter. *Benefit:* a picture in a slide deck from March can be checked against the model in March, which is the entire reason an engineering organisation keeps slides.
2. **NO DEPENDENCY FOR WHAT THE BROWSER ALREADY DOES.** SVG is native; paginated PDF comes from the browser's own print-to-PDF driven by a print stylesheet. No PDF library, no headless browser in the product, nothing fetched. An air-gapped organisation can print from the same page it reads.
3. **EVERY OUTPUT CARRIES ITS PROVENANCE.** Project, revision, and the time it was generated appear on the artefact itself. A slide with a model picture and no revision is a rumour.
4. **READABLE AT PROJECTION SCALE.** A diagram that is legible at 100% on a laptop is not legible on a projector. Outputs are sized for their medium, and text never falls below a floor.

## The outputs

### V1 - Diagram export (the fastest win)
- A **download route** for the SVG of any diagram: structure, process, and the composition view. The same bytes the page renders, with a proper filename carrying the project and revision.
- A **full-screen presentation view** of a diagram - chrome, navigator and panels hidden, the diagram filling the viewport, with the project/revision caption kept. This is what goes on a projector.
- Determinism is tested: two exports of the same revision are byte-identical.

### V2 - Print stylesheet
- Every workbench page prints cleanly: no navigation, no panels, no buttons; page breaks that keep a table row and its heading together; the revision in the running header.
- The requirement table, coverage summary, gate results and checks print as a document, not as a screenshot of a web page.

### V3 - The model document (the one that matters for reviews)
A printable document assembled from the model, addressed to whoever has to approve something:
1. **Cover**: project, revision, generated-at, and the honest one-line statement of what the model is and is not.
2. **Summary**: counts, coverage verdict, the latest gate result.
3. **Structure**: the containment view as a diagram *and* an indented list (some readers want the picture, some want the list).
4. **Requirements**: the table with reqIds, text, and coverage state per requirement.
5. **Traceability**: what satisfies what, with the uncovered list stated explicitly - never omitted.
6. **Composition** (for a platform model): the subsystems, their pinned revisions, the process, and the measured-vs-asserted boundary.
7. **Checks**: what has been run on this revision, with verdicts.
Every section headed by a page break, and **the whole document carrying its revision on every page**.

### V4 - The evidence pack
The audit-shaped export: the model document plus the analysis runs, the gate evidence records, the commit provenance (authored / imported / unknown) and the audit entries - as files in a directory, each content-addressed, with an index. This is what an organisation hands to an assessor, and it is the reason the records were stored this way from the start.

## What this is NOT

- **Not a slide designer.** The output is a diagram or a document, not a themed deck; people will drop the SVG into their own deck, and that is the right boundary.
- **Not a rendering engine.** No layout engine of our own beyond what the diagram work already provides.
- **Not lossy by default.** SVG, not a screenshot; text, not an image of text.

## Slices

| Slice | Deliverable |
|---|---|
| **V1** | SVG download for every diagram type + a full-screen presentation view, both carrying project and revision |
| **V2** | Print stylesheet across the workbench |
| **V3** | The model document (cover, summary, structure, requirements, traceability, composition, checks) |
| **V4** | The evidence pack for an assessor |

## Acceptance criteria

- An exported SVG of a revision is byte-identical to another export of the same revision, and shows the revision on its face.
- The presentation view renders a diagram legibly at projector scale with the chrome gone.
- Every workbench page prints without navigation, panels or buttons, with the revision in the header.
- The model document states the uncovered requirements explicitly and never omits them.
- Nothing is fetched, no dependency is added, and the whole thing works offline.
