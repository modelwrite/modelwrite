# mw-binding-capella

The Capella / Arcadia binding: reads a stated subset of a Capella `.capella`
model into OKF. It is a **viewer**, not a round-trippable binding: it imports but
does not export (Direction::ImportOnly). Part of the
[modelwrite](https://github.com/modelwrite/modelwrite) MBSE engine
(AGPL-3.0-or-later).

This is STPA slice **T4** - the adoption route. The offer is not "do STPA in
Modelwrite" but "keep doing STPA in Capella (where MIT's STAMP Tools plugin
lives), and bring the model here for the completeness check, the traceability
and the baseline trend". The reader carries the control structure a team already
built in Arcadia - components, functions, functional exchanges, allocations and
constraints - into OKF, naming everything outside the subset rather than
dropping it.

## What a Capella model actually is (the format)

A Capella "project" is a **folder**, not a single file:

| File | What it is | Machine-readable? |
|---|---|---|
| `<name>.capella` | The **semantic model** (XMI 2.0). Root `capellamodeller:Project`; element type in `xsi:type` (e.g. `org.polarsys.capella.core.data.ctx:SystemFunction`); element identity in the `id` attribute (a UUID); cross-references as `#uuid` string attributes (`source`/`target`/`sourceElement`/`targetElement`), not `xmi:idref`. | **Yes** - this is what the reader imports. |
| `<name>.aird` | The Sirius **diagram representation** layer (GMF/Sirius XMI, ~20 MB for the IFE sample): diagram descriptors, layout and geometry. | Yes, but it is the representation, not the semantics. Rejected with a clear message. |
| `<name>.afm` | Viewpoint metadata (347 bytes): the Capella version and applied viewpoint. | Yes, but it is metadata. |
| `.project`, `images/` | Eclipse project file and diagram image assets. | Not model content. |

The semantic model is organised as `Project -> SystemEngineering ->
ownedArchitectures` (OperationalAnalysis, SystemAnalysis, LogicalArchitecture,
PhysicalArchitecture, EPBSArchitecture). Each architecture holds functions
(`ownedFunctionPkg`), components (`ownedSystemComponentPkg` etc.), exchanges
and allocations. Functions nest via `ownedFunctions`, components via
`owned...Components`; ports are `inputs`/`outputs`/`ownedFeatures`;
flows are `ownedFunctionalExchanges`/`ownedComponentExchanges`; the
component-performs-function link is `ownedFunctionalAllocation`
(ComponentFunctionalAllocation, `sourceElement` = component, `targetElement`
= function). Requirements appear as `capellacore:Constraint` with an
`OpaqueExpression` body (the samples carry no classic `re:Requirement`).

## The subset

The reader is a hand-written roxmltree walk (no new dependency - roxmltree is
already the workspace XML parser, and the engine's rust-version 1.75 floor is
untouched). It carries:

- **components** - `SystemComponent`, `LogicalComponent`, `PhysicalComponent`,
  `Entity` - as blocks (kind "block") whose stereotype names the Capella type;
  an `actor="true"` element adds an "actor" stereotype. These are the
  controllers and controlled processes of the STPA control structure.
- **functions** - `SystemFunction`, `LogicalFunction`, `PhysicalFunction`,
  `OperationalActivity` - as blocks (kind "block") whose stereotype names the
  Capella type. These are the control actions. A function nested inside another
  adds a "contains" edge.
- **FunctionalExchange** - as a "dependency" graph edge from the source function
  to the target function, labelled with the exchange name. This is the directed
  control-action / feedback flow.
- **ComponentExchange / CommunicationMean** - as a "dependency" graph edge from
  the source component to the target component.
- **ComponentFunctionalAllocation** - as a "part" graph edge (label "allocated")
  from the component to the function it performs: the STPA
  controller-owns-control-function link.
- **Constraint** - as a requirement whose reqText is the text of its
  OpaqueExpression body (the system-level constraint / hazard mitigation).
  `re:Requirement` is carried the same way if present.
- **ports** - `FunctionInputPort`/`FunctionOutputPort`/`ComponentPort`/
  `PhysicalPort` - as attributes of their owning function/component.

## The boundary (what is OUTSIDE the subset)

Everything else is reported on import as a named entry - a **Unmappable content
loss** (named, never silent, never a panic) or a **Lossy drop** where the content
survives but an id or a namespace is flattened. That includes:

- the **traceability/realization links** (TransfoLink, ComponentRealization,
  FunctionRealization, FunctionalExchangeRealization, InformationRealization,
  the `*ArchitectureRealization` links, `*Involvement` links) - OKF has no
  home for "this element realizes that element" as a first-class typed link the
  reader reconstructs today;
- the **data/information model** (Class, ExchangeItem, ExchangeItemElement,
  Association, Generalization, Property, DataPkg, Interface, InterfacePkg,
  Enumeration, NumericType, BooleanType, StringType, PhysicalQuantity, Unit);
- the **state/mode machinery** (StateMachine, State, Mode, Region,
  StateTransition, the pseudo-states) - OKF has a single stateMachine section,
  not per-component modes;
- the **interaction/scenario layer** (Scenario, InstanceRole, SequenceMessage,
  MessageEnd, Execution*, CombinedFragment, InteractionOperand, StateFragment,
  InteractionState);
- **functional chains and capabilities** (FunctionalChain, FunctionalChain*,
  Capability, Mission, OperationalCapability, OperationalProcess,
  CapabilityRealization*, ExchangeCategory);
- **physical/deployment detail** (Part, ConfigurationItem, PhysicalLink,
  PartDeploymentLink, ComponentPortAllocation, PortAllocation, PortRealization);
- the **REC/RPL replicable-element catalog** (RecCatalog, CatalogElement*);
- **packages** are flattened (members promoted) and the package id/name is a
  named Lossy drop, because OKF has no namespace concept.

A malformed document, a non-UTF-8 source, a `.aird` or `.afm` handed to the
reader, or a project without a name is a clean `BindingError::Import`, never a
panic.

## The mapping table is declaration, not gate input

`mapping_table()` is the published boundary, one row per construct marked
exact/lossy/unmappable. It is not read by the gate; the gate enforces the
per-import `LossReport`, whose entries name what was actually lost or left
unmapped at instance granularity. The two share the leading construct name.

## Conformance corpus and the measured result

The fixtures in `fixtures/` are HAND-WRITTEN XMI that mirrors the real
`.capella` shape (verified against the fetched corpus); they pin the subset
boundary and are committed (no network). The real corpus is **NOT committed**:
the two Capella samples (`eclipse-capella/capella` samples and
`dbinfrago/Capella-IFE-sample`) are **EPL-2.0**, which is not AGPL-compatible,
so the platform ships only the fetch script (sample/examples/README.md) and the
operator runs it locally - the same principle as operator-supplied symbol packs.
The real-sample test (`tests/real_capella.rs`) locates the fetched model by the
`CAPELLA_IFE_CAPELLA` environment variable or the documented fetch location, and
skips loudly when it is absent. The measured counts on the real IFE sample are
printed by the test; see the sample README for how to fetch it.

Run the measurement with:

    cargo test -p mw-binding-capella --test real_capella -- --nocapture

## Direction

ImportOnly. The notation is large and this is a stated first subset; a binding
that cannot export is a viewer and says so in its metadata rather than pretend.
The round-trip harness refuses it outright (BindingError::Viewer).
