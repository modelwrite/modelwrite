// SPDX-License-Identifier: AGPL-3.0-or-later
//! The shared page shell: the header, the navigation rail, the inline stylesheet, and the
//! sign-in and error renderings.
//!
//! There is deliberately no build step and no asset pipeline. The stylesheet is inlined in
//! the page, and every dynamic value is spliced through maud, which escapes it by
//! construction. A value that reaches a page from a model, a commit message, a branch name
//! or an error therefore cannot execute as markup: the only unescaped content is the
//! developer-written stylesheet, wrapped in `maud::PreEscaped` on purpose.

use std::collections::HashSet;

use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};
use maud::{html, Markup, PreEscaped, DOCTYPE};

use crate::api::{map_store_error, ApiState};
use crate::auth::{Identity, Permission};
use crate::error::ApiError;
use crate::store::{Commit, Store};

/// The ONE stylesheet the whole product wears: the site, the workbench pages, the presentation
/// view and the standalone SVG export all take their palette from this block. It is
/// `pub(crate)` for exactly that reason - ui/export.rs resolves the tokens below by name rather
/// than restating them, so a rebrand cannot leave a downloaded artefact wearing the old colours.
pub(crate) const STYLE: &str = r#"
/* =========================================================================
   modelwrite design system
   Hand-written, no build step, nothing fetched. Every colour is a custom
   property below so the whole UI shares one palette.
   ========================================================================= */

:root {
  color-scheme: light;

  /* ===== the shared instrument palette =====================================
     These literals are mirrored in website/styles.css, whose token block is
     marked "Palette marker: instrument-2026-09". One palette, two surfaces:
     the site and the workbench are one product and must not look like two.
     Change a literal here and change it there. */

  /* -- accent: the ONE brand colour. Used for the primary action, the
     selected tree row, links and the focus ring. Everything else is neutral
     so the accent always means "interactive / active". */
  --accent: #0b6e99;
  --accent-strong: #075a7e;
  --accent-tint: #e6f2f7;
  --on-accent: #ffffff;

  /* -- chrome: the graphite instrument face across the top of every page. It
     is the same face the site wears, and it is the only dark surface in the
     product. */
  --chrome: #101418;
  --chrome-raised: #1a2026;
  --chrome-ink: #e7edf2;
  --chrome-muted: #9aa5ae;
  --chrome-line: #2a323a;
  --accent-bright: #5cc6de;

  /* -- neutrals: surface = raised sheet, surface-1 = canvas and muted well,
     surface-2 = hover, border = hairline, border-muted = faint rule.
     text / text-2 / text-3 are three measured ink steps. All three clear
     WCAG AA (4.5:1) on every surface they are used on; --text-3 was #6e7781
     (4.09:1 on the canvas) and is now #5f6973 (5.21:1). */
  --surface: #ffffff;
  --surface-1: #f5f7f9;
  --surface-2: #e9edf1;
  --border: #cfd7de;
  --border-muted: #e4e9ee;
  --text: #101418;
  --text-2: #49535c;
  --text-3: #5f6973;

  /* -- semantics: PASS / FAIL / UNKNOWN / WARN. Each is a tint plus an ink so
     a chip is readable without relying on colour alone (a border and a text
     label always accompany the tint). */
  --pass: #116b34;
  --pass-bg: #e4f2e9;
  --pass-ink: #0d5228;
  --fail: #a3231c;
  --fail-bg: #fae9e7;
  --unknown: #5f6973;
  --unknown-bg: #edf1f4;
  --warn: #7a5200;
  --warn-bg: #fbf1da;

  /* -- kinds: one colour per element kind. Used for the small dot in the
     containment tree and the diagram so the same kind is the same colour
     everywhere. Unknown kinds fall back to the neutral grey. */
  --kind-block: #0b6e99;
  --kind-actor: #6b3fa0;
  --kind-usecase: #075a7e;
  --kind-requirement: #7a5200;
  --kind-signal: #116b34;
  --kind-interface: #0a6e80;
  --kind-activity: #9a4a0a;
  --kind-state: #9c2f6e;
  --kind-statemachine: #a3231c;
  --kind-neutral: #5f6973;

  /* -- kind fills: the very light tint of each kind ink, used as the node fill
     in the diagram. The diagram build writes a per-kind fill as a presentation
     attribute; a CSS declaration beats a presentation attribute, so these rules
     are what actually paint the boxes and the palette stays owned here. */
  --kind-block-bg: #e6f2f7;
  --kind-actor-bg: #efe9f6;
  --kind-usecase-bg: #e3eef4;
  --kind-requirement-bg: #fbf1da;
  --kind-signal-bg: #e4f2e9;
  --kind-interface-bg: #e3f0f3;
  --kind-activity-bg: #faeee4;
  --kind-state-bg: #f8e9f2;
  --kind-statemachine-bg: #fae9e7;
  --kind-neutral-bg: #ffffff;

  /* -- type: a system UI face for everything (no serif), a monospace face for
     ids, hashes and code. */
  --font-ui: system-ui, -apple-system, "Segoe UI", Roboto, Helvetica, Arial, sans-serif;
  --font-mono: ui-monospace, "SFMono-Regular", "Cascadia Code", "JetBrains Mono", Consolas, "Liberation Mono", Menlo, monospace;

  /* -- square corners: a measuring instrument does not have rounded ones. */
  --radius: 2px;
  --radius-sm: 2px;
  --header-h: 2.9rem;
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
  background: var(--chrome);
  border-bottom: 1px solid var(--chrome-line);
  position: sticky; top: 0; z-index: 20;
}
.site-header .brand {
  display: inline-flex; align-items: center; gap: 0.5rem;
  font-family: var(--font-mono);
  font-weight: 650; font-size: 13px; letter-spacing: 0.02em;
  color: var(--chrome-ink);
}
.site-header .brand::before {
  content: "";
  flex: 0 0 auto;
  width: 16px; height: 16px;
  background-image: url("data:image/svg+xml,%3Csvg%20xmlns='http://www.w3.org/2000/svg'%20width='16'%20height='16'%20viewBox='0%200%2016%2016'%20fill='none'%3E%3Ccircle%20cx='4'%20cy='8'%20r='2.4'%20fill='%235cc6de'/%3E%3Ccircle%20cx='12'%20cy='4'%20r='2.4'%20fill='%235cc6de'%20opacity='0.45'/%3E%3Ccircle%20cx='12'%20cy='12'%20r='2.4'%20fill='%235cc6de'%20opacity='0.45'/%3E%3Cpath%20d='M6.2%207%209.8%204.7M6.2%209l3.6%202.3'%20stroke='%235cc6de'%20stroke-width='1.2'%20stroke-linecap='round'/%3E%3C/svg%3E");
  background-repeat: no-repeat;
  background-size: contain;
}
.site-header .brand:hover { color: var(--accent-bright); text-decoration: none; }
.site-header .project {
  font-size: 12px; color: var(--chrome-ink);
  background: var(--chrome-raised);
  border: 1px solid var(--chrome-line);
  padding: 0.12rem 0.55rem;
  border-radius: var(--radius);
  max-width: 18rem; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
}
.site-header .who { margin-left: auto; color: var(--chrome-muted); font-size: 12px; }

/* -- chrome: layout and rail -------------------------------------------- */

.layout { display: flex; min-height: calc(100vh - var(--header-h)); }
.rail {
  flex: 0 0 11rem;
  border-right: 1px solid var(--border);
  padding: 1rem 0.75rem;
  background: var(--surface-1);
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
  padding-bottom: 0.35rem;
  margin-bottom: 0.75rem;
  border-bottom: 1px solid var(--border);
  letter-spacing: 0.02em;
  text-transform: uppercase;
  font-size: 12px;
  color: var(--text-2);
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
  gap: 0;
  border-top: 1px solid var(--border);
  border-left: 1px solid var(--border);
  background: var(--surface);
}
/* gap: 0 with a rule on two sides of every cell gives a true ruled grid and
   leaves no filled empty cell at the end of an incomplete row. The cell is the
   ruled box and a column: the whole-card link is its body, and the health band
   is its footer. The column is the fallback; where the engine has subgrid the
   cell takes its two rows from the grid, so the body and the band are one
   height each across a row. */
ul.projects li {
  margin: 0; padding: 0;
  display: flex; flex-direction: column;
  border: none;
  border-right: 1px solid var(--border);
  border-bottom: 1px solid var(--border);
  background: var(--surface);
}
/* Subgrid is the alignment, not a flourish. In a column of its own, the band's
   height is its own text's, so a one-line band sits shorter than a wrapped one
   and the rule over the bands runs ragged across the row - and the top of a
   band is the line the eye follows when it scans a row. Sharing the grid's two
   tracks gives every cell in a row one body height and one band height, so the
   band rule and the band itself are a single horizontal line. */
@supports (grid-template-rows: subgrid) {
  ul.projects li {
    display: grid;
    grid-template-rows: subgrid;
    grid-row: span 2;
  }
}

ul.branches li, ul.proposals li, ul.proposal-list li {
  border: 1px solid var(--border);
  border-radius: var(--radius);
  padding: 0.75rem 1rem;
  margin-bottom: 0.5rem;
  background: var(--surface);
}
ul.proposals li h2, ul.proposal-list li h2 { margin-top: 0; }

a.project-card {
  position: relative;
  display: flex; flex-direction: column; gap: 0.3rem;
  flex: 1 1 auto;
  padding: 0.8rem 0.9rem 0.8rem 1.05rem;
  border: none;
  border-radius: 0;
  background: var(--surface);
  color: var(--text);
}
/* The kind mark: a petrol index tab down the left edge of every project, so a
   row of projects reads as a set of instruments and the eye can run down the
   column. It is decoration with a job, not an icon, and it is drawn in CSS -
   no image, no request. */
a.project-card::before {
  content: "";
  position: absolute; left: 0; top: 0; bottom: 0;
  width: 3px;
  background: var(--accent);
  opacity: 0.9;
}
/* The scope mark: a square at the top right of every card, the same square on
   every card, so the wall is legible as a grid even before it is read.
   It is DECORATION and NOT a control, so it carries no label: it is a ::after,
   with no DOM node, no tab stop, nothing to select and nothing for a screen
   reader to reach (its content is empty), and the card around it is one link.
   It reads as an orphaned checkbox only because a small square outline is what
   a checkbox looks like. It is the registration mark of the wall; this comment
   says so, because there is no control here to label. */
a.project-card::after {
  content: "";
  position: absolute; right: 0.7rem; top: 0.7rem;
  width: 7px; height: 7px;
  border: 1px solid var(--text-3);
}
a.project-card:hover {
  background: var(--accent-tint);
  text-decoration: none;
}
a.project-card:hover::before { background: var(--accent-strong); }
a.project-card:focus-visible { outline: 2px solid var(--accent); outline-offset: -2px; }
.project-name { font-family: var(--font-mono); font-weight: 650; font-size: 13px; letter-spacing: -0.01em; color: var(--text); }
a.project-card:hover .project-name { color: var(--accent-strong); }
.project-meta { color: var(--text-2); font-size: 11.5px; font-family: var(--font-mono); }
.project-meta .dot { color: var(--text-3); margin: 0 0.3rem; }
.project-message { color: var(--text); font-size: 13px; }
.project-author { color: var(--text-3); font-size: 12px; }
.project-open {
  margin-top: 0.4rem;
  font-size: 11px; font-weight: 650; color: var(--accent);
  letter-spacing: 0.08em; text-transform: uppercase;
  display: inline-flex; align-items: center; gap: 0.3rem;
}

/* -- the per-card health readout ----------------------------------------- */
/* The front door's answer to "is this model any good?": the NAMED gap counts the
   health view computes, for this project's default branch head. There is no score
   and no percentage here on purpose - this product names gaps, it never
   summarises them into one number - so the band lists the named counts, with a
   short label each, and links to the view that names every one of them.
   A clean model is plainly clean: pass ink, the words "no gaps", no band.
   A model with gaps wears the warn band, so twelve cards triage in one glance.
   A model with nothing to measure says so and wears neither, because
   "nothing to measure" is not "nothing wrong". No colour is introduced: this
   block reuses the tokens above and nothing else. */
.project-health {
  display: flex; flex-wrap: wrap; align-items: baseline; align-content: flex-start;
  gap: 0.35rem;
  margin: 0;
  padding: 0.45rem 0.9rem 0.45rem 1.05rem;
  border-top: 1px solid var(--border-muted);
  font-family: var(--font-mono);
  font-size: 11.5px;
  color: var(--text-2);
}
.project-health .dot { color: inherit; opacity: 0.55; margin: 0; }
/* A count and the separator that introduces it are ONE wrapping unit, so the
   dot always travels with the count it precedes and a band that wraps is never
   left ending a line on a dangling separator. The unit carries the same 0.35rem
   inside it that the band carries between units, so the spacing is unchanged. */
.project-health .health-gap {
  display: inline-flex; align-items: baseline; gap: 0.35rem;
}
.project-health .health-label {
  font-size: 10px; font-weight: 700;
  letter-spacing: 0.06em; text-transform: uppercase;
}
.project-health .health-link {
  margin-left: auto;
  font-family: var(--font-ui);
  font-weight: 650;
  color: var(--accent);
  white-space: nowrap;
}
.project-health.is-clean { color: var(--pass); font-family: var(--font-ui); }
.project-health.is-clean .health-verdict { font-weight: 650; }
.project-health.is-unmeasured { color: var(--text-3); font-family: var(--font-ui); }
.project-health.has-gaps {
  background: var(--warn-bg);
  border-top-color: var(--warn);
  border-left: 3px solid var(--warn);
  color: var(--warn);
  font-weight: 600;
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

/* A whole-screen verdict line: pass (green ink) or fail (red ink), never colour alone
   (the word always accompanies the tint). */
.check-passed { color: var(--pass); font-weight: 600; }
.check-failed { color: var(--fail); font-weight: 600; }

/* -- the STPA method-coverage state: three states, never two ------------ */
/* The same three states, the same meaning and the same tokens as the project-health
   band: is-clean (pass), has-gaps (fail), and is-unmeasured - the analysis has not been
   performed, so nothing was measured over. "Nothing to measure" is not "nothing wrong",
   so the unmeasured state never wears the pass colour and never shows a zero. A check
   with no subject is MARKED, in words, not rendered as a count. */
.stpa-state {
  font-family: var(--font-ui); font-weight: 600;
  padding: 0.75rem 1rem; border-radius: var(--radius);
  margin: 0 0 0.75rem;
}
.stpa-state.is-unmeasured {
  background: var(--warn-bg);
  border-left: 3px solid var(--warn);
  color: var(--warn);
}
.stpa-state.has-gaps { color: var(--fail); }
.stpa-state.is-clean { color: var(--pass); }
.model-section.is-unmeasured { border-left: 3px solid var(--warn); }
.check-unmeasured, .cannot-evaluate { color: var(--text-3); }
.check-unmeasured { font-weight: 500; }
.cannot-evaluate { margin: 0.35rem 0 0; }

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
/* A long allocation target (an opaque element id) wraps inside the column so it
   never bleeds into the next one; the full value stays on hover via its title. */
li.allocation { overflow-wrap: anywhere; }
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
.loss-accept-summary { font-size: 13px; color: var(--text); margin: 0 0 0.5rem; }
.loss-accept-summary strong { font-size: 15px; }
.loss-accept-summary .mw-accept-count { font-weight: 600; color: var(--accent-strong); }
.loss-select-all { margin: 0.25rem 0 0.5rem; }
.loss-select-all label { font-size: 13px; font-weight: 600; color: var(--text); }
.accept-actions { display: flex; gap: 0.5rem; flex-wrap: wrap; margin-top: 0.75rem; }
.accept-actions button[name="accept_all"] { background: var(--pass); border-color: var(--pass); }
.accept-actions button[name="accept_all"]:hover { background: var(--pass-ink); border-color: var(--pass-ink); }

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
  font-size: 13px; font-weight: 600; color: var(--chrome-ink);
  background: var(--chrome-raised);
  border: 1px solid var(--chrome-line);
  border-radius: var(--radius);
  padding: 0.28rem 0.65rem;
  cursor: pointer;
}
.site-header .switcher summary::-webkit-details-marker { display: none; }
.site-header .switcher summary::after { content: "\25be"; color: var(--chrome-muted); }
.site-header .switcher[open] summary { border-color: var(--accent); }
.site-header .switcher .menu {
  position: absolute; top: calc(100% + 0.35rem); left: 0; z-index: 40;
  min-width: 15rem;
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  box-shadow: 0 8px 24px rgba(16, 20, 24, 0.16);
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
  font-size: 12px; font-weight: 650; color: #101418;
  background: var(--accent-bright); border: 1px solid var(--accent-bright);
  border-radius: var(--radius); padding: 0.28rem 0.7rem;
}
.site-header .new-model:hover { background: #7fd6e8; border-color: #7fd6e8; color: #101418; text-decoration: none; }

.site-header .chip {
  margin-left: auto;
  display: inline-flex; align-items: center; gap: 0.45rem;
  font-size: 12px; color: var(--chrome-muted);
  background: var(--chrome-raised); border: 1px solid var(--chrome-line);
  border-radius: var(--radius); padding: 0.28rem 0.75rem;
  white-space: nowrap;
}
.site-header .chip::before {
  content: ""; width: 7px; height: 7px; border-radius: 50%;
  background: var(--pass);
}

/* The global find-element search: one box in the header, on every project page. */
.site-search { display: flex; align-items: center; gap: 0.35rem; }
.site-search input[type="search"] { width: 15rem; max-width: 32vw; }
/* the search box is on the graphite face too, so it takes the dark control
   treatment rather than the light form default */
.site-header .site-search input[type="search"] {
  background: var(--chrome-raised); border-color: var(--chrome-line); color: var(--chrome-ink);
}
.site-header .site-search input[type="search"]::placeholder { color: var(--chrome-muted); }

/* -- global find-element results ------------------------------------------ */
ul.search-hits { list-style: none; margin: 0; padding: 0; }
li.search-hit {
  display: flex; align-items: baseline; gap: 0.6rem; flex-wrap: wrap;
  border: 1px solid var(--border); border-radius: var(--radius);
  padding: 0.5rem 0.75rem; margin-bottom: 0.4rem; background: var(--surface);
}
li.search-hit code { font-family: var(--font-mono); }
.hit-name { font-weight: 600; }
.hit-kind { color: var(--text-2); font-size: 12px; }
.hit-actions { margin-left: auto; display: inline-flex; gap: 0.6rem; }
.hit-actions a { font-size: 12px; font-weight: 600; }

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
  font-weight: 650;
  box-shadow: inset 3px 0 0 var(--accent);
}

/* -- chrome: the context bar --------------------------------------------- */
/* Muted and slim on purpose: it is CONTEXT ("Viewing X · version · commit"),
   never an editable control, so it must not look like the white-bordered
   form inputs below. */
.context-bar {
  display: flex; align-items: center; gap: 0.5rem; flex-wrap: wrap;
  font-size: 12px; color: var(--text-2);
  background: var(--surface);
  border: 1px solid var(--border);
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

main.mw-diagram { max-width: none; padding: 1.25rem 1.5rem; }

.diagram-toolbar {
  display: flex; flex-wrap: wrap; align-items: center; gap: 0.5rem;
  margin: 0.25rem 0 0.75rem;
}
.view-toggle {
  display: inline-flex; border: 1px solid var(--border); border-radius: var(--radius);
  overflow: hidden; background: var(--surface);
}
.view-option { padding: 0.3rem 0.75rem; font-size: 12.5px; color: var(--text-2); }
.view-option:hover { color: var(--text); text-decoration: none; }
.view-option.current { background: var(--accent-tint); color: var(--accent-strong); font-weight: 600; }
.diagram-controls { display: inline-flex; gap: 0.3rem; align-items: center; }
/* V1 visual outputs: the download and the presentation view. Server-rendered links, so both
   work with JavaScript disabled. */
.diagram-outputs { display: inline-flex; gap: 0.3rem; align-items: center; margin-left: auto; }
.diagram-output {
  font-family: var(--font-ui); font-size: 12.5px; font-weight: 600; color: var(--accent);
  background: var(--surface); border: 1px solid var(--border);
  border-radius: var(--radius-sm); padding: 0.22rem 0.6rem;
}
.diagram-output:hover { border-color: var(--accent); color: var(--accent-strong); text-decoration: none; }
.diagram-controls button, .kind-filter {
  font-family: var(--font-ui); font-size: 12.5px;
  border: 1px solid var(--border); background: var(--surface); color: var(--text-2);
  border-radius: var(--radius-sm); padding: 0.22rem 0.6rem; cursor: pointer;
}
.diagram-controls button:hover, .kind-filter:hover { border-color: var(--accent); color: var(--text); }
.kind-filter.active { background: var(--accent-tint); border-color: var(--accent); color: var(--accent-strong); font-weight: 600; }
.kind-count { color: var(--text-3); font-size: 11px; }
.mw-diagram-search {
  font-family: var(--font-ui); font-size: 12.5px;
  border: 1px solid var(--border); border-radius: var(--radius-sm);
  padding: 0.22rem 0.6rem; width: 12rem; background: var(--surface); color: var(--text);
}

.diagram-viewport {
  overflow: auto;
  height: calc(100vh - 16rem);
  min-height: 26rem;
  border: 1px solid var(--border);
  border-radius: var(--radius);
  background: var(--surface);
  position: relative;
}
.mw-diagram-svg { display: block; width: 100%; height: 100%; cursor: grab; }
.mw-diagram-svg.dragging { cursor: grabbing; }

.diagram-props {
  position: absolute; top: 0.75rem; right: 0.75rem; width: 15rem; max-width: calc(100% - 1.5rem);
  background: var(--surface); border: 1px solid var(--border); border-radius: var(--radius);
  padding: 0.7rem 0.9rem;
}
.diagram-props h3 { font-size: 14px; font-weight: 650; margin: 0 0 0.5rem; overflow-wrap: anywhere; }
.diagram-props dl { margin: 0; }
.diagram-props dt { font-size: 11px; text-transform: uppercase; letter-spacing: 0.04em; color: var(--text-3); margin-top: 0.4rem; }
.diagram-props dt:first-child { margin-top: 0; }
.diagram-props dd { margin: 0.05rem 0 0; font-size: 12.5px; overflow-wrap: anywhere; color: var(--text); }

svg g.node { cursor: pointer; }
/* rx is set to 6 in the SVG; the instrument has square corners. A CSS
   geometry property beats the attribute where it is supported, and degrades to
   the attribute where it is not. */
svg g.node .node-rect { stroke: var(--text-3); stroke-width: 1.5px; rx: 2px; fill: var(--kind-neutral-bg); }
svg g.node[data-mw-kind="block"] .node-rect { fill: var(--kind-block-bg); }
svg g.node[data-mw-kind="actor"] .node-rect { fill: var(--kind-actor-bg); }
svg g.node[data-mw-kind="usecase"] .node-rect { fill: var(--kind-usecase-bg); }
svg g.node[data-mw-kind="requirement"] .node-rect { fill: var(--kind-requirement-bg); }
svg g.node[data-mw-kind="signal"] .node-rect { fill: var(--kind-signal-bg); }
svg g.node[data-mw-kind="interface"] .node-rect { fill: var(--kind-interface-bg); }
svg g.node[data-mw-kind="activity"] .node-rect { fill: var(--kind-activity-bg); }
svg g.node[data-mw-kind="state"] .node-rect { fill: var(--kind-state-bg); }
svg g.node[data-mw-kind="stateMachine"] .node-rect { fill: var(--kind-statemachine-bg); }
svg g.node .node-name {
  font-family: var(--font-ui); font-size: 13px; fill: var(--text); pointer-events: none;
}
svg g.node .node-kind {
  font-family: var(--font-ui); font-size: 11px; fill: var(--text-2);
  text-transform: uppercase; letter-spacing: 0.04em; pointer-events: none;
}
svg g.node[data-mw-kind="block"] .node-rect { stroke: var(--kind-block); }
svg g.node[data-mw-kind="block"] .node-kind { fill: var(--kind-block); }
svg g.node[data-mw-kind="actor"] .node-rect { stroke: var(--kind-actor); }
svg g.node[data-mw-kind="actor"] .node-kind { fill: var(--kind-actor); }
svg g.node[data-mw-kind="usecase"] .node-rect { stroke: var(--kind-usecase); }
svg g.node[data-mw-kind="usecase"] .node-kind { fill: var(--kind-usecase); }
svg g.node[data-mw-kind="requirement"] .node-rect { stroke: var(--kind-requirement); }
svg g.node[data-mw-kind="requirement"] .node-kind { fill: var(--kind-requirement); }
svg g.node[data-mw-kind="signal"] .node-rect { stroke: var(--kind-signal); }
svg g.node[data-mw-kind="signal"] .node-kind { fill: var(--kind-signal); }
svg g.node[data-mw-kind="interface"] .node-rect { stroke: var(--kind-interface); }
svg g.node[data-mw-kind="interface"] .node-kind { fill: var(--kind-interface); }
svg g.node[data-mw-kind="activity"] .node-rect { stroke: var(--kind-activity); }
svg g.node[data-mw-kind="activity"] .node-kind { fill: var(--kind-activity); }
svg g.node[data-mw-kind="state"] .node-rect { stroke: var(--kind-state); }
svg g.node[data-mw-kind="state"] .node-kind { fill: var(--kind-state); }
svg g.node[data-mw-kind="stateMachine"] .node-rect { stroke: var(--kind-statemachine); }
svg g.node[data-mw-kind="stateMachine"] .node-kind { fill: var(--kind-statemachine); }

/* -- node glyphs: kind marks and declared 2525 symbols --------------------- */
/* The glyph path draws in currentColor, so the node's `color` token (set per kind below)
   is what colours it; the text keeps its own explicit fill and is unaffected. */
svg g.node { color: var(--text-3); }
svg g.node[data-mw-kind="block"] { color: var(--kind-block); }
svg g.node[data-mw-kind="actor"] { color: var(--kind-actor); }
svg g.node[data-mw-kind="usecase"] { color: var(--kind-usecase); }
svg g.node[data-mw-kind="requirement"] { color: var(--kind-requirement); }
svg g.node[data-mw-kind="signal"] { color: var(--kind-signal); }
svg g.node[data-mw-kind="interface"] { color: var(--kind-interface); }
svg g.node[data-mw-kind="activity"] { color: var(--kind-activity); }
svg g.node[data-mw-kind="state"] { color: var(--kind-state); }
svg g.node[data-mw-kind="stateMachine"] { color: var(--kind-statemachine); }
svg g.node .node-glyph { pointer-events: none; }
svg g.node .mw-2525-unknown { font-family: var(--font-ui); font-weight: 700; fill: var(--text); pointer-events: none; }

g.mw-selected-node rect { stroke: var(--accent); stroke-width: 3px; }
g.mw-selected-node .node-name { fill: var(--accent-strong); font-weight: 600; }
g.node.mw-hover rect { stroke-width: 2.5px; }
g.node.mw-filtered-out { display: none; }
g.node.mw-search-match .node-rect { stroke: var(--warn); stroke-width: 3px; }
/* Hover a node: it and its immediate neighbours stay bright, everything else dims. */
svg.dimmed g.node { opacity: 0.18; }
svg.dimmed g.node.mw-active { opacity: 1; }

svg .mw-edge { fill: none; stroke: var(--text-3); stroke-width: 1.3px; }
svg .mw-edge.containment { stroke: var(--text-2); }
svg .mw-edge.dependency { stroke: var(--accent); }
svg .mw-edge.flow { stroke: var(--kind-activity); }
svg .arrow-accent { fill: var(--accent); }
svg .arrow-containment { fill: var(--text-2); }
svg .arrow-flow { fill: var(--kind-activity); }
svg .arrow-neutral { fill: var(--text-3); }
svg g.edge .edge-label {
  display: none; font-family: var(--font-ui); font-size: 11px; fill: var(--text-2);
  pointer-events: none;
}
svg g.edge.mw-incident .edge-label, svg g.edge.mw-selected .edge-label { display: block; }
svg g.edge.mw-incident .mw-edge { stroke-width: 2.4px; }

svg g.dangling .dangling-shape { fill: var(--fail-bg); stroke: var(--fail); stroke-width: 1.5px; }
svg g.dangling .dangling-label { font-family: var(--font-ui); font-size: 11px; fill: var(--fail); font-weight: 600; pointer-events: none; }
svg g.dangling .dangling-id { font-family: var(--font-mono); font-size: 10px; fill: var(--fail); pointer-events: none; }

/* -- symbol report: declared-but-unmappable symbols, never silently boxed --- */
.symbol-report {
  margin-top: 0.75rem; border: 1px solid var(--border); border-left: 3px solid var(--warn);
  border-radius: var(--radius); padding: 0.6rem 0.9rem; background: var(--surface);
}
.symbol-report h2 { font-size: 13px; margin: 0 0 0.3rem; color: var(--text); }
.symbol-report p { font-size: 12px; color: var(--text-2); margin: 0 0 0.5rem; }
.symbol-report ul { margin: 0; padding-left: 1.2rem; }
.symbol-report li { font-size: 12px; color: var(--text-2); margin-bottom: 0.2rem; }
.symbol-report code { color: var(--warn); font-family: var(--font-mono); }


/* -- composition: subsystems, boundary, process --------------------------- */

ul.subsystems {
  list-style: none; margin: 0; padding: 0;
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(16rem, 1fr));
  gap: 0.75rem;
}
li.subsystem-card {
  border: 1px solid var(--border);
  border-radius: var(--radius);
  padding: 0.85rem 1rem;
  background: var(--surface);
  margin: 0;
}
.subsystem-head { display: flex; align-items: center; gap: 0.5rem; margin-bottom: 0.5rem; }
.subsystem-head .covered, .subsystem-head .uncovered { margin-left: auto; }
.role-chip {
  display: inline-block;
  font-weight: 600; font-size: 11px;
  padding: 0.04rem 0.45rem;
  border-radius: 999px;
  border: 1px solid var(--border);
  color: var(--text-2);
  background: var(--surface-1);
  white-space: nowrap;
  line-height: 1.5;
}
.subsystem-project { font-weight: 650; font-size: 14px; color: var(--text); }
.subsystem-revision { color: var(--text-2); font-size: 12px; margin-top: 0.15rem; }
.subsystem-revision code { color: var(--text-2); }
.subsystem-reason { color: var(--fail); font-size: 12px; margin: 0.4rem 0 0; }
.subsystem-bounds { margin-top: 0.5rem; }
.bounds-label {
  display: block; font-size: 11px; font-weight: 600;
  text-transform: uppercase; letter-spacing: 0.04em;
  color: var(--text-3); margin-bottom: 0.25rem;
}
.subsystem-bounds code.bound {
  display: inline-block;
  font-size: 12px;
  background: var(--surface-1);
  border: 1px solid var(--border-muted);
  border-radius: var(--radius-sm);
  padding: 0.02rem 0.4rem;
  margin: 0 0.2rem 0.2rem 0;
  overflow-wrap: anywhere;
}
.composition-summary { color: var(--text-2); font-size: 13px; margin: 0 0 0.75rem; }

.boundary-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(16rem, 1fr));
  gap: 0.75rem;
}
.boundary-panel {
  border: 1px solid var(--border);
  border-radius: var(--radius);
  padding: 0.85rem 1rem;
  background: var(--surface);
}
.boundary-panel h3 { margin-top: 0; }
.boundary-measured { border-left: 3px solid var(--pass); }
.boundary-asserted { border-left: 3px solid var(--warn); }
.boundary-panel ul { margin: 0; padding-left: 1.1rem; }
.boundary-panel li { margin-bottom: 0.35rem; font-size: 13px; }
/* -- variant impact (compare) -------------------------------------------- */

.impact-refs {
  width: 100%;
  border-collapse: collapse;
  font-size: 13px;
  margin: 0 0 1rem;
}
.impact-refs th {
  text-align: left;
  font-size: 11px; font-weight: 600;
  text-transform: uppercase; letter-spacing: 0.04em;
  color: var(--text-3);
  padding: 0.4rem 0.6rem;
  border-bottom: 1px solid var(--border);
}
.impact-refs td {
  padding: 0.5rem 0.6rem;
  border-bottom: 1px solid var(--border-muted);
  vertical-align: top;
}
.impact-ref.changed td { background: var(--warn-bg); }
.impact-role { font-weight: 600; white-space: nowrap; }
.impact-side { color: var(--text-2); }
.impact-side .impact-project { color: var(--text); font-weight: 600; }
.impact-side code { color: var(--text-2); }

.impact-changed, .impact-partial {
  display: inline-block;
  font-weight: 600; font-size: 11px;
  padding: 0.04rem 0.45rem;
  border-radius: 999px;
  border: 1px solid var(--warn);
  background: var(--warn-bg);
  color: var(--warn);
  white-space: nowrap;
  line-height: 1.5;
}
.impact-changed { margin-left: 0.4rem; }

ul.impact-reqs { list-style: none; margin: 0 0 0.75rem; padding: 0; }
li.impact-req {
  display: flex; align-items: baseline; gap: 0.5rem;
  padding: 0.45rem 0.6rem;
  border-bottom: 1px solid var(--border-muted);
}
li.impact-req .impact-req-id { font-family: var(--font-mono); font-weight: 600; }
li.impact-req .impact-req-name { color: var(--text-2); font-size: 13px; }
.impact-state { margin-left: auto; }

.impact-summary { color: var(--text); font-size: 13px; margin: 0.25rem 0 0.75rem; }

.process-flow {
  display: flex;
  align-items: stretch;
  gap: 0.5rem;
  flex-wrap: wrap;
  margin-top: 0.5rem;
}
.flow-layer {
  display: flex;
  flex-direction: column;
  gap: 0.5rem;
  justify-content: center;
}
.flow-arrow { align-self: center; color: var(--text-3); font-size: 18px; }
.flow-step {
  border: 1px solid var(--border);
  border-radius: var(--radius);
  padding: 0.6rem 0.8rem;
  background: var(--surface);
  min-width: 9rem;
  display: flex;
  flex-direction: column;
  gap: 0.3rem;
}
.step-name { font-weight: 650; font-size: 13px; }
.step-satisfies { color: var(--text-2); font-size: 12px; }
.flow-parallel-label {
  font-size: 10px; font-weight: 600;
  text-transform: uppercase; letter-spacing: 0.05em;
  color: var(--text-3); text-align: center;
}

/* -- chrome: the version selector and version actions -------------------- */
/* The version selector is a plain <details> disclosure inside the context bar,
   exactly like the model switcher in the top bar: server-rendered links, no
   JavaScript, the choice carried in the URL. */

.context-bar .version-switcher { position: relative; }
.context-bar .version-switcher summary {
  list-style: none;
  display: inline-flex; align-items: center; gap: 0.4rem;
  font-size: 12px; font-weight: 650; color: var(--text);
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  padding: 0.15rem 0.5rem;
  cursor: pointer;
}
.context-bar .version-switcher summary::-webkit-details-marker { display: none; }
.context-bar .version-switcher summary::after { content: "\25be"; color: var(--text-3); }
.context-bar .version-switcher[open] summary { border-color: var(--accent); }
.context-bar .version-switcher .menu {
  position: absolute; top: calc(100% + 0.3rem); left: 0; z-index: 40;
  min-width: 17rem; max-width: 26rem;
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  box-shadow: 0 8px 24px rgba(16, 20, 24, 0.16);
  padding: 0.35rem; margin: 0; list-style: none;
  max-height: 24rem; overflow-y: auto;
}
.context-bar .version-switcher .menu li {
  display: flex; align-items: center;
}
.context-bar .version-switcher .menu a {
  display: block; padding: 0.32rem 0.55rem; border-radius: var(--radius-sm);
  color: var(--text); font-size: 12.5px;
}
.context-bar .version-switcher .menu a:hover { background: var(--surface-1); text-decoration: none; }
.context-bar .version-switcher .menu a.branch, .context-bar .version-switcher .menu a.commit { flex: 1 1 auto; }
.context-bar .version-switcher .menu a.compare {
  flex: 0 0 auto; font-size: 11px; font-weight: 600; color: var(--accent);
  padding: 0.32rem 0.55rem;
}
.context-bar .version-switcher .menu a.current {
  background: var(--accent-tint); color: var(--accent-strong); font-weight: 650;
}
.context-bar .version-switcher .menu code { color: var(--text-3); font-size: 11px; }
.context-bar .version-switcher .menu a.branch code.tip { margin-left: 0.4rem; }
.context-bar .version-switcher .menu li.vs-label {
  font-size: 10px; font-weight: 700;
  text-transform: uppercase; letter-spacing: 0.06em;
  color: var(--text-3);
  padding: 0.4rem 0.55rem 0.1rem;
}

.context-bar .ctx-actions a.ctx-action {
  font-size: 12px; font-weight: 600; color: var(--accent);
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  padding: 0.15rem 0.55rem;
}
.context-bar .ctx-actions a.ctx-action:hover { border-color: var(--accent); text-decoration: none; }

/* -- the drop zone: the front door (E1) ---------------------------------- */
/* The first thing on the projects page and the whole of /onboard: one target,
   no format choice, no project-name box required first. It is a real
   multipart file upload, so the flow completes with JavaScript disabled; the
   inline script only adds drag-and-drop on top of the same input. */

.mw-dropzone {
  background: var(--surface);
  border: 1px dashed var(--border);
  border-left: 3px solid var(--accent);
  border-radius: var(--radius);
  padding: 1.1rem 1.5rem 1.25rem;
  margin-bottom: 1.75rem;
}
.mw-dropzone.mw-dragging { border-color: var(--accent); background: var(--accent-tint); }
.mw-dropzone h1 { margin-bottom: 0.4rem; }
.mw-dropzone-hint { color: var(--text-2); font-size: 13px; margin: 0 0 1rem; max-width: 46rem; }
.mw-dropzone-form { margin: 0; }
.mw-dropzone-form input[type="file"] {
  display: block;
  width: 100%;
  max-width: 40rem;
  font-family: var(--font-ui);
  font-size: 13px;
  color: var(--text);
  background: var(--surface-1);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  padding: 0.7rem 0.75rem;
}
.mw-dropzone-actions { display: flex; align-items: center; gap: 0.85rem; flex-wrap: wrap; margin: 0.9rem 0 0; }
.mw-dropzone-file {
  font-family: var(--font-mono);
  font-size: 12px;
  color: var(--accent-strong);
  min-height: 1rem;
}
.mw-dropzone-advanced { margin-top: 0.9rem; }
.mw-dropzone-advanced summary { font-size: 12px; color: var(--text-2); cursor: pointer; }
.mw-dropzone-advanced summary:hover { color: var(--text); }
.mw-dropzone-advanced .advanced-grid { display: flex; gap: 1.5rem; flex-wrap: wrap; margin-top: 0.75rem; }
.mw-dropzone-advanced .advanced-grid p { margin: 0; }
.mw-dropzone-advanced input[type="text"], .mw-dropzone-advanced select { max-width: 22rem; }

/* -- the onboarding result: the engine's numbers, in words ---------------- */

.onboard-summary {
  background: var(--surface);
  border: 1px solid var(--border);
  border-left: 3px solid var(--accent);
  border-radius: var(--radius);
  padding: 1rem 1.25rem;
  margin-bottom: 1.25rem;
}
.onboard-summary p { margin: 0 0 0.6rem; font-size: 14px; }
.onboard-summary p:last-child { margin-bottom: 0; }

.onboard-facts {
  width: 100%;
  border-collapse: collapse;
  margin: 0 0 1.25rem;
  font-size: 13px;
  background: var(--surface);
}
.onboard-facts th {
  text-align: left;
  font-size: 11px; font-weight: 600;
  text-transform: uppercase; letter-spacing: 0.04em;
  color: var(--text-3);
  padding: 0.4rem 0.6rem;
  border-bottom: 1px solid var(--border);
}
.onboard-facts th.num, .onboard-facts td.num { text-align: right; }
.onboard-facts td { padding: 0.4rem 0.6rem; border-bottom: 1px solid var(--border-muted); }
.onboard-facts td.num { font-family: var(--font-mono); }

ul.onboard-classes { list-style: none; margin: 0 0 1.25rem; padding: 0; }
ul.onboard-classes li {
  font-size: 13px;
  padding: 0.4rem 0.6rem;
  border-bottom: 1px solid var(--border-muted);
  background: var(--surface);
}
.onboard-classes .lc-count { font-family: var(--font-mono); font-weight: 650; color: var(--warn); }
.onboard-classes .lc-name { font-family: var(--font-mono); color: var(--text); }
.onboard-classes .lc-example { color: var(--text-3); font-size: 12px; }

.onboard-accept { margin: 0.5rem 0 1rem; }
.onboard-accept button { font-size: 15px; padding: 0.6rem 1.1rem; }
details.onboard-losses { margin-top: 0.5rem; }
details.onboard-losses > summary {
  cursor: pointer;
  font-size: 13px;
  font-weight: 600;
  color: var(--accent);
  padding: 0.3rem 0;
}
details.onboard-losses > summary:hover { color: var(--accent-strong); }

/* -- analyses: the library, one run, and the diff ------------------------- */
/* Every colour here is one of the tokens above; this block introduces no new
   token, so the analyses pages wear the same identity as the rest of the
   product. The severity chips reuse .covered / .uncovered / .mw-badge-unknown
   rather than defining a fourth way to say gap, clean and unmeasured. */

.analysis-run { display: block; }
.analysis-tallies { display: flex; flex-wrap: wrap; gap: 0.3rem; margin: 0.35rem 0 0.2rem; }
ul.findings { list-style: none; margin: 0; padding: 0; }
ul.findings li.finding {
  border: 1px solid var(--border-muted);
  border-left: 3px solid var(--border);
  border-radius: var(--radius);
  padding: 0.5rem 0.75rem;
  margin-bottom: 0.4rem;
  background: var(--surface);
}
.finding-subject { font-size: 13px; color: var(--text); }
.finding-id {
  font-family: var(--font-mono); font-size: 11px; color: var(--text-3);
  margin: 0.2rem 0 0; overflow-wrap: anywhere;
}
.finding-statement { margin: 0.2rem 0 0; font-size: 13px; color: var(--text); }
.finding-evidence { margin-top: 0.3rem; }
.finding-evidence summary { font-size: 12px; color: var(--text-2); cursor: pointer; }
.finding-evidence summary:hover { color: var(--accent-strong); }
.finding-evidence pre {
  margin: 0.3rem 0 0; padding: 0.5rem 0.7rem;
  background: var(--surface-1); border: 1px solid var(--border-muted);
  border-radius: var(--radius); font-size: 11.5px; overflow: auto;
}
.analysis-summary { font-size: 14px; font-weight: 600; color: var(--text); margin: 0.5rem 0 1rem; }
.diff-section { margin: 0.75rem 0 1.25rem; }

"#;

pub fn html_response(status: StatusCode, markup: Markup) -> Response {
    (status, Html(markup.into_string())).into_response()
}

/// The sections of a model, in navigator order. The key is the route segment and the
/// label is what the navigator shows; the current section is marked on every page inside a
/// model.
pub const SECTIONS: &[(&str, &str)] = &[
    ("overview", "Overview"),
    ("health", "Health"),
    ("analyses", "Analyses"),
    ("stpa", "STPA"),
    ("structure", "Structure"),
    ("composition", "Composition"),
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
    /// Every version (branch) of the current project, as (name, tip), for the version
    /// selector in the context bar.
    pub branches: Vec<(String, String)>,
    /// Recent commits reachable from the branch tips, newest first, so the selector can offer
    /// a commit as a version to switch to.
    pub recent_commits: Vec<Commit>,
    /// Whether the caller may write the current project, so the context bar never offers an
    /// action (new version, make current) the caller cannot perform.
    pub can_write: bool,
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
        for project in state
            .store_for(identity)
            .list_projects()
            .map_err(map_store_error)?
        {
            if identity.may_reach(&project.name) {
                projects.push(project.name);
            }
        }
        projects.sort();
        let (branches, recent_commits) = match current {
            Some(project) => load_versions(state, identity, project)?,
            None => (Vec::new(), Vec::new()),
        };
        Ok(Nav {
            projects,
            current: current.map(str::to_string),
            section: None,
            branch: None,
            commit: None,
            branches,
            recent_commits,
            can_write: identity.may(Permission::Write),
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
            branches: Vec::new(),
            recent_commits: Vec::new(),
            can_write: false,
        }
    }
}

/// The versions of a project: its branches and the recent commits reachable from those tips.
/// Both come from the store's own reads - no new store method exists for the workbench.
#[allow(clippy::type_complexity)]
fn load_versions(
    state: &ApiState,
    identity: &Identity,
    project: &str,
) -> Result<(Vec<(String, String)>, Vec<Commit>), ApiError> {
    let branches = state
        .store_for(identity)
        .list_branches(project)
        .map_err(map_store_error)?;
    let recent = recent_commits(state.store_for(identity).as_ref(), project, &branches)?;
    Ok((branches, recent))
}

/// Every commit reachable from a branch tip, deduplicated and ordered newest first. The walk
/// follows parents through the store's own `commit` reads and stops at a commit already seen,
/// so a cycle in stored data cannot loop forever.
fn recent_commits(
    store: &dyn Store,
    project: &str,
    branches: &[(String, String)],
) -> Result<Vec<Commit>, ApiError> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut out: Vec<Commit> = Vec::new();
    for (_name, tip) in branches {
        let mut stack = vec![tip.clone()];
        while let Some(hash) = stack.pop() {
            if !seen.insert(hash.clone()) {
                continue;
            }
            if let Some(commit) = store.commit(project, &hash).map_err(map_store_error)? {
                for parent in &commit.parents {
                    stack.push(parent.clone());
                }
                out.push(commit);
            }
        }
    }
    out.sort_by_key(|commit| std::cmp::Reverse(commit_time(commit)));
    out.truncate(15);
    Ok(out)
}

/// A commit's creation time as seconds since the epoch, for newest-first ordering.
fn commit_time(commit: &Commit) -> i64 {
    commit.created_at.parse::<i64>().unwrap_or(0)
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
                    a class="brand" href="https://modelwrite.org" target="_blank" rel="noopener" { "modelwrite" }
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
                    @if let Some(current) = &nav.current {
                        form class="site-search" method="get" action={ "/ui/projects/" (crate::ui::urlencode(current.as_str())) "/search" } {
                            @if let Some(branch) = &nav.branch {
                                input type="hidden" name="branch" value=(branch.as_str());
                            }
                            input type="search" name="q" placeholder="Find element" aria-label="Find an element";
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

/// The context bar: which project, version (branch) and commit are being viewed, plus the
/// version selector and the version actions. The selector is a plain `<details>` disclosure of
/// server-rendered links - one click to switch version, with the choice in the URL - so it works
/// with JavaScript disabled exactly like the model switcher.
fn context_bar(nav: &Nav) -> Markup {
    let project = nav.current.as_deref().unwrap_or("");
    let current_ref = current_ref(nav);
    html! {
        div class="context-bar" {
            span class="ctx-label" { "Viewing" }
            @if !project.is_empty() {
                span class="ctx-project" { (project) }
            }
            @if !nav.branches.is_empty() {
                (version_switcher(nav))
            }
            @if let Some(commit) = &nav.commit {
                span class="ctx-sep" { "·" }
                span { "commit " code { (short_hash(commit)) } }
            }
            span class="ctx-actions" {
                // Creating a version WRITES, so the link is offered only to a caller who may
                // write; the page itself enforces the same decision server-side.
                @if nav.can_write {
                    a class="ctx-action" href={ "/ui/projects/" (crate::ui::urlencode(project)) "/version/new" } { "New version" }
                }
                @if current_ref.is_some() {
                    a class="ctx-action" href={ "/ui/projects/" (crate::ui::urlencode(project)) "/compare?from=" (crate::ui::urlencode(current_ref.as_deref().unwrap_or("")) ) } { "Compare" }
                }
                // Make current merges the branch being viewed into the main line, so it is
                // offered only from a non-main branch and only to a writer.
                @if let Some(branch) = &nav.branch {
                    @if branch != "main" && nav.can_write {
                        a class="ctx-action" href={ "/ui/projects/" (crate::ui::urlencode(project)) "/version/make-current?candidate=" (crate::ui::urlencode(branch.as_str())) } { "Make current" }
                    }
                }
            }
        }
    }
}

/// The endpoint that identifies the version being viewed, for a compare `from`. A branch name
/// wins; a commit-scoped view without a branch falls back to the commit hash.
fn current_ref(nav: &Nav) -> Option<String> {
    nav.branch.clone().or_else(|| nav.commit.clone())
}

/// The version selector: a disclosure listing every branch (one click to switch, with the
/// version in the URL) and the recent commits (one click to open a historical commit). Each
/// other branch also offers a `compare` link with both sides pre-filled, so "did my change
/// break anything" is one click from the context bar.
fn version_switcher(nav: &Nav) -> Markup {
    let project = nav.current.as_deref().unwrap_or("");
    let current_ref = current_ref(nav);
    html! {
        details class="version-switcher" {
            summary {
                @if let Some(branch) = &nav.branch {
                    "version " code { (branch) }
                } @else if let Some(commit) = &nav.commit {
                    "commit " code { (short_hash(commit)) }
                } @else {
                    "switch version"
                }
            }
            ul class="menu" {
                @for (name, tip) in &nav.branches {
                    li {
                        a.branch.current[nav.branch.as_deref() == Some(name.as_str())]
                          href={ "/ui/projects/" (crate::ui::urlencode(project)) "/overview?branch=" (crate::ui::urlencode(name.as_str())) } {
                            (name) code class="tip" { (short_hash(tip)) }
                        }
                        @if let Some(from) = &current_ref {
                            @if nav.branch.as_deref() != Some(name.as_str()) {
                                a class="compare" href={ "/ui/projects/" (crate::ui::urlencode(project)) "/compare?from=" (crate::ui::urlencode(from.as_str())) "&to=" (crate::ui::urlencode(name.as_str())) } { "compare" }
                            }
                        }
                    }
                }
                @if !nav.recent_commits.is_empty() {
                    li class="vs-label" { "Recent commits" }
                    @for commit in &nav.recent_commits {
                        li {
                            a class="commit" href={ "/ui/projects/" (crate::ui::urlencode(project)) "/overview?commit=" (crate::ui::urlencode(commit.hash.as_str())) } {
                                code { (short_hash(&commit.hash)) }
                                @if !commit.message.is_empty() { " " (commit.message.as_str()) }
                            }
                        }
                    }
                }
            }
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
