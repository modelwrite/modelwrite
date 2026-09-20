# NOTICE

Modelwrite is a community project. The engine, portal, judge harness, and
sample data are contributions of the modelwrite contributors.

The coffee-machine corpus under sample/corpus/coffee-machine/ is based on
course work by Alex Kovaceski in the MEMKO MBSEF-SEng course (September
2026): a SysML model built in CATIA Magic, its OKF export, and the analysis
portal described in the knowledge pack. The original material is shared as
the starting corpus for this project with the permission of the author.

The name modelwrite and the modelwrite.org domain are used in this notice
as the project identity.

## Diagram symbology

The diagram glyphs and MIL-STD-2525/APP-6 frames in engine/graph/src/symbol.rs
are hand-drawn SVG path data authored for this project. No third-party icon or
symbol geometry is vendored: there is no copied path data from any icon set in
the repository.

milsymbol v3.0.4 (MIT, spatialillusions) is cited as the reference
implementation for MIL-STD-2525/APP-6 frame geometry - the frame shapes
(rectangle / diamond / square / unknown "clover" for ground, the arch / peaked /
rectangle for air, the orbit oval for space) were checked against its output,
but the paths here were drawn by hand against the standard, not copied. The
standard implemented is MIL-STD-2525D / APP-6D (numeric 20-character SIDC).

milsymbol: https://github.com/spatialillusions/milsymbol - MIT License.
