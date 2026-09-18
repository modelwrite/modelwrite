# SysML v1 XMI fixtures

These documents are HAND-WRITTEN synthetic XMI. They exercise the reader in
engine/binding-xmi and prove nothing about any vendor's exporter. Real vendor
XMI needs its own corpus and its own fidelity report, which is a later tranche.

- coffee-grinder.xmi: the happy path - a package holding two blocks, a
  composite property, a primitive property with a default, a comment, and a
  Satisfy dependency edge.
- unknown-element.xmi: a known block plus an unmapped element (uml:StateMachine)
  and an unmapped attribute (visibility), to prove nothing is dropped in silence.
- malformed.xmi: deliberately broken XML, to prove a malformed document is a
  clean error, never a panic.

## Conventions

- Element identity is xmi:id, carried verbatim as the OKF element id.
- A block is a uml:Class carrying a Block stereotype, applied as a child
  element with a base_Class reference: <Block xmi:id="..." base_Class="..."/>.
- A dependency is a uml:Dependency with client/supplier references and a
  Satisfy/Allocate/Refine/Verify stereotype applied as a child element with a
  base_Dependency reference.
- A property is an ownedAttribute uml:Property with a type reference (an
  xmi:id that resolves to a block name, or a primitive type literal), an
  aggregation (none/shared/composite) and an optional default.
- Documentation is an ownedComment uml:Comment whose body is the text.
- A package is a namespace: its members are promoted to the top-level
  structure and the package itself is named in the loss report.
