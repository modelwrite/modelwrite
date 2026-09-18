# Real-world models

Everything in `engine/binding-xmi/fixtures/` is HAND-WRITTEN, and that directory says
so. This one is different, and the difference is the point.

## Why this directory exists

A synthetic fixture tests the reader. It cannot tell you what happens when a real
export from a real tool arrives, because the person who wrote the fixture already
knew what the reader understood. The first time a real Papyrus SysML model was
imported, it exposed a usability defect that no hand-written fixture had: the loss
report could not distinguish *"I could not carry your model content"* from *"I
recognised a profile declaration, which carries no content"*, so thirteen
declarations drowned one real finding. That defect is fixed, and this file is the
regression that holds the fix in place.

## LandingGear.uml

Source: https://github.com/KevinDelcourt/LandingGear (LandingGear.uml, and its
companion LandingGear.notation). Retrieved for interoperability testing.

It is a **Papyrus project skeleton**, and that is worth stating plainly because it
surprised us: the file contains **zero model elements**. No packagedElement, no
ownedAttribute, no Class. It consists entirely of ten `uml:ProfileApplication`
and two `uml:PackageImport` declarations whose targets are `pathmap://` URIs.

`pathmap://` is an Eclipse-internal scheme. Those references resolve inside an
Eclipse workspace and NOWHERE ELSE - not on a server, not in CI, not in this
platform. Any organisation exporting Papyrus models should expect them, and should
expect that a model referencing libraries this way depends on an IDE to interpret
it.

## What this file does and does not prove

It proves the reader handles a real Papyrus document without panicking, reports
what it finds, and states plainly that it found no model content.

It does NOT prove that this platform can migrate a real, content-rich SysML model.
That test has not been run, because a content-rich public example has not yet been
found. Until it is, any claim about migrating real vendor models is unproven - and
the honest statement of this platform's readiness is exactly that.
