# Thirty Meter Telescope - XMI extraction

The TMT model is published as a CATIA Magic (Cameo) project, a `.mdzip` ZIP
container whose `com.nomagic.magicdraw.uml_model.model` entry holds the UML/SysML
XMI document the XMI reader consumes.

## Source

- Repository: https://github.com/Open-MBEE/TMT-SysML-Model
- Commit: b2a33b71c65678498b538195dcb947a7962ca75b
- Licence: Apache-2.0 (LICENSE) plus a Caltech BSD-style COPYRIGHT - both kept
  beside this file.

## What is bundled

- `tmt-2022x.model.xmi` - the extracted `com.nomagic.magicdraw.uml_model.model`
  entry of `TMT.mdzip` (the 2022x Cameo export), 36,158,244 bytes, 255,402 lines.
- `LICENSE`, `COPYRIGHT` - upstream licence files, kept verbatim.
- `README.md` - the upstream repository README.

Not bundled: the ~27 MB `TMT.mdzip` container (its XMI is extracted here), the
`TMT-2024x.mdzip` (2024x), the ~22 MB `EA/TMT.qeax` Enterprise Architect export,
and the ~268 MB `Presentations/` folder. Fetch them from upstream if needed.

## How to re-extract

    git clone --depth 1 --filter=blob:none --sparse https://github.com/Open-MBEE/TMT-SysML-Model.git
    cd TMT-SysML-Model
    git sparse-checkout set TMT.mdzip LICENSE COPYRIGHT README.md
    # extract com.nomagic.magicdraw.uml_model.model from TMT.mdzip (any ZIP tool)
    # place it at tmt-2022x.model.xmi

The measurement that consumes this file is `engine/binding-xmi/tests/real_tmt.rs`.
