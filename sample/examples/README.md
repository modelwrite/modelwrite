# Open MBSE example corpus

Permissively-licensed MBSE example models, bundled for the modelwrite engine
(AGPL-3.0-or-later). Each example keeps its upstream LICENSE/COPYRIGHT file
beside it; the table below records the source repo, commit and licence.

The rule that decides what lives here and what does not:

- **Bundle-safe** (Apache-2.0, BSD-3-Clause, MPL-2.0) - copied into this
  repository. These licences are compatible with an AGPL-3.0-or-later engine.
- **Not bundle-safe** (EPL-2.0) - NOT copied. EPL-2.0 is not compatible with an
  AGPL engine, so the platform ships only the *fetch script* and the operator
  runs it locally - the same principle as operator-supplied symbol packs: we
  ship the mechanism, never the restricted content.

Fetch the non-bundle-safe examples (and the large bundle-safe ones) with:

    pwsh ./Get-OpenMbseExamples.ps1 -Dest <path>

## Bundled examples

| Example | Upstream repo | Commit | Licence | Size | What it demonstrates |
|---|---|---|---|---|---|
| sysml-v1/openmbee-tmt | Open-MBEE/TMT-SysML-Model | b2a33b7 | Apache-2.0 (LICENSE) + Caltech BSD-style (COPYRIGHT) | 36 MB (extracted XMI) | The Thirty Meter Telescope: a **real Cameo/MagicDraw SysML v1 export**. This is the content-rich vendor model the XMI reader is measured against (engine/binding-xmi/tests/real_tmt.rs). |
| sysml-v1/gaphor-examples | gaphor/gaphor (examples only) | 968f6bb | Apache-2.0 | 665 KB | Small single-concept SysML v1 models, including the canonical Gaphor coffee machine. **Gaphor .gaphor is Gaphor's own XML format, not XMI - not importable by the current reader.** |
| sysml-v2/gfse-models | GfSE/SysML-v2-Models | ebbb0c3 | BSD-3-Clause | 198 KB (models only) | GfSE community SysML v2 models: system-of-systems, a family model, an Eve-Online mining frigate, SE_Models. Textual .sysml. |
| sysml-v2/mbse4u-batmobile | MBSE4U/dont-panic-batmobile | f6cdf35 | Apache-2.0 | 35 KB | The Batmobile from the 'Don't Panic' SysML v2 introduction. Textual .sysml + notebook. |
| sysml-v2/airbus-apollo-11 | airbus/apollo-11-sysml-v2 | 6e9c93f | MPL-2.0 | 426 KB | The Apollo 11 mission, a layered SysML v2 model (Purpose, Function, Logical, Requirements, ...). Textual .sysml. |

## Not bundled (fetch with the script)

| Example | Upstream repo | Commit | Licence | Why |
|---|---|---|---|---|
| sysml-v2/omg-sysml-v2-release | Systems-Modeling/SysML-v2-Release (sysml/src, kerml/src) | fb97b75 | EPL-2.0 | Not AGPL-compatible. |
| arcadia-capella/eclipse-capella-samples | eclipse-capella/capella (samples only) | c5a3a5c | EPL-2.0 | Not AGPL-compatible. |
| arcadia-capella/dbinfrago-ife-variant | dbinfrago/Capella-IFE-sample | 739641d | EPL-2.0 (model) | Not AGPL-compatible. |
| sysml-v2/mbse4u-sysmlv2-book | MBSE4U/the-sysmlv2-book-examples | dae1f63 | Apache-2.0 | Bundle-safe but ~90 MB (Cameo .mdszip); kept out of git for size. |

## What is importable today, and what is not

- **SysML v1 XMI** - importable. The binding is mw-binding-xmi (engine/binding-xmi),
  and the real-vendor measurement is engine/binding-xmi/tests/real_tmt.rs (plus
  real_magicdraw.rs for the smaller coffee-machine corpus already in sample/corpus/).
- **SysML v2 (.sysml)** - textual notation, NOT XMI. Reading it needs a NEW
  binding (a SysML v2 textual-notation reader), which is its own tranche of work
  and is deliberately out of scope here. The .sysml files are bundled as corpus
  material for that FUTURE reader; they cannot be imported today.
- **Gaphor (.gaphor)** - Gaphor's own XML model format, NOT the OMG uml:/SysML XMI
  the reader understands. Bundled as corpus material; a Gaphor binding does not
  exist today.

## Thirty Meter Telescope - how the XMI was obtained

sysml-v1/openmbee-tmt/tmt-2022x.model.xmi is the
com.nomagic.magicdraw.uml_model.model entry extracted from TMT.mdzip (the 2022x
Cameo export). The .mdzip itself is ~27 MB and is not committed; see
sysml-v1/openmbee-tmt/EXTRACTION.md.

## Licence audit

The upstream licence audit these examples came from is the LICENCES.md that
accompanied the fetch (C:/Users/alexk/Downloads/SysML/LICENCES.md). Each example
directory keeps its upstream LICENSE/COPYRIGHT beside the model.
