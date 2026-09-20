# mw-binding-sysmlv2

The SysML v2 TEXTUAL NOTATION binding: reads a stated subset of .sysml text into
OKF. It is a **viewer**, not a round-trippable binding: it imports but does not
export (Direction::ImportOnly). Part of the
[modelwrite](https://github.com/modelwrite/modelwrite) MBSE engine
(AGPL-3.0-or-later).

## Why a parser, not a vision model

SysML v2 textual notation is a text grammar with a published specification.
Parsing it is a parser, because a parser's losses are enumerable and a vision
model's are not. A vision or language model that "reads" .sysml and writes OKF
would be inference, not migration: nobody could say what was dropped, the loss
report could not name it, and the round-trip harness could not measure it. The
pixelrag/pixelshot image stack remains the right tool for what a parser cannot
do - scanned or photographed legacy documents with no machine-readable form -
and that is noted here as a FUTURE path flagged as extraction-with-review, not
as a binding. It is out of scope for this crate.

## The subset

The reader is a hand-written recursive-descent parser (no parser-generator
dependency, keeping the engine's rust-version 1.75 floor untouched). It carries:

- **package** - the outermost package name becomes the OKF project; nested
  packages are flattened (members promoted, each package named as a Lossy drop
  because OKF has no namespace concept).
- **part def / attribute def / item def** (with abstract and individual
  modifiers) - carried as a block (kind "block") whose stereotype names the
  SysML v2 keyword; abstract/individual fold into the stereotype.
- **specialization** (:> or specializes <base>) - carried as a
  "generalization" graph edge from the child to the base.
- **features** inside a definition - attribute, part, item, ref (and ref item):
  carried as an OKF Attribute; part/item add a "part" edge and ref a
  "reference" edge to the feature's type. Type multiplicity [..] is folded into
  the type string; the direction in/out is dropped and NAMED (OKF Attribute has
  no direction slot).
- **doc** comments - carried as the enclosing element's documentation.
- **requirement** with an id and name - carried as a requirement (name, reqId,
  documentation); subject members become "subject" graph edges to the subject's
  type, and assert constraint { ... } is carried as an attribute whose default
  holds the constraint text.
- **satisfy <req> by <subject>** - carried as a "dependency" graph edge
  labelled "satisfy" from the subject to the requirement.
- **import** - a declaration: recognised and deliberately not carried (it
  carries no model content).
- **line (//) and block (/* */) comments** - trivia, skipped.

## The boundary (what is OUTSIDE the subset)

Everything else is reported on import as a named Unmappable loss - never a
silent drop and never a panic. That includes:

- calc def / calc (calculations)
- state def / state / transition / action def (state machines)
- actor, usecase, association, boundary, include (use-case models)
- enum def / enum, variation / variant, timeslice
- SYSMOD/custom #... extensions (e.g. #system, #systemObjective, #derivation)
- top-level part/attribute usages (a "part x : T" owned by a package, not a
  definition)
- :>> redefinition bindings and feature redefinitions (their semantics are not
  reconstructed)

A malformed document (non-UTF-8, an unterminated comment or block, a package
without a name) is a clean BindingError::Import, never a panic.

## The mapping table is declaration, not gate input

mapping_table() is the published boundary, one row per construct marked
exact/lossy/unmappable. It is not read by the gate; the gate enforces the
per-import LossReport, whose entries name what was actually lost or left
unmapped at instance granularity. The two share the leading construct name.

## Conformance corpus and the measured result

The fixtures in fixtures/ pin the subset boundary (committed, no network). The
real corpus is the bundled example set at sample/examples/sysml-v2/ (the GfSE
community models, the MBSE4U Batmobile, the Airbus Apollo 11 mission). The
real-example tests import the two smallest models:

- Drone_BaseArchitecture.sysml (1514 bytes): 1 block, 4 requirements, 1 subject
  edge; 6 content losses named (the top-level "part drone" usage, the
  #derivation extension, three :>> subject redefinitions to non-carried usages,
  and the "satisfy ... by drone" whose subject is that usage).
- InternetModel_v1.sysml (1207 bytes): 9 blocks (5 part def, 4 attribute def),
  5 generalization edges, 1 doc comment; 0 content losses, 2 lossy drops (the
  in/out directions on the flow items).

Run the measurement with:

    cargo test -p mw-binding-sysmlv2 --test real_examples -- --nocapture

## Direction

ImportOnly. The notation is large and this is a stated first subset; a binding
that cannot export is a viewer and says so in its metadata rather than pretend.
The round-trip harness refuses it outright (BindingError::Viewer).
