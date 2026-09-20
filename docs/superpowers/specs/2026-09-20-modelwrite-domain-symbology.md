# Domain symbology: MIL-STD-2525 and APP-6

**Status:** decided direction. For defence platforms we adopt the STANDARD rather than drawing our own pictures.

## The delta, as asked

| | Mermaid | D2 | us today | needed |
|---|---|---|---|---|
| Generic shapes | ~15 | ~20 | 7, by kind | - |
| Diagram types | ~15 | many | 3 (structure, process, composition) | sequence, state, parametric |
| Icon sets | via Iconify (**fetched at runtime**) | icon packs (**fetched**) | none bundled | - |
| **Military symbology** | **none** | **none** | **none** | **MIL-STD-2525 / APP-6** |

Neither notable diagram library has military symbology, and both fetch their icon sets at runtime - which an air-gapped site cannot do. So this is not a "which renderer do we adopt" question; it is **which standard do we implement**.

## The standard

**MIL-STD-2525** (US) and **APP-6 / STANAG** (NATO) define symbols for exactly what a defence programme needs: land equipment (**tanks**, vehicles, artillery), sea surface and subsurface (**ships**, submarines), air (**aircraft**, UAVs), space, installations, and activities. They are built from a **frame** (affiliation: friend/hostile/neutral/unknown, and the dimension: land/sea/air) plus a **glyph** (the thing) plus **modifiers** (echelon, mobility, status).

A bespoke tank icon would be **non-compliant** and unusable in a real staff product. An organisation will require 2525/APP-6 by name. That settles it.

## The library

**`milsymbol` v3.0.4 - MIT** (spatialillusions). Pure JavaScript, no dependencies, and it turns a **SIDC** (Symbol Identification Code) into **deterministic SVG**. That satisfies this project's own decision rule exactly: *adopt a library when it computes something we can verify* - a SIDC to geometry is a pure function, not an ownership claim over the model-to-picture mapping.

MIT is compatible with the AGPL engine. The standard itself (2525, APP-6) is a public specification.

## Architecture - the same boundary as the layout

A `symbol(kind, stereotype, domain)` function returning SVG geometry, behind a thin module, so the renderer does not care where a shape came from:

- **Bundled**: the generic engineering shapes we already draw, plus the 2525 **frame** geometry and the glyph set for the dimensions a programme names.
- **The SIDC mapping is DECLARED, not guessed.** Which symbol a "frigate" or a "main battle tank" is, is domain knowledge the platform must not invent. A model declares it - a stereotype or an attribute - and the platform renders it. Guessing would be the same defect as a silent mapping.
- **Operator pack**: the site's own symbols for anything outside a standard, loaded by configuration, named in the diagram's provenance.

## Rulings

1. **STANDARDS BEFORE ICONS.** Where a standard exists, implement it; do not draw a house style. Cost of the alternative is a diagram that no staff process accepts.
2. **A STANDARD VERSION IS A BINDING.** 2525 has revisions (B/C/D/E) and APP-6 has editions; they differ. Like the XMI bindings, a version is a named, versioned adapter with its own conformance set - never "the standard" in the abstract.
3. **THE PLATFORM NEVER GUESSES A SIDC.** The model declares it; the renderer draws it.
4. **GEOMETRY IS PORTED WITH ATTRIBUTION, NOT FETCHED.** The engine is Rust and renders server-side; vendoring a JavaScript renderer would put the picture's determinism in the browser. Port the frame and glyph geometry we need, record `milsymbol` (MIT) in `NOTICE` as the reference implementation, and keep every node's `data-mw-id` bound to the model element.
5. **DETERMINISM AND COMPLETENESS STILL HOLD.** A symbol is a pure function of the declaration; the same model renders byte-identically, and every element is still drawn.

## Slices

| Slice | Deliverable |
|---|---|
| **S1** | The symbol boundary: `symbol(kind, stereotype, domain)` with the current shapes behind it, and a declared (not guessed) mapping from model elements to symbols |
| **S2** | The 2525/APP-6 frame and the platform glyphs a programme names (land, sea surface, subsurface, air), at a stated standard version, with a conformance test set |
| **S3** | Operator symbol packs, with provenance recorded on the diagram |
| **S4** | The declared-SIDC path: a model states its symbols and the diagram renders them |
