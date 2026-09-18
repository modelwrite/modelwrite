# Slice 3 - The authoring workbench

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (- [ ]) syntax for tracking.

**Goal:** an engineer can open a browser, see the model, understand what changed, resolve a merge conflict, run the gate, and edit the model - without learning an API. This is the slice that makes modelwrite usable by the people who are not the ones who installed it.

**Why the browser and why no build step:** the design is browser-first, and the audience is an engineering organisation that may be air-gapped and may have a security policy that forbids an npm supply chain. So the workbench is **server-rendered HTML with SVG and a small amount of vanilla JavaScript**, served by the same axum service under `/ui`. There is no bundler, no node_modules, no asset pipeline. A page that cannot be built is a page that cannot be audited, and a tool an organisation cannot install is a tool it does not adopt.

**What "authoring" means here, and what it does not:** this slice gives structured editing - change an element's name, documentation, stereotype or attributes through a form, which becomes a commit through the same gate as any other change. It deliberately does NOT give free-form diagram drawing in this slice. Diagrams are rendered from the model with a deterministic layout, because a model whose meaning lives in hand-placed boxes is a model that cannot be merged, diffed or proved - which is the entire point of this platform. Layout that carries meaning arrives later as a binding extension, preserved and round-tripped rather than invented here.

**Tech Stack:** Rust, axum 0.7, the existing engine crates, server-rendered HTML, inline SVG, vanilla JavaScript. One templating dependency (maud) is declared in Task 1 and only there, because compile-time HTML beats string concatenation for escaping safety.

**Spec:** docs/superpowers/specs/2026-09-17-modelwrite-platform-design.md (sections 4.2, 4.4, 5).

## Global Constraints

- Rust stable; edition 2021. The server crate is at rust-version 1.88 (raised when a transitive dependency required it). The engine crates remain at 1.75.
- Every source file begins with: // SPDX-License-Identifier: AGPL-3.0-or-later
- Cargo is not on PATH in fresh shells: begin every shell command sequence with $env:Path = "$env:USERPROFILE\\.cargo\\bin;" + $env:Path
- Use --no-fail-fast when running a test suite.
- Stage and commit only the paths a task owns. Never amend or rewrite a commit.
- THE UI IS A CLIENT OF THE EXISTING API, NOT A SECOND IMPLEMENTATION. Every action the UI performs goes through the same handlers, the same permission checks and the same audit log. A UI-only write path would be a hole in every guarantee the previous slices established.
- The UI must respect authentication: with auth configured, an unauthenticated request to a UI route is a 401 rendered as a sign-in prompt, never a crash or a silent empty page.
- HTML ESCAPING IS NOT OPTIONAL: every value that comes from a model, a commit message, a branch name or an error must be escaped. A model is untrusted input from a colleague, a supplier or an import.
- Every view must render usefully for the coffee-machine corpus, which is the only model every contributor has.
- Tests drive the router in process and assert on the HTML, so a view that stops rendering a section fails the suite.

---

## Task 1: The workbench shell and the project list

**Files:**
- Modify: server/Cargo.toml (the templating dependency, declared here only)
- Create: server/src/ui/mod.rs, server/src/ui/layout.rs, server/src/ui/pages.rs
- Modify: server/src/lib.rs (a /ui route, the module)
- Test: server/tests/ui.rs

**Interfaces:**
- Produces: `GET /ui` renders the project list with, per project, its branch count and its latest commit; `GET /ui/projects/:project` renders the project's branches, each with its tip and the tip's message and author. Both require Read and render a sign-in prompt on 401. The layout provides a header (project name, identity subject, auth mechanism) and a navigation rail that later tasks extend.

**Acceptance:** with the coffee-machine corpus loaded, `/ui` lists it; `/ui/projects/coffee` lists `main` with the tip commit's message; a viewer sees the pages; an unauthenticated request with auth configured sees a sign-in prompt and status 401; a project name containing `<script>` is escaped and cannot execute.

---

## Task 2: The model views

**Files:**
- Create: server/src/ui/model.rs
- Modify: server/src/ui/pages.rs, server/src/lib.rs (routes)
- Test: server/tests/ui.rs

**Interfaces:**
- Produces: `GET /ui/projects/:project/model?branch=&commit=` rendering four sections of the OKF document: **structure** as an indented tree of elements with kind, stereotypes and documentation; **requirements** as a table with id, reqId, text and the elements that satisfy it; **traceability** as a matrix of requirements against the elements allocated to them, with uncovered requirements visibly marked; **state and activity** as a list of regions, states and transitions, and activities with their nodes.
- The traceability matrix is computed by the EXISTING coverage code in the graph crate, not re-implemented in the UI.
- Unresolved edges (the corpus has two) render as explicit broken links rather than being hidden.

**Acceptance:** the coffee-machine corpus renders 49 blocks, 25 requirements, 9 signals and 8 activities; ten uncovered requirements are visibly marked, matching the coverage report; the two dangling edges are shown as unresolved.

---

## Task 3: Diff and conflict review

**Files:**
- Create: server/src/ui/review.rs
- Modify: server/src/ui/pages.rs, server/src/lib.rs
- Test: server/tests/ui.rs

**Interfaces:**
- Produces: `GET /ui/projects/:project/compare?from=&to=` rendering the engine's section-scoped diff grouped by section, with added, removed and changed entries, and the gate verdict for the pair with a link to the evidence. `POST /ui/projects/:project/merge` performs a merge and, on 409, renders each conflict with base, ours and theirs side by side and a resolution form.
- The diff is the ENGINE's diff. The UI must not compute its own, or the two would disagree and the reviewer would trust the wrong one.

**Acceptance:** comparing the clean and corrupted corpus commits shows the missing requirement and the isolated node exactly as the gate reports them, and shows the failed verdict with the evidence path.

---

## Task 4: Editing that becomes a commit

**Files:**
- Create: server/src/ui/edit.rs
- Modify: server/src/ui/pages.rs, server/src/lib.rs
- Test: server/tests/ui.rs

**Interfaces:**
- Produces: `GET /ui/projects/:project/edit/:element` rendering a form for an element's name, documentation, stereotypes and attributes; `POST` the same path to commit the change. The commit carries the verified identity as its author, takes a lock on the element for the duration of the request, and is refused with the gate's reasons if the result is invalid. The form shows which OTHER holders currently hold the element, and refuses to submit while it does.

**Acceptance:** editing a block's documentation through the form creates a commit whose author is the identity; editing an element locked by another holder is refused with the holder named; an edit that would produce an invalid model is refused with the validator's errors rendered, not a 500.

---

## Task 5: The gate, rendered where a reviewer looks

**Files:**
- Create: server/src/ui/gate.rs
- Modify: server/src/ui/pages.rs, server/src/lib.rs
- Test: server/tests/ui.rs

**Interfaces:**
- Produces: `GET /ui/projects/:project/gate` listing recent gate runs newest first with their verdict, reference and candidate hashes and a link to the evidence file; `GET /ui/projects/:project/gate/:reference/:candidate` rendering one run in full: pass or fail, the fidelity failures by name, the coverage numbers, and the evidence record.

**Acceptance:** the corpus self-pass renders green with its counts; the corrupted pair renders red naming the missing element and the isolated node; a viewer without Review sees the list but not the detail, matching the API's permissions.

---

## Task 6: The diagram view

**Files:**
- Create: server/src/ui/diagram.rs
- Modify: server/src/ui/pages.rs, server/src/lib.rs
- Test: server/tests/ui.rs

**Interfaces:**
- Produces: `GET /ui/projects/:project/diagram?branch=&commit=` rendering the graph as inline SVG with a DETERMINISTIC layout: nodes grouped by kind on a grid, edges drawn as lines with labels, requirement nodes visually distinct, and unresolved endpoints drawn as dangling markers. The same model must always produce byte-identical SVG, so that a diagram can be diffed and cached.

**Acceptance:** rendering the coffee-machine corpus twice produces identical SVG; every node and edge appears; an unresolved endpoint is drawn rather than dropped; the SVG contains no script and no external reference, so it is safe to render in any browser and to embed in a report.

---

## Completion criteria

- [ ] cargo test --workspace --no-fail-fast passes; fmt and clippy clean.
- [ ] Every view renders the coffee-machine corpus correctly, and the tests assert on the rendered HTML rather than on JSON.
- [ ] Every action in the UI goes through the existing API handlers, so permissions, locks, the audit log and the gate all apply unchanged.
- [ ] No unescaped value from a model, name or message can reach the page.
- [ ] The workbench needs no build step: no node, no bundler, no asset pipeline.

## What the next slice must add (not in this plan)

Slice 4, interoperability: the bindings that bring an organisation's existing models in - SysML v1 XMI first, because it is the format most likely to be stranded - with the three-way mapping matrix, the loss report, and the migration gate described in docs/superpowers/specs/2026-09-17-modelwrite-standards-interoperability.md.
