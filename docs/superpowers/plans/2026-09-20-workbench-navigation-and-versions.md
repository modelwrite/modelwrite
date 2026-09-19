# Workbench navigation and version control — plan

## The problem, as experienced

Three models now exist (coffee-machine, sandwich-toaster, purchasing-terminal) and the workbench offers **one bare "Projects" link**. There is no way to tell where you are, no way to switch model, and no way to move between a model's sections without scrolling a wall of them. The engine already has versions (branches, commits, diff, merge) and the UI exposes almost none of it: **no version selector, no "new version", no "make this the current version".**

That is not a styling problem. It is a missing information architecture.

## The navigation model: three levels, always visible

**1. Global bar (top, every page).**
Product mark · **model switcher** (the list of models, current one marked) · **New model** · **Import a legacy model** · identity chip.
Answers: *which model am I in, and how do I get to another one?*

**2. Left navigator (every page inside a model).**
- **MODELS** — every project, current highlighted. One click to switch.
- **SECTIONS** for the current model: **Overview · Structure · Requirements · Traceability · Diagram · Checks · Changes · Proposals · Import · Assist**.
Answers: *what is in this model, and where do I go next?*
Server-rendered links, so it works with JavaScript disabled.

**3. Context bar (top of the content area).**
**Project · Version · Commit** with the version actions: **New version · Compare · Make current**.
Answers: *which version am I looking at, and how do I change it?*

## Screen inventory

| Section | Purpose | Exists today? |
|---|---|---|
| Overview | what this model is: counts, coverage, last checks, versions | partial (project page) |
| Structure | the containment tree + element detail | yes (as part of one long page) |
| Requirements | requirement table, reqIds, text, satisfied-by, coverage | yes |
| Traceability | requirement → satisfying element, and the uncovered list | partial |
| Diagram | rendered SVG, selectable | yes |
| Checks | gate runs for this model, newest first | yes |
| Changes | version history, diffs between versions, compare any two | engine exists, UI thin |
| Proposals | agent proposals + accept/refuse | yes |
| Import | migrate a legacy model | yes |
| Assist | describe a change in words → proposal | yes |

Today most of these are sections of ONE page. The change is to give each its own address and a place in the navigator, with the page kept as an Overview that links out.

## Change control: versions are branches

No new engine concept is invented. A **version** is a branch; its identity is its tip commit.

1. **New version** — branch from a chosen tip, named (e.g. `v2-triangle-plates`). Made from the context bar.
2. **Edit on that version** — the editor and the assist both commit to the selected branch (they already take a branch).
3. **See what happens** — **Compare** renders the engine's section-scoped diff between two versions, plus the gate verdict and its evidence. This is the "did my change break anything" step.
4. **Select that as the version** — **Make current** merges the chosen version into the main line, through the existing merge endpoint, and the merge is a commit like any other: it appears in Changes, with an audit entry and, when the candidate fails the gate, **the failure is shown before the merge is offered**, not after.

## Rulings

1. **The current model and version are always in the URL.** Any page can be linked, bookmarked, and shared; no hidden state. *Cost if wrong:* longer URLs. *Benefit:* every screen is addressable, which is also what makes it testable.
2. **The navigator is links, not a JavaScript menu.** It must work with JS disabled, like everything else here.
3. **No new engine concepts.** Versions are branches; selecting one is a merge. The UI is a client of endpoints that already exist and are already tested.
4. **A version action is recorded.** Creating, comparing and promoting all leave audit entries and, for a promotion, a commit with provenance — because "which version is current, and who made it current" is exactly the question an organisation asks later.
5. **The gate is shown before the promotion, not after.** An engineer decides with the evidence in front of them.

## Acceptance criteria

- From any page, the **current model and version are visible**, and switching either takes one click.
- **Three models are navigable**, each with its own sections at their own addresses.
- A **new version can be created, edited, compared and made current**, and the result is visible in Changes.
- **Screenshots reviewed** at each step (a picture, not a claim), and the headless browser suite extended to cover the navigator and the version selector — the existing 7 tests stay green.
- Works with **JavaScript disabled**; no external request; no build step.

## Sequence

1. **N1** the shell: global bar, model switcher, left navigator, section routes, context bar showing project/version/commit.
2. **N2** per-section addresses: split the long model page into Overview + its sections, preserving the existing content and the tests' hooks.
3. **N3** version control: version selector, New version, Compare between versions, and Make current with the gate shown first.
4. **N4** verification sweep: screenshots of every section for every model, browser tests extended, no-JS check.
