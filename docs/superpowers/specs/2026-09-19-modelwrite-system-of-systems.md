# System of systems — how complex programmes compose

**Status:** specification, written 2026-09-19. It absorbs and replaces the design note of the same
date, and extends docs/superpowers/specs/2026-09-17-modelwrite-platform-design.md. It governs the
composition workstream, whose slices are ordered in §6.

**House rule.** A claim in this document must be checkable or labelled unchecked. Where the platform
can prove something, this spec says so with the proof; where it cannot, the spec says so in the same
breath. The compositional gate carries this rule as data — measured vs asserted, §4 — not as prose.

## 1. The problem, stated honestly

A project holds ONE model — one OkfRoot, one graph — with its own commits, branches, gate runs and
provenance. Several projects can be queried together by the analytics slice (which products meet
which specifications), but that is a READ-ONLY report over independent projects.

What a defence programme needs is not a report. It needs to COMPOSE: a platform built out of
subsystems, each owned by a different team, each at a specific revision, with the composition itself
as provable as the single model is. Today there is no primitive for that. Specifically, none of these
exist:

1. A level ABOVE the project — a programme, a platform, a mission — that has its own versioned, gated
   identity.
2. A reference from one model to a model element in ANOTHER project at a pinned revision. Today an
   edge can only point within one model's graph.
3. Cross-model traceability: a requirement held in the platform model, satisfied by an element inside
   a subsystem model owned by a different team.
4. A compositional gate: proving that the set of subsystems, at their pinned revisions, actually
   covers the platform's requirements — the same proof the gate gives inside one model, raised one
   level.

Why this cannot be patched in: the single-model graph is what makes the gate provable. Round-trip
fidelity, zero isolated nodes, coverage — all are computed over one document. Composition that shares
edges across documents would break every one of those proofs unless the composition itself is a
first-class, content-addressed thing.

## 2. Rulings

Four decisions bind the whole design. Each is stated with its benefit and its cost if wrong; they are
not preferences.

**R1 — Reference, never copy.** A platform model holds a subsystem by reference — project, pinned
revision, role — never by copying the subsystem's graph into the platform document. Copying forks the
truth; referencing keeps one.

- Benefit: one source of truth. A subsystem change is made once, and a platform picks it up only when
  it re-pins — a deliberate, diffable act, never a silent one.
- Cost if wrong: a copied subsystem drifts from its real state; the platform's gate proves things
  about a copy that no longer matches what the team owns, and a change in the subsystem is invisible
  to the platform until someone re-syncs — staleness presented as currency.

**R2 — A pinned revision is part of the reference's identity.** Two references to the same project at
different revisions are two different references. The revision is not metadata that can default to
"latest" or be allowed to drift.

- Benefit: "which revision does this platform integrate?" has one deterministic answer, and that
  answer is diffable.
- Cost if wrong: a reference whose revision can move means the platform model's meaning changes
  without a commit — the one defect this platform has spent its life catching: a claim that outran its
  evidence.

**R3 — A redaction never alters a hash.** Redaction is applied to presented content only. A reference
always binds to the full, unredacted, content-addressed revision; the hash travels, the content does
not.

- Benefit: an export-controlled subsystem can be referenced from an unclassified model, and the proof
  (the hash, the gate record) survives while the content stays behind the boundary.
- Cost if wrong: if redaction changed the hash, the reference would no longer point at the integrated
  revision, and every downstream proof — resolution, was-gated — would silently detach from the real
  content.

**R4 — The compositional gate labels measured vs asserted.** Every gate finding states whether it was
measured (computed over content the engine holds) or asserted (a claim the platform records that the
engine does not verify).

- Benefit: the platform never presents the global system-of-systems graph property as proved when it
  is not.
- Cost if wrong: without the label, a compositional report inherits the authority of the single-model
  gate without its proof — the same category error the house rule exists to prevent.

## 3. The primitives

### 3.1 The programme level is a project

A platform model is a project like any other: its own OkfRoot, its own graph, its own commits,
branches, gate runs and provenance. The level ABOVE the project is not a new storage engine; it is a
new KIND OF CONTENT a project may hold — typed references to other projects at pinned revisions. A
platform model inherits all of the versioning, diffing and audit machinery for free, and adds one
capability: composition.

This matters because a platform model is itself gateable, diffable and attributable with the tools the
organisation already has — including being the TARGET of another platform's reference (platforms
integrating platforms, §5.1).

### 3.2 The typed subsystem reference

A subsystem reference has three parts, all required:

| Field | Meaning |
|---|---|
| `project` | The identity of the subsystem project. |
| `revision` | The pinned commit hash of the subsystem model integrated by this platform — the content address, not a branch name and not "latest". |
| `role` | The role the subsystem plays in the platform, e.g. `combat-system`, `radar`, `propulsion`, `power`. The role is model vocabulary (a typed label or reference element), not free prose. |

A reference MAY also name specific elements within the pinned revision that the platform binds to —
the interfaces or blocks the platform connects to — each named by element id within that revision.
These are the cross-model traceability edges: a platform requirement is satisfied by an element inside
a subsystem, and the reference is the typed edge that closes the coverage.

A reference is ordinary model content. It lives in the platform model's document, so it is diffed,
gated and versioned exactly like any other element. A reference is not a live link: it names a
revision that has already been committed, and it cannot silently change when the subsystem moves on
(§7).

### 3.3 How a platform model declares its integration

The platform model's document carries a set of reference entries — one per integrated subsystem. Each
entry names the project, the pinned revision, the role, and (optionally) the bound elements. Changing
the integrated revision of a subsystem is a commit to the platform model that edits the `revision`
field of that entry.

### 3.4 How two platforms differ — visibly and diffably

Two platform models that integrate different revisions of the same subsystem differ in the
`revision` field of that reference. Because the reference is ordinary content:

- The difference is VISIBLE: a one-line attribute change in the platform model's document, named as
  "platform A integrates radar@r1; platform B integrates radar@r2".
- The difference is DIFFABLE twice over. First, the platform models diff at the reference: which
  subsystems, which roles, which revisions changed. Second, the content is diffable: because both
  revisions are content-addressed and stored, the engine can diff `radar@r1` against `radar@r2` and
  show exactly what changed in the integrated behaviour — not merely that a hash changed.

This is the configuration-management payoff: a fleet's divergence is data you can read and diff, not a
state you have to infer from which copy is newer.

## 4. The compositional gate, and its honest boundary

The compositional gate is the single-model gate lifted one level. It runs over the platform model's
own document, exactly as the single-model gate runs over a project's document, plus a bounded set of
cross-model checks. It states its boundary the way the analytics slice states confidence: measured vs
asserted.

### 4.1 What is measured — provable without importing

Three things are proved, and none of them requires importing a subsystem's full graph into the
platform document:

1. **Resolution.** Every subsystem reference resolves: the named project exists, the pinned revision
   exists, and every bound element exists within that revision. This is a lookup against the
   subsystem's stored, content-addressed revisions.
2. **Every integrated revision was itself gated.** A subsystem revision carries its gate evidence —
   the gate run that passed for that commit. The compositional gate checks that the pinned revision
   has a passing gate record. It is a lookup of the record, not a re-run of the subsystem's gate.
3. **Platform coverage, computed locally.** Within the platform model's own graph, every platform
   requirement has either (a) a satisfying element in the platform model itself, or (b) a typed
   cross-model reference to a satisfying element in a subsystem at its pinned revision. Coverage is
   computed over the platform's graph, where a reference is the edge that closes the trace; the
   single-model coverage computation is unchanged.

### 4.2 What is asserted — and must say so

What the gate CANNOT prove without importing every subsystem into one document is the GLOBAL graph
property: that the union of the platform and all its subsystems, transitively, at their pinned
revisions, forms one connected, coverage-complete, orphan-free graph — the same health the single-model
gate proves for one model. Proving it requires materialising the union and running the gate over it,
which is the copy R1 forbids.

So the compositional gate states, in the same breath:

- Measured: resolution, was-gated, platform coverage — all computed over content the engine holds.
- Asserted: the combined system-of-systems, as a whole, has the global graph property. The platform
  records this as an assertion, labelled as such, and never presents it as a measured proof.

The label is data, not prose. A gate finding carries a `kind` of `measured` or `asserted`, so a
report cannot blur a proof and a claim the way a naive column of green ticks would.

## 5. Defence concepts

### 5.1 Platforms integrating platforms

A platform model's reference can point at another platform model, because a platform model is a
project (§3.1). A ship (platform) integrates a combat system (platform) which integrates a radar
(project). Nothing in the reference type cares whether the target is a leaf project or a platform —
composition is references all the way down.

The gate carries this at the was-gated step: the combat-system platform, at its pinned revision, was
itself gated — which in turn checked the radar reference at ITS pinned revision. The gate proves
was-gated at each level it is asked to check; it does not flatten the whole tree into one graph, and
the global property across the nesting remains the assertion of §4.2, not a measurement.

### 5.2 Security-boundary annotations

A reference carries an optional `boundary` annotation. When a reference crosses a security (enclave)
boundary, it says so: `boundary: security`. A security case reads differently from a safety case, and
the annotation is what lets each read correctly:

- A SAFETY case asks of a reference: does this dependency hold under failure? It wants the reference
  to resolve, the revision to be gated, the coverage to be real.
- A SECURITY case asks of the same reference: does data flow across this boundary, and is it
  authorised? It wants the boundary marked, and it wants to know what travels.

The annotation is model data, diffed and gated like any other field. The platform annotates the
boundary; it does NOT evaluate the security case. No security-model claim is made beyond the
annotation (§7).

### 5.3 Export-controlled subsystems: content redacted, hash intact

An export-controlled subsystem — a crypto, a weapon component — is referenced by an unclassified
platform model. The reference names the project, the pinned revision and the role; for the
unclassified reader, the referenced CONTENT is redacted (not shipped, not readable). The HASH is not
redacted.

Why the proof survives while the content does not travel: the hash commits to the exact content
without revealing it. The resolution check and the was-gated check can be answered — a yes/no plus a
hash comparison — where the content is held, and the unclassified side still sees that a specific,
named, gated revision is integrated, without that revision's content crossing the boundary.

This is R3 in operation: redaction is a presentation and access-control layer over the content; the
reference binds to the full content-addressed revision. If redaction altered the hash, the reference
would no longer resolve to the integrated revision, and the proof would detach from the real content.

### 5.4 Fleet baselines

A fleet (or class) is a set of platform models: a class baseline plus one platform model per hull (or
per-hull deltas against the baseline). Each hull's platform model is a project, so it is versioned,
gated and diffable. Because hulls share the reference primitive, divergence is visible:

- Hull A integrates radar@r1; hull B integrates radar@r2. The diff between hull A's and hull B's
  platform models shows the reference-revision difference as a named, one-line change — plus any local
  differences, separately.
- A class baseline is a platform model the per-hull models are derived from or reference; a hull that
  diverges from the baseline is a diff, not a mystery.

The fleet baseline is configuration management of a defence platform, with the same machinery as any
other model: pinned, diffable, attributable.

## 6. Delivery slices

The composition workstream orders its work: the platform-model primitive before the gate before the
defence concepts. (Slice numbers are internal to the workstream; the master roadmap assigns the final
number when the workstream enters it.)

| Slice | Capability | The organisation can now |
|---|---|---|
| S0 | The platform-model primitive: the programme level as a project; the typed subsystem reference (project + pinned revision + role); platform models that declare, pin and re-pin each subsystem's revision; visible, diffable divergence between platform models | Declare what a platform integrates, at which revision, and diff two platforms exactly |
| S1 | The compositional gate: resolution at the pinned revision; the was-gated check; platform coverage via satisfying element or typed cross-model reference; measured vs asserted labelling | Gate a platform model and see precisely what is proved and what is asserted |
| S2 | Defence concepts I: platforms integrating platforms; security-boundary annotations on references (security vs safety case) | Model a platform of platforms and mark which references cross an enclave boundary |
| S3 | Defence concepts II: redaction-preserving references (content redacted, hash intact) and fleet baselines (a platform model per hull, diffable) | Reference an export-controlled subsystem without moving its content, and baseline a fleet hull by hull |

S0 is the precondition for everything after it: without the typed, pinned reference there is nothing
for the gate to check or for the defence concepts to annotate. S1 is the precondition for the defence
concepts being trusted: a boundary annotation or a redaction is meaningful only on top of a reference
whose resolution and gating are already proved. S2 and S3 land after the primitive and the gate
because they are annotations and presentations OF the reference, not new composition machinery.

## 7. What this spec deliberately does NOT do

- **No distributed editing.** This spec adds no cross-project live authoring, no shared mutable links,
  no multi-project transaction. A platform model is edited like any other project; a subsystem is
  edited in its own project. Composition is a declared, committed, pinned relationship — not a shared
  workspace.
- **No live cross-model linking that could silently change a pinned revision.** A reference names a
  revision that has already been committed. There is no "latest", no auto-follow, no background refresh
  that re-points a platform at a newer subsystem revision. Moving a platform to a newer revision is an
  explicit commit, diffed and attributed like any change.
- **No security-model claims beyond annotation.** The platform marks that a reference crosses a
  security boundary; it does not evaluate the security case, does not enforce classification or
  clearance, and does not certify that a composition is secure. Security is the thing being annotated,
  not the thing being decided here.
- **No import-as-proof of the global graph.** The platform does not flatten a system of systems into
  one document and present the union's gate run as the composition's proof, because that is the copy
  R1 forbids and the over-claim R4 forbids.
- **No replacement of the analytics slice.** Portfolio compliance across projects remains the analytics
  slice's read-only report. This workstream adds composition, not a second way to query independent
  projects.

## 8. What the platform honestly offers, today and after

Today, what the platform honestly offers a defence programme is: each subsystem as a proved, versioned
model; portfolio compliance across them; and no composition. Saying anything stronger would repeat the
one defect this project has spent its whole life catching: a claim that outran its evidence.

After this workstream, it offers the same — plus composition at the only level that can be honest:
typed references at pinned revisions, a compositional gate that says exactly what it measured and what
it asserted, and defence concepts that annotate and redact without ever pretending the annotation is
the proof.

## 9. Open decisions

- The exact name and shape of the `role` vocabulary (an open enum vs a closed per-domain set), and
  whether roles are defined in the platform model or in a shared library.
- Whether the was-gated check (§4.1.2) is required or advisory when the subsystem's gate record is
  held behind an export boundary (where the record may be a hash attestation rather than a readable
  report).
- Whether per-hull models are full platform models or deltas against a class baseline. The working
  assumption is full platform models with the baseline as a shared reference, because a full model
  keeps every hull independently gateable.
- How far the was-gated check recurses through nested platforms (§5.1) before it stops and labels the
  remainder asserted.
