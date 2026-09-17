# Standards interoperability and versioned migration

Status: design section, written 2026-09-17. It extends
docs/superpowers/specs/2026-09-17-modelwrite-platform-design.md and governs Slice 4 of the roadmap.

## 1. The problem, stated honestly

An organisation that has built models over twenty years has not built them in one standard. It has
XMI exports from a UML tool that stopped being supported, requirements in DOORS, simulation models
that only one person can run, and CAD structures whose semantics live in a vendor's proprietary
format. The standards have moved underneath all of it: SysML v1 is a UML profile interchanged as
XMI; SysML v2 is a different metamodel with a textual notation and an API; and the tools that read
one do not read the other.

The usual answer, "convert everything to the new standard", fails for three reasons, and each one is
a reason an organisation refuses to migrate:

1. **Conversion loses things silently.** XMI round trips between tools have never preserved diagram
   layout, and vendor dialects add semantics the standard never defined. A conversion that quietly
   drops the part of the model someone cares about is worse than no conversion.
2. **Conversion is irreversible in practice.** Once the original artifact is gone, the questions it
   answered are gone with it, and nobody can prove what was lost.
3. **The standards will change again.** A migration that has to be repeated by hand every time OMG
   revises a version is not a strategy, it is a recurring project.

So the requirement is not "support SysML v2". It is: **an organisation must be able to bring models
built under any standard, at any version, into one place, prove exactly what survived the journey,
keep the original forever, and never be forced to convert in order to participate.**

## 2. The principle: the model is the constant, standards are bindings

OKF is deliberately version-neutral. It describes blocks, requirements, interfaces, signals, state
machines, activities and their relationships - the things every MBSE standard has always described -
and says nothing about which standard described them. A standard version is therefore not the centre
of the system. It is a **binding**: a translator, with a version, a conformance corpus and a fidelity
record.

This single decision answers most of the version question:

| Question | Answer under a binding architecture |
|---|---|
| Which standard does modelwrite use? | OKF, which is not a standard version at all |
| What happens when SysML v3 arrives? | A new binding beside the others, not a rewrite |
| Can a 2009 model and a 2026 model live in one repository? | Yes, and they are queried together |
| Do we have to convert to take part? | No. A model is stored in its source binding and read through it |

## 3. What a binding is

A binding is a versioned adapter with obligations, not a best-effort importer:

- **Identity.** `bindingId` and `bindingVersion`, e.g. `sysml-v1-xmi@2.4`, `sysml-v2-api@1.0`,
  `reqif@1.2`, `ap242@2.0`, `fmi@3.0`, plus `toolVendor` and `toolVersion` where the dialect matters.
- **Two directions.** Import (source artifact to OKF) and export (OKF to source artifact). A binding
  that cannot export is a **viewer**, and is labelled as one so no one mistakes reading for migrating.
- **A conformance corpus.** Real artifacts, committed, with their expected OKF and their fidelity
  records. The corpus is the definition of what the binding means, exactly as the coffee-machine
  corpus defines the engine's round trip.
- **A mapping matrix.** Every element and attribute the source standard can express, marked
  `exact`, `lossy` or `unmappable`. This is data, not prose: the gate reads it.
- **A fidelity record per import.** Produced by the existing gate, in the existing evidence format,
  so a migration is proved with the same machinery as any other change.

## 4. Fidelity is proved against the source artifact

The gate already proves round-trip fidelity: no element, edge, attribute or document field lost.
For migration the **reference** is the original artifact, not an idealised standard:

- Import the source artifact to OKF, then export back to the source binding, and compare.
- Anything that does not survive is reported by name, with its value, before anyone commits anything.
- Losses are classified, not merely counted, because "the diagram layout moved" and "a requirement's
  verification method disappeared" are not the same kind of news.

This is why the provable gate matters more for migration than for authoring: **it is the only thing
that turns "we migrated the models" into a statement someone can audit.**

## 5. The version question, concretely

For SysML specifically, the two versions disagree structurally rather than cosmetically:

- v1 is a UML profile: element identity is a UML element id inside an XMI document whose meaning
  depends on the tool's profile implementation.
- v2 is its own metamodel with textual and API interchange, and no direct requirement to preserve
  v1 constructs that v2 redesigned.
- Diagram layout, colours, and tool-specific extension data are outside both standards and are the
  first casualties of any naive conversion.

The binding answers each of these with a decision rather than a hope:

1. **v1 models are stored as v1 models.** Import through `sysml-v1-xmi`; the source XMI is retained
   byte for byte, content-addressed, forever.
2. **A v1 model can be exported back to XMI**, which is how an organisation keeps using the tool it
   already pays for while the repository becomes the system of record.
3. **v1 to v2 is a migration, not a conversion.** It runs the mapping matrix, produces a
   loss-by-loss report, and requires an explicit decision for every `lossy` and `unmappable` entry.
4. **The v1 original is never deleted by the migration.** The migration is a new commit whose
   provenance names the source artifact and the binding versions on both sides.
5. **Non-standard semantics survive as binding extensions.** Colours, layout, vendor properties and
   diagram geometry are preserved as first-class, namespaced extension data attached to the elements
   they belong to - carried through every operation, exported only by bindings that understand them,
   and never silently dropped. This is what makes round trip through modelwrite better than a
   round trip between two tools.

## 6. The mapping matrix: exact, lossy, unmappable

Every migration between two binding versions is governed by a matrix with three verdicts per mapping:

- **exact** - the target expresses the same thing. Migration is mechanical.
- **lossy** - the target expresses something close, and the difference matters. Migration requires a
  recorded decision: accept the loss, or attach the original as an extension, or leave the element
  behind in the source model and reference it.
- **unmappable** - the target cannot express it at all. The element is preserved as an extension and
  the migration report says so in plain language.

The rule that makes this trustworthy: **no mapping is ever applied silently.** A migration that
loses a requirement's verification method produces a report entry naming the requirement and the
attribute, and the commit message records who accepted it. Silence is the failure mode this design
exists to prevent.

## 7. Migration is a gated, reviewable, non-destructive operation

A migration is not a file conversion. It is a change to a model, so it goes through exactly the same
machinery as any other change:

1. Import the source artifact as a commit on a migration branch, with the artifact retained.
2. Run the mapping matrix to produce the proposed migration plus its conflict-and-loss report.
3. A human or an agent resolves every non-exact entry. This is where the AI agent loop earns its
   place: reading a 4,000-element report and proposing decisions is exactly the work there are not
   enough engineers to do.
4. Run the gate in strict mode. Evidence is recorded, including the loss report.
5. Commit. The branch, the author, the binding versions and the accepted losses are all in the
   audit record.

Nothing about this step is special-cased, which means a migration can be reviewed, reverted,
attributed and audited with tools the organisation already has.

## 8. Standards evolution: a version bump is a binding bump

When a standard is revised, the response is fixed in advance:

1. A new binding version is created beside the old one. The old one is never mutated: a model
   validated against `sysml-v1-xmi@2.4` stays validated against it forever.
2. A **specification diff** is recorded: what the new version changed, as data, with a summary an
   engineer can read.
3. An **impact analysis** runs across the corpus: which models use constructs whose meaning changed.
4. Migration is a decision, not an event. Models move when their owner decides, and the repository
   can hold both versions indefinitely.
5. Every fidelity record names the binding version it was produced under, so a standards change
   never retroactively invalidates a historical proof - a property that matters enormously the first
   time someone asks "how do we know this model was correct in 2026?"

## 9. Cross-version analytics

Because every binding lands in the same canonical form, analytics do not care which standard a model
came from. "Which products meet the specifications", "which requirements are most expensive", and
"where are the coverage gaps" are answered across a corpus that mixes twenty years and three
standards, and every answer can be traced back through provenance to the source artifact and the
fidelity record that admitted it.

This is the payoff for refusing to make any one standard version the centre. The organisation's
knowledge becomes queryable as one thing, while each model keeps its own identity and its own proof.

## 10. What we will and will not promise

We will promise:

- No silent loss: every loss is named before it is committed, with evidence.
- No destructive migration: the source artifact is retained and addressable forever.
- No forced conversion: a model in an old binding is a full participant, not a second-class citizen.
- No retroactive invalidation: fidelity records are bound to the versions that produced them.
- No standard lock-in at the core: a new version is a new adapter, never a rewrite.

We will not promise:

- That a conversion is lossless. It usually is not, and the point of measuring is to say exactly how.
- That we understand every vendor dialect on day one. Coverage grows per binding, and each binding
  states honestly what it does not yet handle rather than guessing.
- That diagram geometry survives every path. It survives through OKF extensions, but a tool that
  cannot read extensions will not show it.

## 11. Binding roadmap, in order, with reasons

1. **sysml-v1-xmi** - the largest installed base, and the format most likely to be stranded. Diagrams
   and vendor extensions are the known hard part, so this binding is the one that proves the
   extension strategy.
2. **sysml-v2-api and textual** - the destination standard, and the natural authoring format for new
   work.
3. **reqif** - requirements live outside models in most organisations, and often in the oldest tool
   in the building.
4. **ap242 and ap233** - supplier and cross-organisation exchange, where the customer dictates the
   format.
5. **fmi** - simulation and verification artefacts, so "verified" can be linked to a runnable model.
6. **Vendor and CAD bindings** - including the CATIA structures the field guide describes, each with
   its own corpus and its stated limits.

Order is by installed base multiplied by risk of loss, not by fashion.

## 12. Open decisions

- Which vendor dialects to support first, and whether a dialect is a distinct binding or a profile of
  a standard binding. The working assumption is a profile, so `sysml-v1-xmi@2.4+cameo` is one binding
  with dialect rules.
- Whether extension data is stored inline on elements or in a sidecar partition. Inline is simpler to
  carry through merges; sidecar is cleaner for formats that produce enormous layout data.
- How far the migration agent may go without a human. The working assumption is: it may propose
  every non-exact decision and must record them, but a human approves the commit, until a corpus of
  its decisions has been reviewed and shown to be sound.
