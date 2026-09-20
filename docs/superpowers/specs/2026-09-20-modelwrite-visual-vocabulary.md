# Visual vocabulary: shapes, icons and operator-supplied packs

## The ask

More than boxes and basic shapes, so a diagram reads like an engineering diagram rather than a graph.

## The answer, in priority order

### 1. The STANDARD SHAPES first - no library needed
Systems-engineering notation is a *shape* vocabulary before it is an icon vocabulary, and we already have the vocabulary in the model: every element declares its **kind** and its **stereotypes**. That is enough to draw:
- a **requirement** as the classic document shape with a `«requirement»` keyword and its reqId;
- a **block** as a rectangle with a name compartment;
- an **activity** as a rounded rectangle;
- a **signal** as a small flagged shape;
- an **actor** as a stick figure;
- an **interface** as a circle-and-line (the ball-and-socket convention);
- a **state** as a rounded rectangle, a **use case** as an ellipse.

This costs nothing, adds no licence, ships no asset, and is the notation an engineer already reads. **It is the highest-value step and it comes before any icon set.**

### 2. A BUNDLED ICON SET for kind glyphs - small, licensed, embedded
A handful of glyptic accents (a gear, a document, a lightning bolt, a person) makes kinds scannable at a distance, which matters on a projector. Rules:
- **Licence-compatible only.** The project is AGPL-3.0-or-later; icons must be MIT, ISC, Apache-2.0 or CC0 (Lucide/Feather and Bootstrap Icons are the usual choices). Every set used is recorded with its licence and version in a `NOTICE` file, and the licence text ships with it - the same discipline the code already follows.
- **EMBEDDED IN THE BINARY**, never fetched: SVG path data included at compile time, exactly as the workbench's own javascript is. No CDN, no network at load, no asset directory to deploy.
- **A small, explicit, OVERRIDABLE MAP** from (kind, stereotype) to glyph - data in the code, not scattered conditionals. An unknown kind gets a neutral default rather than nothing.
- **Icons augment, never replace.** Every element keeps its text label: never encode meaning in shape or colour alone.

### 3. OPERATOR-SUPPLIED PACKS for anything brand-shaped
An organisation will want its own symbols - a vendor's terminal, a radar, a specific pump, a company mark.
- **The product bundles NO third-party logos or trademarks.** Brand marks are protected regardless of the licence on the file, and an open-source project cannot ship a customer's or a vendor's mark. This is a legal boundary, not a preference.
- Instead the deployment may point at an **icon pack**: a directory of SVG files plus a small manifest mapping (kind or stereotype) to file. The platform loads it at startup and uses it exactly like the bundled set, with the pack's own name and version recorded in the diagram's provenance.
- An air-gapped site supplies its own files; nothing is fetched, ever.

## Rulings

1. **NOTATION BEFORE DECORATION.** Standard engineering shapes are the first step; icon sets are the second. A diagram using the correct shapes and no icons reads better than the reverse.
2. **NO TRADEMARKED MARK SHIPS IN THE BINARY.** Brand-shaped symbols are operator-supplied, always. *Cost if wrong:* a little convenience for the user. *Benefit:* the project stays legitimately distributable.
3. **EVERY ASSET IS LICENSED, RECORDED AND EMBEDDED.** A `NOTICE` file lists each set, its version and its licence; assets are compiled in; nothing is fetched at runtime.
4. **A GLYPH IS NEVER THE ONLY CARRIER OF MEANING.** Text stays; shape and colour augment. This is an accessibility rule and a correctness rule at once - the same reason the coverage chips carry a word as well as a colour.
5. **AN UNKNOWN KIND DEGRADES GRACEFULLY.** A neutral box with its label, never a missing glyph or a broken image.

## Acceptance criteria

- Diagrams render the standard shapes from kind and stereotype, with text labels intact.
- Any bundled icon set is licence-recorded in NOTICE, embedded in the binary, and needs no network.
- An operator icon pack can be supplied by configuration, is used identically to the bundled set, and its name and version appear in the diagram's provenance.
- Determinism holds: the same model and the same pack render byte-identically.
- No trademarked mark is present anywhere in the repository.
