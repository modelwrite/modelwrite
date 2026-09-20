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

---

# Appendix: other domains, and the licence classes that decide everything

The question "which symbol libraries should we adopt for networking, cloud, IT, electrical, fluid dynamics?" has the same answer in every domain, because **the deciding factor is never the domain, it is the licence class**.

## The three classes

### Class A - PERMISSIVE OR PUBLIC DOMAIN: bundle it
Safe to compile into the binary with attribution. Examples found:
- **MIL-STD-2525 / APP-6** via `milsymbol` (**MIT**) - defence platforms: tanks, ships, aircraft.
- **Electrical symbols** - `basverdoes/ElectricalSymbolLibrary` (**public domain**, SVG).
- General UI glyphs: **Lucide (ISC)**, **Feather (MIT)**, **Tabler (MIT)**, **Bootstrap Icons (MIT)**, **Material Symbols (Apache-2.0)**.
- **P&ID symbol sets** - at least one open set exists (BVLtd, American National Standard); its licence must be VERIFIED per pack before use, which is exactly why a pack declares its licence.

### Class B - COPYLEFT: bundle only knowingly
Compatible with an AGPL-3.0-or-later engine when the licence is GPL-3.0-or-later or CC-BY-SA **and** attribution and share-alike are honoured for the pack itself:
- **KiCad** symbol libraries (**CC-BY-SA-4.0**).
- **QElectroTech** element collections (GPL-family).
Rule: the pack keeps its own licence, is attributed, and is not silently relicensed. A pack that cannot satisfy that does not ship.

### Class C - PROPRIETARY OR TRADEMARKED: never bundle, always operator-supplied
This is the largest class by far, and it is where most of the domains the question named actually live:
- **IEC 60617** (electrical graphical symbols), **ISO 10628** and **ISA-5.1** (P&ID and instrumentation), **ASME Y14.5** (GD&T), **ISO 128** - **standards bodies sell these publications.** The symbols are the copyrighted content. Implementing to a standard you have licensed is normal engineering; redistributing its artwork is not.
- **Cisco network icons**, **AWS / Azure / Google Cloud architecture icons** - trademarked vendor marks whose terms restrict redistribution. An organisation that uses them has its own entitlement; the platform has none.
- **Any company's own mark.**

## So the architecture is one mechanism, not seven libraries

**A symbology PACK**, which is the same shape as the interoperability binding already built for legacy XMI:

- a pack **declares** the standard it implements **and its version** (2525E, IEC 60617, ISA-5.1, "our house symbols"), because a standard version is a binding and they differ between revisions;
- a pack **declares its licence**, ships with its `NOTICE`, and is either bundled (Class A/B) or **supplied by the deployment** (Class C);
- a pack **maps model declarations to symbols** - and the platform **never guesses**: a model says what a "frigate", a "pump" or a "VPC" is; the pack draws it;
- a pack that cannot draw something reports it as an **unmappable**, never a silent fallback to a box.

That last point is the whole reason this is safe: the same discipline as the loss report. **A diagram must never imply a symbol it did not draw.**

## What this means practically

| Domain | Approach |
|---|---|
| Defence platforms | Bundle 2525/APP-6 geometry (MIT reference); operator adds house symbols |
| Electrical | Bundle a public-domain set; operator may supply an IEC-60617 pack they are licensed for |
| Process / P&ID / fluid | Bundle nothing by default; the operator supplies the ISA-5.1 or ISO 10628 pack their licence covers |
| Networking / cloud / IT | **Bundle nothing**; the operator supplies vendor icons they are entitled to use |
| Engineering drawing | Conventions (not artwork) are implementable: line types, hatching, dimension style |

## Ruling

**THE PLATFORM BUNDLES ONLY WHAT IT MAY REDISTRIBUTE, AND SHIPS THE MECHANISM FOR EVERYTHING ELSE.** Cost if wrong: a user must supply one directory for their licensed standards. Benefit: the project stays legitimately distributable, and an air-gapped defence site can use its own controlled symbol library without asking anyone's permission.
