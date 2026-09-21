# Capella fixtures

These documents are HAND-WRITTEN synthetic XMI that mirrors the real `.capella`
shape, verified against the fetched corpus (dbinfrago/Capella-IFE-sample and
eclipse-capella/capella samples). They exercise the reader in
engine/binding-capella and prove nothing about Capella's exporter. Real vendor
models need their own corpus and their own fidelity report - that is
tests/real_capella.rs, measured against the EPL-2.0 corpus the operator fetches
(NOT committed).

- control-structure.capella: the happy path - two SystemComponents (a controller
  and a controlled process), three SystemFunctions (a control action, its
  feedback and a nested sub-function), two FunctionalExchanges (the directed
  control flow and its feedback), two ComponentFunctionalAllocations, ports, and
  a Constraint with an OpaqueExpression body. The two packages are flattened and
  the dropped port/exchange/allocation ids are named.
- out-of-scope.capella: a carried function and component plus out-of-scope
  elements (TransfoLink, StateMachine, Part) to prove nothing is dropped in
  silence.
- malformed.capella: deliberately broken XML, to prove a malformed document is a
  clean error, never a panic.
- representation.aird: a Sirius .aird root (the diagram representation layer),
  to prove the reader rejects it with a clear message rather than mis-reading it.

## Conventions

- Element identity is the Capella `id` attribute (a UUID), carried verbatim as
  the OKF element id and graph node id. The ids of exchanges, allocations and
  ports have no OKF slot (GraphEdge/Attribute) and are named as Lossy drops.
- Element type is the local name of `xsi:type` ("SystemComponent",
  "FunctionalExchange", ...); the namespace is the Capella 7.0.0 metamodel.
- Cross-references are `#uuid` string attributes, not `xmi:idref`: exchanges
  use `source`/`target`, allocations use `sourceElement`/`targetElement`
  (sourceElement = component, targetElement = function).
- A component or function carries its Capella type as the OKF stereotype; both
  are kind "block" (OKF node kinds are a closed set with no
  SystemComponent/SystemFunction, so the Arcadia type is preserved as the
  stereotype).
- A Constraint's text is the `bodies` of its `ownedSpecification`
  OpaqueExpression, carried as the requirement's reqText.
