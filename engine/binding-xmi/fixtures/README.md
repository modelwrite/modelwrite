# SysML v1 XMI fixtures

These documents are HAND-WRITTEN synthetic XMI. They exercise the reader in
engine/binding-xmi and prove nothing about any vendor's exporter. Real vendor
XMI needs its own corpus and its own fidelity report, which is a later tranche.

- coffee-grinder.xmi: the happy path - a package holding two blocks, a
  composite property, a primitive property with a default, a comment, and a
  Satisfy dependency edge. Its dropped ids (Model, Property x2, Comment,
  Dependency) and the package flattening are all named in the loss report.
- unknown-element.xmi: a known block plus an unmapped element (uml:StateMachine)
  and an unmapped attribute (visibility), to prove nothing is dropped in silence.
- malformed.xmi: deliberately broken XML, to prove a malformed document is a
  clean error, never a panic.
- comment-nonblock.xmi: comments whose annotatedElement targets a package, a
  dangling id, and a mixed (block + dangling) list, to prove a body that cannot
  attach to an emitted block is named rather than dropped.
- name-loss.xmi: a comment carrying a name and a stereotyped dependency carrying
  a name, to prove both dropped names are reported.
- nonblock-class.xmi: a class without the Block stereotype holding a property,
  to prove the class and its property are named individually.
- foreign-attr.xmi: a foreign-namespaced attribute sharing the local name
  "name" with the UML attribute, to prove it is not read as UML and is reported.
- root-attr.xmi: an xmi:version attribute on the xmi:XMI root, to prove root
  attributes are reported rather than passed over.
- magicdraw-requirements.xmi: the MagicDraw REQUIREMENT/TRACEABILITY shape - a
  requirement is a uml:Class carrying a <sysml:Requirement base_Class=... Id=...
  Text=.../> stereotype application, and Satisfy/Allocate are uml:Abstraction
  elements whose client/supplier are CHILD elements (<client xmi:idref=.../>)
  with a <sysml:Satisfy base_Abstraction=.../> sibling application.
- requirement-no-id.xmi: a requirement whose <sysml:Requirement> stereotype
  application carries an EMPTY Id (Id=""), exactly as the two TMT template
  requirements (#parent, #child) do. The reader carries the empty Id verbatim as
  an empty reqId - a fact about the source, not a parsing gap - and the OKF
  validator accepts it (a missing identifier is not a structural error).

## Conventions

- Element identity is xmi:id. A block's xmi:id is carried verbatim as the OKF
  element id and graph node id; the ids of uml:Model, uml:Package, uml:Property,
  uml:Dependency and uml:Comment have no OKF slot and are named in the loss
  report (never dropped in silence).
- A block is a uml:Class carrying a Block stereotype, applied as a child
  element with a base_Class reference: <Block xmi:id="..." base_Class="..."/>.
- A dependency is a uml:Dependency with client/supplier references and a
  Satisfy/Allocate/Refine/Verify stereotype applied as a child element with a
  base_Dependency reference. MagicDraw instead uses a uml:Abstraction whose
  client/supplier are child elements and applies the stereotype as a sibling
  with a base_Abstraction reference; the reader accepts both forms.
- A requirement is a uml:Class carrying a Requirement stereotype applied as
  <sysml:Requirement base_Class=... Id=... Text=.../>; the class carries the
  name, Id carries reqId and Text carries reqText.
- A property is an ownedAttribute uml:Property with a type reference (an
  xmi:id that resolves to a block name, or a primitive type literal), an
  aggregation (none/shared/composite) and an optional default.
- Documentation is an ownedComment uml:Comment whose body is the text; a body
  whose annotatedElement is not an emitted block is named in the loss report.
- A package is a namespace: its members are promoted to the top-level
  structure and the package itself is named in the loss report.