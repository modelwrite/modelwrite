# Concepts

Seven ideas, each in one short paragraph with the why. Read this once, then use
[tasks](tasks.md) for the clicks.

## OKF: one document, one truth

OKF, the Open Knowledge Format, is a single self-describing JSON document that holds a
systems model: its elements, requirements, interfaces, signals, a state machine,
activities, and a graph of every relationship. One file drives every consumer: the
workbench, analysis, AI, and the gate. The format is version-neutral on purpose. It
describes the things every MBSE standard has always described, and says nothing about which
standard described them.

Why it matters: the model, not a tool, becomes the thing you keep. A 2009 model and a 2026
model land in the same format and are queried together.

## A standard is a binding, not the centre

A standard version (SysML v1, SysML v2, ReqIF) is a binding: a versioned translator with an
identity such as sysml-v1-xmi@2.4, a conformance corpus, and a fidelity record. The binding
maps the source standard to OKF and back, and its mapping table declares every construct
exact, lossy, or unmappable. A new standard version is a new binding beside the old one,
never a rewrite of the core.

Why it matters: standards move. When the model is the constant and the standard is an
adapter, a version bump is a decision, not a migration project.

## A commit, and content addressing

Every model is a commit. The OKF document is stored as bytes addressed by the sha256 of
those bytes, and the commit records that address plus its parents, author, message, and
provenance. The same bytes always hash to the same address.

Why it matters: a hash commits to exact content without revealing it. The original artifact
of a migration is retained byte for byte under its own address, forever, so "what did we
lose" has a deterministic answer.

## A version is a branch

A version is a branch, and its identity is its tip commit. Making a version names a branch
and a commit to start from. Making a version current merges that branch into the main line,
after the gate has run.

Why it matters: you do not copy a model to make a variant. You branch, edit, compare, and
merge, and every one of those is a diffable, attributed commit.

## The gate: fidelity, coverage, integration

The gate compares a reference model to a candidate and returns a verdict plus a
deterministic evidence record. It checks three things, and it validates the candidate.
Fidelity: the round-trip diff, with every missing or extra element, edge, or changed
attribute named. Coverage: every requirement either covered or named as uncovered.
Integration: the graph is one connected component with no isolated nodes. The evidence
reproduces byte for byte from the same inputs.

Why it matters: the gate is the only authority on whether a change is correct. No tool
parameter, log line, or model output can mark a run as passed.

## Provenance: how a commit was made

Every commit records how it was produced: authored (a normal commit or edit), imported (a
migration through a binding, naming the retained artifact and the accepted losses), accepted
(an agent proposed and a human accepted, naming both), or unknown (a commit written before
provenance existed). Absence is never upgraded to a claim.

Why it matters: a reader a year later can tell whether a machine drafted a change and a
person decided it, and which person.

## Composition: reference, never copy

A platform model composes subsystems by reference. Each reference names a project, a pinned
revision, and a role. It never copies the subsystem's graph into the platform document. A
revision is pinned, so the reference cannot silently move when the subsystem does.

Why it matters: copying forks the truth. Referencing keeps one source of truth, and changing
an integrated revision is a deliberate, diffable commit.

## Analyses and findings

An analysis is a record, not a side effect. A run is stored against the revision it
examined, with its definition, its findings, and its evidence. The gate is already an
analysis, and checks are already attached to a commit. The rule that governs all of it: the
engine measures, an LLM may narrate, and narration is labelled as interpretation beside the
measured findings, never instead of them.

Why it matters: a finding is reproducible and checkable, which is the whole point of the
platform.
