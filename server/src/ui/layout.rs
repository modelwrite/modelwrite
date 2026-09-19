// SPDX-License-Identifier: AGPL-3.0-or-later
//! The shared page shell: the header, the navigation rail, the inline stylesheet, and the
//! sign-in and error renderings.
//!
//! There is deliberately no build step and no asset pipeline. The stylesheet is inlined in
//! the page, and every dynamic value is spliced through maud, which escapes it by
//! construction. A value that reaches a page from a model, a commit message, a branch name
//! or an error therefore cannot execute as markup: the only unescaped content is the
//! developer-written stylesheet, wrapped in `maud::PreEscaped` on purpose.

use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};
use maud::{html, Markup, PreEscaped, DOCTYPE};

use crate::api::{map_store_error, ApiState};
use crate::auth::Identity;
use crate::error::ApiError;

const STYLE: &str = r#"
/* =========================================================================
   modelwrite design system
   Hand-written, no build step, nothing fetched. Every colour is a custom
   property below so the whole UI shares one palette.
   ========================================================================= */

:root {
  color-scheme: light;

  /* -- accent: the ONE brand colour. Used sparingly for the primary action,
     the selected tree row, links and the focus ring. Everything else is
     neutral grey so the accent always means "interactive / active". */
  --accent: #2563eb;
  --accent-strong: #1d4ed8;
  --accent-tint: #eff6ff;
  --on-accent: #ffffff;

  /* -- neutrals: the chrome grey scale. surface = raised panel, surface-1 =
     muted well, surface-2 = hover, border = strong line, border-muted =
     faint line. text / text-2 / text-3 are three ink steps (primary,
     secondary, faint). */
  --surface: #ffffff;
  --surface-1: #f6f8fa;
  --surface-2: #eaeef2;
  --border: #d1d9e0;
  --border-muted: #e5eaef;
  --text: #1f2328;
  --text-2: #59636e;
  --text-3: #8b949e;

  /* -- semantics: PASS / FAIL / UNKNOWN / WARN. Each is a tint plus an ink so
     a chip is readable without relying on colour alone (a border and a text
     label always accompany the tint). */
  --pass: #1a7f37;
  --pass-bg: #dafbe1;
  --fail: #cf222e;
  --fail-bg: #ffebe9;
  --unknown: #6e7781;
  --unknown-bg: #f0f2f5;
  --warn: #9a6700;
  --warn-bg: #fff8c5;

  /* -- kinds: one colour per element kind. Used for the small dot in the
     containment tree and the diagram so the same kind is the same colour
     everywhere. Unknown kinds fall back to the neutral grey. */
  --kind-block: #2563eb;
  --kind-actor: #8250df;
  --kind-usecase: #0550ae;
  --kind-requirement: #9a6700;
  --kind-signal: #1a7f37;
  --kind-interface: #0a7ea4;
  --kind-activity: #bc4c00;
  --kind-state: #bf3989;
  --kind-statemachine: #cf222e;
  --kind-neutral: #6e7781;

  /* -- type: a system UI face for everything (no serif), a monospace face for
     ids, hashes and code. */
  --font-ui: system-ui, -apple-system, "Segoe UI", Roboto, Helvetica, Arial, sans-serif;
  --font-mono: ui-monospace, "SFMono-Regular", "Cascadia Code", "JetBrains Mono", Consolas, "Liberation Mono", Menlo, monospace;

  --radius: 6px;
  --radius-sm: 4px;
  --header-h: 3rem;
}

/* -- base ---------------------------------------------------------------- */

* { box-sizing: border-box; }
html { -webkit-text-size-adjust: 100%; }
body {
  margin: 0;
  font-family: var(--font-ui);
  font-size: 14px;
  line-height: 1.5;
  color: var(--text);
  background: var(--surface-1);
}

a { color: var(--accent); text-decoration: none; }
a:hover { text-decoration: underline; }

code, pre, kbd {
  font-family: var(--font-mono);
}
code { font-size: 0.85em; }

/* Keyboard focus: an outline on every interactive control, never colour alone. */
:where(a, button, input, textarea, select, [tabindex], .mw-node, .mw-group-label):focus-visible {
  outline: 2px solid var(--accent);
  outline-offset: 2px;
}

/* -- type scale ---------------------------------------------------------- */
/* page title > section title > body > meta, each a real size/weight step. */

h1 {
  font-size: 20px; font-weight: 650; line-height: 1.25;
  letter-spacing: -0.01em;
  margin: 0 0 0.6rem;
}
h2 { font-size: 15px; font-weight: 600; margin: 0 0 0.5rem; }
h3 { font-size: 13px; font-weight: 600; margin: 0 0 0.35rem; }
h4 { font-size: 12px; font-weight: 600; margin: 0 0 0.25rem; }

.meta { color: var(--text-2); font-size: 12px; margin: 0.25rem 0 0.75rem; }
.meta code { color: var(--text-2); }

/* -- chrome: top bar ----------------------------------------------------- */

.site-header {
  display: flex; align-items: center; gap: 0.75rem;
  height: var(--header-h);
  padding: 0 1.25rem;
  background: var(--surface);
  border-bottom: 1px solid var(--border);
  position: sticky; top: 0; z-index: 20;
}
.site-header .brand {
  display: inline-flex; align-items: center; gap: 0.5rem;
  font-weight: 700; font-size: 15px; letter-spacing: -0.01em;
  color: var(--text);
}
.site-header .brand::before {
  content: "";
  flex: 0 0 auto;
  width: 16px; height: 16px;
  background-image: url("data:image/svg+xml,%3Csvg%20xmlns='http://www.w3.org/2000/svg'%20width='16'%20height='16'%20viewBox='0%200%2016%2016'%20fill='none'%3E%3Ccircle%20cx='4'%20cy='8'%20r='2.4'%20fill='%232563eb'/%3E%3Ccircle%20cx='12'%20cy='4'%20r='2.4'%20fill='%232563eb'%20opacity='0.45'/%3E%3Ccircle%20cx='12'%20cy='12'%20r='2.4'%20fill='%232563eb'%20opacity='0.45'/%3E%3Cpath%20d='M6.2%207%209.8%204.7M6.2%209l3.6%202.3'%20stroke='%232563eb'%20stroke-width='1.2'%20stroke-linecap='round'/%3E%3C/svg%3E");
  background-repeat: no-repeat;
  background-size: contain;
}
.site-header .brand:hover { color: var(--accent); text-decoration: none; }
.site-header .project {
  font-size: 12px; color: var(--text-2);
  background: var(--surface-1);
  border: 1px solid var(--border-muted);
  padding: 0.12rem 0.55rem;
  border-radius: 999px;
  max-width: 18rem; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
}
.site-header .who { margin-left: auto; color: var(--text-2); font-size: 12px; }

/* -- chrome: layout and rail -------------------------------------------- */

.layout { display: flex; min-height: calc(100vh - var(--header-h)); }
.rail {
  flex: 0 0 11rem;
  border-right: 1px solid var(--border);
  padding: 1rem 0.75rem;
  background: var(--surface);
}
.rail ul { list-style: none; margin: 0; padding: 0; }
.rail li { margin-bottom: 0.15rem; }
.rail a {
  display: block; padding: 0.4rem 0.7rem;
  border-radius: var(--radius);
  color: var(--text-2); font-size: 13px;
}
.rail a:hover { background: var(--surface-1); color: var(--text); text-decoration: none; }

main { flex: 1 1 auto; min-width: 0; padding: 1.5rem 2rem; max-width: 56rem; }
/* A page that lays out a grid or a table across the whole width (the project
   list, the traceability matrix) opts out of the reading-width cap. */
main.mw-wide { max-width: none; }
main.mw-model-ide { max-width: none; padding: 1.25rem 1.5rem; }

.sign-in, .error {
  max-width: 40rem;
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  padding: 1.25rem 1.5rem;
}

/* -- panels: the bordered cards that hold lists and sections ------------ */

.model-section { margin-bottom: 2rem; }
.model-section > h2 {
  padding-bottom: 0.4rem;
  margin-bottom: 0.75rem;
  border-bottom: 1px solid var(--border-muted);
}

ul.projects, ul.branches, ul.proposals, ul.proposal-list, ul.accepted-items, ul.gaps,
ul.signals, ul.interfaces, ul.states, ul.transitions, ul.allocations, ul.activity-nodes,
ul.unresolved-edges, ul.failures, ul.isolated, ul.diff-list, ul.loss-list, ul.loss-accept {
  list-style: none; margin: 0; padding: 0;
}

/* The project list is a responsive card grid; the WHOLE card is one link, so
   the obvious action (open the model) is a click anywhere on the card. */
ul.projects {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(19rem, 1fr));
  gap: 0.75rem;
}
ul.projects li { margin: 0; padding: 0; border: none; background: none; }

ul.branches li, ul.proposals li, ul.proposal-list li {
  border: 1px solid var(--border);
  border-radius: var(--radius);
  padding: 0.75rem 1rem;
  margin-bottom: 0.5rem;
  background: var(--surface);
}
ul.proposals li h2, ul.proposal-list li h2 { margin-top: 0; }

a.project-card {
  display: flex; flex-direction: column; gap: 0.3rem;
  height: 100%;
  padding: 0.85rem 1rem;
  border: 1px solid var(--border);
  border-radius: var(--radius);
  background: var(--surface);
  color: var(--text);
}
a.project-card:hover {
  border-color: var(--accent);
  background: var(--accent-tint);
  text-decoration: none;
}
a.project-card:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
.project-name { font-weight: 650; font-size: 15px; color: var(--text); }
a.project-card:hover .project-name { color: var(--accent-strong); }
.project-meta { color: var(--text-2); font-size: 12px; }
.project-meta .dot { color: var(--text-3); margin: 0 0.3rem; }
.project-message { color: var(--text); font-size: 13px; }
.project-author { color: var(--text-3); font-size: 12px; }
.project-open {
  margin-top: 0.4rem;
  font-size: 12px; font-weight: 600; color: var(--accent);
  display: inline-flex; align-items: center; gap: 0.3rem;
}

a.branch-name { font-weight: 600; color: var(--text); }
a.branch-name:hover { color: var(--accent); text-decoration: none; }
.branch-name { font-weight: 600; }
code.tip { color: var(--text-3); margin-left: 0.5rem; font-size: 12px; }
.message { display: block; margin-top: 0.25rem; color: var(--text); }

/* -- empty and unknown states look deliberate --------------------------- */

.empty-state {
  background: var(--surface);
  border: 1px dashed var(--border);
  border-radius: var(--radius);
  padding: 1.25rem 1.5rem;
  color: var(--text-2);
}

.mw-empty, .none { color: var(--text-3); }
.broken { color: var(--fail); font-weight: 600; }

/* -- chips: PASS / FAIL / UNKNOWN / WARN, plus kind labels -------------- */
/* A border plus a text label always accompanies the tint, so the meaning is
   never carried by colour alone. */

.covered, .uncovered,
.mw-badge-covered, .mw-badge-uncovered, .mw-badge-unknown,
.low-confidence {
  display: inline-block;
  font-weight: 600; font-size: 11px;
  padding: 0.04rem 0.45rem;
  border-radius: 999px;
  border: 1px solid transparent;
  white-space: nowrap;
  line-height: 1.5;
}
.covered, .mw-badge-covered { background: var(--pass-bg); color: var(--pass); border-color: var(--pass); }
.uncovered, .mw-badge-uncovered { background: var(--fail-bg); color: var(--fail); border-color: var(--fail); }
.mw-badge-unknown { background: var(--unknown-bg); color: var(--unknown); border-color: var(--unknown); }
.low-confidence { background: var(--warn-bg); color: var(--warn); border-color: var(--warn); }

.element-kind, .node-type {
  font-family: var(--font-mono);
  font-size: 11px; color: var(--text-2);
  background: var(--surface-1);
  border: 1px solid var(--border-muted);
  border-radius: 999px;
  padding: 0.02rem 0.4rem;
  margin-left: 0.4rem;
  white-space: nowrap;
}
.element-stereotypes { color: var(--warn); font-size: 12px; margin-left: 0.4rem; }

/* -- forms: real controls with affordance ------------------------------- */

label { display: block; font-weight: 600; font-size: 12px; color: var(--text-2); margin-bottom: 0.25rem; }

form p { margin: 0 0 0.75rem; }
form input[type="text"], form input[type="search"], form textarea, form select {
  font-family: var(--font-ui);
  font-size: 14px;
  color: var(--text);
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  padding: 0.45rem 0.6rem;
  width: 100%;
}
form textarea { min-height: 4rem; resize: vertical; }
form input[type="text"]:focus-visible, form textarea:focus-visible, form select:focus-visible,
form input[type="search"]:focus-visible {
  border-color: var(--accent);
  outline: 2px solid var(--accent);
  outline-offset: 1px;
}
form input[type="checkbox"] { accent-color: var(--accent); width: auto; }

button {
  font-family: var(--font-ui);
  font-size: 14px; font-weight: 600;
  color: var(--on-accent);
  background: var(--accent);
  border: 1px solid var(--accent);
  border-radius: var(--radius);
  padding: 0.45rem 0.95rem;
  cursor: pointer;
}
button:hover { background: var(--accent-strong); border-color: var(--accent-strong); }
button:active { background: var(--accent-strong); }
button:disabled {
  background: var(--surface-2); border-color: var(--border); color: var(--text-3);
  cursor: not-allowed;
}

.edit-form input[type="text"], .edit-form textarea,
.import-form input[type="text"], .import-form select, .import-form textarea,
.merge-form input[type="text"], .compare-form input[type="text"] { max-width: 40rem; }
.import-form textarea { min-height: 10rem; }
/* A project or branch name is short; it does not need the full content width. */
.create-form input[type="text"], .create-form select { max-width: 24rem; }

.edit-form fieldset {
  border: 1px solid var(--border);
  border-radius: var(--radius);
  margin: 0.75rem 0;
  padding: 0.75rem;
  max-width: 40rem;
}
.edit-form fieldset legend { font-weight: 600; font-size: 12px; color: var(--text-2); padding: 0 0.4rem; }
.attribute-row { display: flex; gap: 0.5rem; margin-bottom: 0.35rem; }
.attribute-row input { flex: 1 1 0; min-width: 0; }

.form-errors, .lock-banner {
  border: 1px solid var(--border);
  border-radius: var(--radius);
  padding: 0.75rem 1rem;
  margin-bottom: 1rem;
}
.form-errors { background: var(--fail-bg); color: var(--fail); border-color: var(--fail); }
.form-errors h2 { color: var(--fail); }
.lock-banner { background: var(--warn-bg); color: var(--warn); border-color: var(--warn); }

/* -- assist: a first-class feature, not a bare form --------------------- */

.assist-panel, .accept-proposal, .review-artifact, .provenance { margin-bottom: 2rem; }

.assist-panel {
  background: var(--surface);
  border: 1px solid var(--border);
  border-left: 3px solid var(--accent);
  border-radius: var(--radius);
  padding: 1rem 1.25rem;
}
.assist-panel h2 {
  display: flex; align-items: center; gap: 0.5rem;
  margin-bottom: 0.5rem;
}
.assist-panel h2::before {
  content: "";
  width: 8px; height: 8px; border-radius: 50%;
  background: var(--accent);
}
.assist-form { display: grid; gap: 0.75rem; margin-top: 0.75rem; }
.assist-form textarea { min-height: 5rem; }

/* -- the reading tree (page body) vs the navigation tree (side panel) --- */
/* Reading: comfortable, documentation visible, kind as a chip. Navigation:
   dense rows with a kind dot and a muted badge; defined further below in
   the .mw-* workbench section. */

.structure-tree, .structure-tree ul { list-style: none; margin: 0; padding: 0; }
.structure-tree ul { margin: 0.15rem 0 0 1.25rem; padding-left: 0.9rem; border-left: 1px solid var(--border-muted); }
.structure-tree li { margin: 0.4rem 0; }
.element-name, .state-name, .activity-name { font-weight: 600; color: var(--text); }
.element-documentation { color: var(--text-2); margin: 0.2rem 0 0; font-size: 13px; }
.element-edit { margin-left: 0.5rem; font-size: 12px; color: var(--text-3); }
.element-edit:hover { color: var(--accent); text-decoration: none; }

li.signal, li.interface, li.state, li.transition {
  padding: 0.35rem 0.6rem;
  border: 1px solid var(--border-muted);
  border-radius: var(--radius-sm);
  margin-bottom: 0.3rem;
  font-size: 13px;
  background: var(--surface);
}
.state-entry, .state-do, .state-exit {
  color: var(--text-2); margin-left: 0.5rem;
  font-family: var(--font-mono); font-size: 12px;
}

/* -- tables: requirements and traceability ------------------------------ */

table.requirements, table.traceability {
  border-collapse: collapse; width: 100%; margin-top: 0.5rem;
  background: var(--surface);
  border: 1px solid var(--border-muted);
  border-radius: var(--radius);
  /* Fixed layout: the text column gets the room it needs and the long opaque
     id column is truncated rather than pushing the table past its container. */
  table-layout: fixed;
}
table.requirements th, table.requirements td,
table.traceability th, table.traceability td {
  border: 1px solid var(--border-muted);
  padding: 0.5rem 0.65rem;
  text-align: left; vertical-align: top;
  font-size: 13px;
}
table.requirements th, table.traceability th {
  background: var(--surface-1);
  font-size: 11px; font-weight: 600;
  text-transform: uppercase; letter-spacing: 0.04em;
  color: var(--text-2);
}
/* Column budget: id narrow and truncated (full id on hover), reqId narrow,
   text widest; the remaining columns share what is left. */
table.requirements th:nth-child(1), table.traceability th:nth-child(1) { width: 14%; }
table.requirements th:nth-child(2), table.traceability th:nth-child(2) { width: 7%; }
table.requirements th:nth-child(3), table.traceability th:nth-child(3) { width: 42%; }
table.requirements th:nth-child(4), table.traceability th:nth-child(4) { width: 25%; }
table.requirements th:nth-child(5), table.traceability th:nth-child(5) { width: 12%; }
.req-id, .req-num { font-family: var(--font-mono); font-size: 12px; color: var(--text-2); }
.req-id { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.req-text { overflow-wrap: anywhere; }
.coverage-summary { color: var(--text-2); font-size: 13px; }
.relation { color: var(--text-2); }
li.allocation, li.activity-node { margin-bottom: 0.15rem; font-size: 12.5px; }
.none { font-style: normal; }

.unresolved-edge {
  border-left: 3px solid var(--fail);
  padding: 0.25rem 0.5rem;
  margin-bottom: 0.25rem;
  font-family: var(--font-mono); font-size: 12px;
}

/* -- state and activity -------------------------------------------------- */

.activity {
  border: 1px solid var(--border);
  border-radius: var(--radius);
  padding: 0.75rem 1rem;
  margin-bottom: 0.75rem;
  background: var(--surface);
}
.activity-name { margin-top: 0; }

/* -- diff / conflict / merge -------------------------------------------- */

.diff-add { color: var(--pass); }
.diff-remove { color: var(--fail); }
.diff-change { color: var(--warn); }
.diff-list li { font-family: var(--font-mono); font-size: 12px; margin-bottom: 0.15rem; }

.conflict {
  border: 1px solid var(--border);
  border-radius: var(--radius);
  padding: 0.75rem 1rem;
  margin-bottom: 1rem;
  background: var(--surface);
}
.conflict h2 { margin-top: 0; }
.conflict-columns { display: flex; gap: 1rem; }
.conflict-side { flex: 1 1 0; min-width: 0; }
.conflict-side h3 { margin-top: 0; }
.conflict-value {
  font-family: var(--font-mono); font-size: 12px;
  white-space: pre-wrap; overflow-wrap: anywhere;
  background: var(--surface-1);
  border: 1px solid var(--border-muted);
  padding: 0.5rem;
  border-radius: var(--radius-sm);
  margin: 0;
}

/* -- import / loss ------------------------------------------------------- */

.fidelity-note {
  background: var(--surface-1);
  border: 1px solid var(--border-muted);
  border-radius: var(--radius);
  padding: 0.6rem 0.75rem;
  color: var(--text-2); font-size: 12px;
}
.loss-list li {
  border-left: 3px solid var(--fail);
  padding: 0.25rem 0.5rem;
  margin-bottom: 0.35rem;
  background: var(--surface);
}
.loss-subject { font-family: var(--font-mono); font-weight: 600; }
.loss-verdict { color: var(--text-2); }
.loss-note { color: var(--text-2); font-size: 12px; }
.loss-accept li { margin-bottom: 0.35rem; }
.loss-accept label {
  display: flex; align-items: baseline; gap: 0.5rem;
  font-weight: 400; font-size: 13px; color: var(--text);
}

/* -- proposals / gate ---------------------------------------------------- */

.proposal-action { font-weight: 600; }
.proposal-subject { font-family: var(--font-mono); margin-left: 0.5rem; }
.proposal-rationale { margin: 0.25rem 0; color: var(--text-2); }
.confidence { color: var(--text-2); font-size: 12px; }
.decision { font-weight: 600; }
.verdict { font-weight: 600; }
.gate-run-link { display: block; color: var(--text); }
.gate-run-link:hover { color: var(--text); text-decoration: none; }
.gate-run-link code { margin: 0 0.3rem; }
.failures li { color: var(--fail); font-size: 13px; margin-bottom: 0.2rem; }

/* -- the three-pane workbench (JS enhancement) -------------------------- */

.mw-workbench { display: flex; gap: 1rem; align-items: flex-start; }

.mw-tree-panel, .mw-props-panel {
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  position: sticky;
  top: calc(var(--header-h) + 1.25rem);
  /* Sticky only sticks when the flex item is not stretched by the container:
     align-items on .mw-workbench is flex-start, and this is belt-and-braces. */
  align-self: flex-start;
  display: flex; flex-direction: column;
  overflow: hidden;
  /* A fixed viewport height (not just a max-height) keeps BOTH panes pinned for
     the whole scroll and leaves no dead column under a short properties panel. */
  height: calc(100vh - var(--header-h) - 2.5rem);
}
.mw-tree-panel { flex: 0 0 18rem; }
.mw-props-panel { flex: 0 0 18rem; }
.mw-content { flex: 1 1 auto; min-width: 0; }

.mw-panel-head {
  flex: 0 0 auto;
  display: flex; align-items: center;
  padding: 0.6rem 0.75rem;
  border-bottom: 1px solid var(--border-muted);
  background: var(--surface-1);
}
.mw-panel-title {
  font-size: 11px; font-weight: 600;
  letter-spacing: 0.06em; text-transform: uppercase;
  color: var(--text-2);
}

/* the tree's own scroll: the header and search stay put, only rows scroll */
.mw-tree {
  flex: 1 1 auto; overflow: auto;
  list-style: none; margin: 0; padding: 0.4rem 0.5rem 0.6rem;
}
.mw-tree ul {
  list-style: none;
  /* Tighter indentation so deep nodes keep enough room for a name. */
  margin: 0 0 0 0.7rem;
  padding: 0 0 0 0.5rem;
  border-left: 1px solid var(--border-muted);
}
.mw-tree li { margin: 0; }

/* dense, professional rows: 26-28px tall, hover and selected states */
.mw-tree .mw-node {
  display: flex; align-items: center; gap: 0.4rem;
  min-height: 27px;
  padding: 0.1rem 0.4rem;
  border-radius: var(--radius-sm);
  cursor: pointer;
  color: var(--text);
}
.mw-tree .mw-node:hover { background: var(--surface-2); }
.mw-tree .mw-node.mw-selected { background: var(--accent); color: var(--on-accent); }

.mw-tree .mw-toggle {
  flex: 0 0 auto; width: 0.9rem;
  text-align: center;
  color: var(--text-3); font-size: 11px;
  user-select: none; cursor: pointer;
}
.mw-node.mw-selected .mw-toggle { color: var(--on-accent); }

.mw-kind-dot {
  flex: 0 0 auto;
  width: 8px; height: 8px; border-radius: 50%;
  background: var(--kind-neutral);
}
.mw-node[data-kind="block"] .mw-kind-dot { background: var(--kind-block); }
.mw-node[data-kind="actor"] .mw-kind-dot { background: var(--kind-actor); }
.mw-node[data-kind="usecase"] .mw-kind-dot { background: var(--kind-usecase); }
.mw-node[data-kind="requirement"] .mw-kind-dot { background: var(--kind-requirement); }
.mw-node[data-kind="signal"] .mw-kind-dot { background: var(--kind-signal); }
.mw-node[data-kind="interface"] .mw-kind-dot { background: var(--kind-interface); }
.mw-node[data-kind="activity"] .mw-kind-dot { background: var(--kind-activity); }
.mw-node[data-kind="state"] .mw-kind-dot { background: var(--kind-state); }
.mw-node[data-kind="stateMachine"] .mw-kind-dot { background: var(--kind-statemachine); }

.mw-node-name {
  flex: 1 1 auto; min-width: 0;
  overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
  font-size: 13px;
}
.mw-kind-badge {
  flex: 0 0 auto;
  font-size: 10.5px; color: var(--text-3);
  background: var(--surface-1);
  border: 1px solid var(--border-muted);
  border-radius: 999px;
  padding: 0.03rem 0.4rem;
  margin-left: auto;
}
.mw-node.mw-selected .mw-kind-badge {
  background: rgba(255, 255, 255, 0.18);
  border-color: transparent;
  color: var(--on-accent);
}
/* A requirement's coverage state in the tree, shown as a text chip (never
   colour alone) pushed to the row's right edge like the kind badge. */
.mw-tree .mw-node .mw-tree-cov { margin-left: auto; flex: 0 0 auto; }

.mw-tree .mw-group-label {
  display: flex; align-items: center; gap: 0.4rem;
  padding: 0.35rem 0.4rem;
  font-size: 11px; font-weight: 600;
  letter-spacing: 0.05em; text-transform: uppercase;
  color: var(--text-2);
  cursor: pointer; user-select: none;
  border-radius: var(--radius-sm);
}
.mw-tree .mw-group-label:hover { background: var(--surface-2); color: var(--text); }
.mw-tree .mw-group-label .mw-toggle { width: 0.9rem; }

.mw-tree .mw-empty { color: var(--text-3); font-size: 12px; padding: 0.5rem 0.75rem; }

.mw-search {
  flex: 0 0 auto;
  margin: 0.6rem 0.75rem 0.4rem;
  font-family: var(--font-ui);
  font-size: 13px;
  color: var(--text);
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  padding: 0.4rem 0.6rem 0.4rem 1.9rem;
  background-image: url("data:image/svg+xml,%3Csvg%20xmlns=%22http://www.w3.org/2000/svg%22%20viewBox=%220%200%2024%2024%22%20fill=%22none%22%20stroke=%22%238b949e%22%20stroke-width=%222%22%20stroke-linecap=%22round%22%3E%3Ccircle%20cx=%2211%22%20cy=%2211%22%20r=%227%22/%3E%3Cline%20x1=%2221%22%20y1=%2221%22%20x2=%2216.2%22%20y2=%2216.2%22/%3E%3C/svg%3E");
  background-repeat: no-repeat;
  background-position: left 0.55rem center;
  background-size: 14px 14px;
}
.mw-search:focus-visible { border-color: var(--accent); outline: 2px solid var(--accent); outline-offset: 1px; }

/* properties pane */
.mw-props-body { flex: 1 1 auto; overflow: auto; padding: 0.75rem 1rem; }
.mw-props-body h3 { font-size: 14px; font-weight: 650; margin: 0 0 0.6rem; overflow-wrap: anywhere; }
.mw-props-body dl { margin: 0; }
.mw-props-body dt {
  font-size: 11px; font-weight: 600;
  color: var(--text-2);
  text-transform: uppercase; letter-spacing: 0.04em;
  margin-top: 0.75rem;
}
.mw-props-body dt:first-child { margin-top: 0; }
.mw-props-body dd { margin: 0.15rem 0 0; font-size: 13px; overflow-wrap: anywhere; }
.mw-props-body .mw-empty { color: var(--text-3); margin: 0.5rem 0 0; }
.mw-attr-list { list-style: none; margin: 0; padding: 0; }
.mw-attr { font-family: var(--font-mono); font-size: 12px; display: block; margin-bottom: 0.1rem; }

.mw-highlight {
  outline: 2px solid var(--accent);
  outline-offset: 2px;
  background: var(--accent-tint);
  border-radius: var(--radius-sm);
}

/* -- chrome: model switcher, identity chip, context bar ----------------- */
/* The global bar's model switcher is a pure-HTML <details> disclosure, so it
   works with JavaScript disabled exactly like the navigator links. */

.site-header .switcher { position: relative; }
.site-header .switcher summary {
  list-style: none;
  display: inline-flex; align-items: center; gap: 0.4rem;
  font-size: 13px; font-weight: 600; color: var(--text);
  background: var(--surface-1);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  padding: 0.28rem 0.65rem;
  cursor: pointer;
}
.site-header .switcher summary::-webkit-details-marker { display: none; }
.site-header .switcher summary::after { content: "\25be"; color: var(--text-3); }
.site-header .switcher[open] summary { border-color: var(--accent); }
.site-header .switcher .menu {
  position: absolute; top: calc(100% + 0.35rem); left: 0; z-index: 40;
  min-width: 15rem;
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  box-shadow: 0 10px 30px rgba(31, 35, 40, 0.14);
  padding: 0.35rem; margin: 0; list-style: none;
}
.site-header .switcher .menu a {
  display: block; padding: 0.4rem 0.6rem; border-radius: var(--radius-sm);
  color: var(--text); font-size: 13px;
}
.site-header .switcher .menu a:hover { background: var(--surface-1); text-decoration: none; }
.site-header .switcher .menu a.current {
  background: var(--accent-tint); color: var(--accent-strong); font-weight: 650;
}

.site-header .new-model {
  font-size: 12px; font-weight: 600; color: var(--on-accent);
  background: var(--accent); border: 1px solid var(--accent);
  border-radius: var(--radius); padding: 0.28rem 0.7rem;
}
.site-header .new-model:hover { background: var(--accent-strong); color: var(--on-accent); text-decoration: none; }

.site-header .chip {
  margin-left: auto;
  display: inline-flex; align-items: center; gap: 0.45rem;
  font-size: 12px; color: var(--text-2);
  background: var(--surface-1); border: 1px solid var(--border-muted);
  border-radius: 999px; padding: 0.28rem 0.75rem;
  white-space: nowrap;
}
.site-header .chip::before {
  content: ""; width: 7px; height: 7px; border-radius: 50%;
  background: var(--pass);
}

/* -- chrome: the left navigator ------------------------------------------ */
.rail .nav-group { margin-bottom: 1.1rem; }
.rail .nav-group-title {
  font-size: 11px; font-weight: 600;
  letter-spacing: 0.06em; text-transform: uppercase;
  color: var(--text-3);
  padding: 0 0.7rem 0.35rem;
}
.rail a.current {
  background: var(--accent-tint);
  color: var(--accent-strong);
  font-weight: 600;
}

/* -- chrome: the context bar --------------------------------------------- */
/* Muted and slim on purpose: it is CONTEXT ("Viewing X · version · commit"),
   never an editable control, so it must not look like the white-bordered
   form inputs below. */
.context-bar {
  display: flex; align-items: center; gap: 0.5rem; flex-wrap: wrap;
  font-size: 12px; color: var(--text-2);
  background: var(--surface-1);
  border: 1px solid var(--border-muted);
  border-radius: var(--radius);
  padding: 0.35rem 0.75rem;
  margin-bottom: 1.25rem;
}
.context-bar .ctx-label {
  font-size: 10px; font-weight: 700;
  letter-spacing: 0.06em; text-transform: uppercase;
  color: var(--text-3);
}
.context-bar .ctx-project { font-weight: 650; color: var(--text); }
.context-bar .ctx-sep { color: var(--text-3); }
.context-bar code { color: var(--text-2); }
.context-bar .ctx-actions { margin-left: auto; display: flex; gap: 0.5rem; }

/* -- chrome: the overview summary and section links ---------------------- */
.overview-summary {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(11rem, 1fr));
  gap: 0.6rem;
  margin-bottom: 1.25rem;
}
.overview-card {
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  padding: 0.7rem 0.9rem;
}
.overview-card .ov-label {
  display: block; font-size: 11px; font-weight: 600;
  text-transform: uppercase; letter-spacing: 0.04em;
  color: var(--text-3); margin-bottom: 0.2rem;
}
.overview-card .ov-value { font-size: 18px; font-weight: 650; color: var(--text); }
.overview-card .ov-note { font-size: 12px; color: var(--text-2); }

.section-links { display: flex; flex-wrap: wrap; gap: 0.4rem; margin-bottom: 1.25rem; }
.section-links a {
  font-size: 12px; color: var(--text); background: var(--surface);
  border: 1px solid var(--border); border-radius: 999px; padding: 0.25rem 0.7rem;
}
.section-links a:hover { border-color: var(--accent); color: var(--accent); text-decoration: none; }
.section-links a.current { background: var(--accent-tint); border-color: var(--accent); color: var(--accent-strong); font-weight: 600; }

/* -- diagram ------------------------------------------------------------- */

svg .kind-header { fill: var(--text-2); }
svg g.node text { font-family: var(--font-ui); }
g.mw-selected-node rect { stroke: var(--accent); stroke-width: 3px; }
"#;

pub fn html_response(status: StatusCode, markup: Markup) -> Response {
    (status, Html(markup.into_string())).into_response()
}

/// The sections of a model, in navigator order. The key is the route segment and the
/// label is what the navigator shows; the current section is marked on every page inside a
/// model.
pub const SECTIONS: &[(&str, &str)] = &[
    ("overview", "Overview"),
    ("structure", "Structure"),
    ("requirements", "Requirements"),
    ("traceability", "Traceability"),
    ("diagram", "Diagram"),
    ("checks", "Checks"),
    ("changes", "Changes"),
    ("proposals", "Proposals"),
    ("import", "Import"),
    ("assist", "Assist"),
];

/// Everything the shell needs to render global navigation: the reachable models, the current
/// one, the current section and (for a commit-scoped view) the branch and commit being shown.
pub struct Nav {
    pub projects: Vec<String>,
    pub current: Option<String>,
    pub section: Option<&'static str>,
    pub branch: Option<String>,
    pub commit: Option<String>,
}

impl Nav {
    /// The models a caller may reach, filtered exactly as the JSON list handler filters them, so
    /// the navigator can never disclose a project name the caller is not in scope for.
    pub fn load(
        state: &ApiState,
        identity: &Identity,
        current: Option<&str>,
    ) -> Result<Nav, ApiError> {
        let mut projects = Vec::new();
        for project in state.store.list_projects().map_err(map_store_error)? {
            if identity.may_reach(&project.name) {
                projects.push(project.name);
            }
        }
        projects.sort();
        Ok(Nav {
            projects,
            current: current.map(str::to_string),
            section: None,
            branch: None,
            commit: None,
        })
    }

    /// The sign-in and error pages have no model context.
    pub fn none() -> Nav {
        Nav {
            projects: Vec::new(),
            current: None,
            section: None,
            branch: None,
            commit: None,
        }
    }
}

/// The address of a section for the current model, preserving the branch/commit being viewed so
/// moving between commit-scoped sections keeps the same version. Sections that are project-wide
/// (checks, changes, proposals, import, assist) ignore the query, so carrying it is harmless.
pub fn section_href(nav: &Nav, key: &str) -> String {
    let project = nav.current.as_deref().unwrap_or("");
    let mut url = format!("/ui/projects/{}/{}", crate::ui::urlencode(project), key);
    if let Some(commit) = &nav.commit {
        url.push_str(&format!("?commit={}", crate::ui::urlencode(commit)));
    } else if let Some(branch) = &nav.branch {
        url.push_str(&format!("?branch={}", crate::ui::urlencode(branch)));
    }
    url
}

/// The single page shell: the global bar, the left navigator (inside a model) and the content
/// area with its context bar. Everything is server-rendered links - the switcher is a plain
/// HTML \<details\> disclosure - so the workbench is fully usable with JavaScript disabled.
pub fn shell(
    title: &str,
    nav: &Nav,
    subject: Option<&str>,
    can_administer: bool,
    mechanism: &str,
    body: Markup,
) -> Markup {
    shell_with_main_class(title, nav, subject, can_administer, mechanism, "", body)
}

/// The same shell with a class on the content `<main>` element, for pages that lay out
/// across the whole width (the project card grid, the traceability matrix).
pub fn shell_with_main_class(
    title: &str,
    nav: &Nav,
    subject: Option<&str>,
    can_administer: bool,
    mechanism: &str,
    main_class: &str,
    body: Markup,
) -> Markup {
    html! {
        (DOCTYPE)
        html lang="en" {
            head {
                meta charset="utf-8";
                meta name="viewport" content="width=device-width, initial-scale=1";
                title { (title) }
                style { (PreEscaped(STYLE)) }
            }
            body {
                header class="site-header" {
                    a class="brand" href="/ui" { "modelwrite" }
                    @if !nav.projects.is_empty() {
                        details class="switcher" {
                            summary {
                                @if let Some(current) = &nav.current { (current) }
                                @else { "switch model" }
                            }
                            ul class="menu" {
                                @for project in &nav.projects {
                                    li {
                                        a.current[nav.current.as_deref() == Some(project.as_str())]
                                          href={ "/ui/projects/" (crate::ui::urlencode(project.as_str())) "/overview" } {
                                            (project)
                                        }
                                    }
                                }
                            }
                        }
                    }
                    @if can_administer {
                        a class="new-model" href="/ui" { "New model" }
                    }
                    span class="chip" {
                        @if let Some(subject) = subject {
                            (subject) " via " (mechanism)
                        } @else {
                            "not signed in"
                        }
                    }
                }
                div class="layout" {
                    @if nav.current.is_some() {
                        (left_nav(nav))
                    }
                    main class=(main_class) {
                        @if nav.current.is_some() {
                            (context_bar(nav))
                        }
                        (body)
                    }
                }
            }
        }
    }
}

/// The left navigator: the MODELS group (one click to switch model) and the SECTIONS group for
/// the current model, the current section marked. Server-rendered links, never a JavaScript menu.
fn left_nav(nav: &Nav) -> Markup {
    html! {
        nav class="rail" aria-label="workbench" {
            div class="nav-group" {
                div class="nav-group-title" { "Models" }
                ul {
                    @for project in &nav.projects {
                        li {
                            a.current[nav.current.as_deref() == Some(project.as_str())]
                              href={ "/ui/projects/" (crate::ui::urlencode(project.as_str())) "/overview" } {
                                (project)
                            }
                        }
                    }
                }
            }
            div class="nav-group" {
                div class="nav-group-title" { "Sections" }
                ul {
                    @for (key, label) in SECTIONS.iter().copied() {
                        li {
                            a.current[nav.section == Some(key)] href=(section_href(nav, key)) { (label) }
                        }
                    }
                }
            }
        }
    }
}

/// The context bar: which project, version (branch) and commit are being viewed. The version
/// actions (new version, compare, make current) attach here in the following task; for now it
/// only displays the context clearly.
fn context_bar(nav: &Nav) -> Markup {
    html! {
        div class="context-bar" {
            span class="ctx-label" { "Viewing" }
            @if let Some(current) = &nav.current {
                span class="ctx-project" { (current) }
            }
            @if let Some(branch) = &nav.branch {
                span class="ctx-sep" { "·" }
                span { "version " code { (branch) } }
            }
            @if let Some(commit) = &nav.commit {
                span class="ctx-sep" { "·" }
                span { "commit " code { (short_hash(commit)) } }
            }
            span class="ctx-actions" {}
        }
    }
}

fn short_hash(hash: &str) -> &str {
    hash.get(..8).unwrap_or(hash)
}

pub fn sign_in_page(mechanism: &str, message: &str) -> Response {
    let body = html! {
        section class="sign-in" {
            h1 { "Sign in required" }
            p { "This workbench is protected by " (mechanism) " authentication." }
            p { (message) }
            p { "Present your credentials with the request (for example an " code { "Authorization: Bearer" } " header) and reload." }
        }
    };
    html_response(
        StatusCode::UNAUTHORIZED,
        shell(
            "sign in — modelwrite",
            &Nav::none(),
            None,
            false,
            mechanism,
            body,
        ),
    )
}

pub fn error_page(
    status: StatusCode,
    subject: Option<&str>,
    mechanism: &str,
    message: &str,
) -> Response {
    let body = html! {
        section class="error" {
            h1 { (status.as_u16()) " " (status.canonical_reason().unwrap_or("error")) }
            p { (message) }
        }
    };
    html_response(
        status,
        shell(
            "error — modelwrite",
            &Nav::none(),
            subject,
            false,
            mechanism,
            body,
        ),
    )
}
