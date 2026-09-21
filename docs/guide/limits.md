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

## No bundled open-source example is loadable today

The Thirty Meter Telescope is the only bundled example in the loadable XMI format, and its
36 MB import needs 48,553 losses accepted by hand before it loads. The Gaphor examples are
Gaphor's own .gaphor format, not XMI, and no Gaphor binding exists. The SysML v2 examples
read as viewers only. Consequence: the showcase's six seeded models are the only models the
platform serves; every bundled open-source example is refused pending acceptance, not
importable, or read-only.

## SysML v2 textual notation is a viewer, not a round-trip

The reader mw-binding-sysmlv2 exists and is Direction::ImportOnly: it imports a stated subset
into OKF and names what it does not carry, but it cannot export, and the round-trip harness
refuses it by design. It is not in the server's binding registry, so the workbench imports
XMI only. Consequence: a SysML v2 model can be read and measured at the engine level, but it
cannot be written back, and it cannot be imported through the workbench.

## The global system-of-systems graph property is asserted, not computed

The compositional gate proves each subsystem reference and each typed cross-model edge
resolves at its pinned revision, and that each integrated revision was itself gated. The
activity's allocatedTo attribute still names a role for the flow view. Consequence: coverage
inside a referenced subsystem is proved per edge, but the whole-system graph property - that
the union of the platform and every subsystem transitively is connected, coverage-complete
and orphan-free - is a stated claim, and the composition page labels it asserted, not
measured.

## The showcase is open-mode and ephemeral

The showcase (trial.modelwrite.org) runs without authentication (anonymous admin) and resets
to its six seeded models every hour. Consequence: do not put real work in the showcase; use
it to read the six deployed models and follow the tour.

## The registered trial cannot deliver its login code

The registered trial (app.modelwrite.org) is live and session-authenticated, but its
transactional login code is written to the operator log, not emailed - the mailer has no SMTP
credential yet. Consequence: a visitor cannot complete registration today; real email
delivery is one SMTP credential away.

## Also true today

- Full behavioural simulation (timing arithmetic over a process, reachability, dead ends, as a
  recorded run) is designed in the analyses spec but not shipped. Consequence: "simulate"
  today means the gate, the coverage report, and the composition process ordering, not
  behaviour execution.
- The workbench has no button that records a gate run. A run is recorded only through the API
  or at import; the compare and make-current pages compute a verdict and record nothing.
  Consequence: getting an evidence record is a curl call, not a click.
