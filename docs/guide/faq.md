# FAQ

Real questions, real answers. Nothing here is softened.

## Can it replace CATIA?

Not yet. Two bindings exist:

<!-- generated:binding-list -->
| Binding | id@version | Direction | Reads |
|---|---|---|---|
| SysML v1 (UML profile) XMI | `sysml-v1-xmi@2.4` | read/write | blocks, requirements, properties and Satisfy/Allocate traceability |
| SysML v2 textual notation (.sysml) | `sysml-v2-textual@1.0` | viewer (ImportOnly) | part/attribute/item definitions, requirements, satisfy traceability and documentation |

The workbench import surface (the server's binding registry) offers `sysml-v1-xmi@2.4`; `sysml-v2-textual@1.0` is not registered in the server, so the workbench does not offer it.
<!-- /generated -->

Requirement and traceability import works against the coffee-machine corpus, which is the
proof fixture. A real, content-rich vendor model is not yet in the corpus, and anything
outside a binding's stated subset is reported unmappable: a named loss, never a silent drop.
Modelwrite has no numeric or physical simulation, so it does not replace the simulation side
of the CATIA portfolio either. Today it is a migration source and a coexistence partner, not a
replacement.

## Can it simulate?

Model-level analysis yes, solvers no. The gate and the coverage report run over the model's
own graph and are shipped. The Composition page orders the business process from the activity
graph and flags cycles. What modelwrite does not have is numeric or physical simulation:
solving fluid, thermal or structural behaviour needs solvers and executables the platform does
not ship. Full behavioural simulation (timing arithmetic over a process, reachability, dead
ends, as a recorded run) is designed in the analyses spec but not yet shipped. If you need a
solver, modelwrite's job is to hold the model and the requirements the solver is checked
against.

## Is my data safe in the trial?

No. The trial runs in open mode (no authentication, anonymous admin) and is tunnelled from a
dev machine. Anything you put there is readable by anyone who reaches it, and the trial can be
reset. Do not put real work in the trial. It exists to let you look at the six deployed models
and follow the tour.

## What happens to my old models?

They are retained byte for byte and content-addressed before anything else happens. A
migration never deletes the source artifact: it is stored under its sha256 address forever,
and the import commit's provenance names the retained artifact and the binding. Every loss is
named before you accept it. A lossy import is refused until you accept each blocking loss by
name, and a loss left unchecked refuses the import again.

## Does an AI change my model?

It proposes; a human accepts. The assist panel produces a review artifact, never a commit.
Acceptance needs write permission, which an agent cannot hold, and the commit path re-checks
the acceptance inside its own transaction. The commit's provenance names both parties: the
agent that proposed and the human that accepted. An agent cannot change a model on its own.

## What does it cost, and what licence?

The code is free and open source. The engine, server, CLI, and today's binding crates are
AGPL-3.0-or-later. The interchange layer (importers and exporters) is designated Apache-2.0
and will ship under that licence as they land; today the binding crates still carry the
workspace AGPL licence.

The split matters for a commercial deployment. AGPL is a copyleft licence whose obligations
trigger when you modify covered code and provide it as a network service, and you must then
offer the corresponding source, including your changes. The Apache-2.0 interchange layer is
the boundary designed to let an organisation write and distribute its own importers and
exporters without taking on that obligation. Whether a specific deployment triggers an AGPL
obligation is a legal question. See LICENSE and NOTICE.md, and read the licences yourself or
ask a lawyer.
