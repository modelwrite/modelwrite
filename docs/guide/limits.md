# Limits

What modelwrite does not do, in one place. Each limit carries its consequence in the same
breath. This is the most valuable page in the guide: it is what makes every other page
believable. Where a capability has a boundary, the boundary is stated here rather than hidden
in a footnote.

## No numeric or physical simulation

There are no solvers, no physics executables, no parametric solver, no finite-element or
computational-fluid model. Consequence: fluid, thermal and structural behaviour must be
computed in a different tool; modelwrite holds the model and the requirements that tool is
checked against, and nothing more.

## Native XMI-to-OKF fidelity is not independently measured

The engine measures only the binding's own OKF to XMI to OKF round trip and diffs it. The
native XMI to OKF read of your artifact, which is the actual migration, is not measured by
the engine. Consequence: the loss report is the binding's self-account of that read, and it
must be read as such, not as an engine proof. The import page states this on the page.

## A real content-rich vendor corpus is only now being added

The proof fixture is the coffee-machine CATIA Magic model. The only real-world file in the
interoperability corpus is a Papyrus project skeleton that contains zero model elements, and
the corpus says so itself. Consequence: any claim that modelwrite can migrate a real,
content-rich vendor model is unproven today, and the measurement will be published as the
corpus grows.

## SysML v2 textual notation has no reader yet

The only binding shipped is sysml-v1-xmi@2.4. SysML v2 is a roadmap binding, not code.
Consequence: a SysML v2 textual model cannot be imported today, and no claim to the contrary
should be read into the roadmap.

## The activity-to-subsystem-element link is a role name, not a cross-model edge

A platform activity's link to an element inside a subsystem is carried by the activity's
allocatedTo attribute, which names a role. It is not a first-class edge from one model to
another. Consequence: coverage inside a referenced subsystem is not computed, and the
composition page labels as asserted what it cannot measure.

## The trial is open-mode and ephemeral

The trial runs without authentication (anonymous admin) and is tunnelled from a dev machine,
and it can be reset. Consequence: do not put real work in the trial; use it to read the
deployed models and follow the tour.

## Also true today

- Full behavioural simulation (timing arithmetic over a process, reachability, dead ends, as a
  recorded run) is designed in the analyses spec but not shipped. Consequence: "simulate"
  today means the gate, the coverage report, and the composition process ordering, not
  behaviour execution.
- The workbench has no button that records a gate run. A run is recorded only through the API
  or at import; the compare and make-current pages compute a verdict and record nothing.
  Consequence: getting an evidence record is a curl call, not a click.
