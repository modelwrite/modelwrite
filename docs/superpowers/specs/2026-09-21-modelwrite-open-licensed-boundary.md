# The open-versus-licensed boundary

**Status:** decided by the owner. The line is one sentence:

> **Free to prove your model. Paid to govern the data.**

## Who the open tier serves

An engineer on a project delivering to an RFT. Their question is *"is my model good?"* - and their answer must never cost anything, because the person asking is not the person with the budget.

**Open, forever:**

1. **The readers** - XMI and SysML v2 textual import, with the full loss report. Nothing mapped silently, everything named.
2. **The assessment** - the gate, coverage, traceability, provenance, and the compositional gate.
3. **THE GRAPH AND ITS GAPS - headline, not appendix.** The original graph work found orphaned items AND orphaned groups: elements with no edges, groups with no connection to the rest of the model, dangling requirements, the disconnected component. This is the analytic the owner called "super important", and it is what an engineer delivering to an RFT needs most: a named list of what is floating, what is isolated and what is missing, before a reviewer finds it. The engine already measures it (isolated nodes, disconnected components, uncovered requirements) - the open tier must PRESENT it as the first model-health analytic.
4. **Model health by baseline** - coverage, orphans, churn as a trend line towards a design review.
5. **The analytics schema and every transport** - REST, CLI, MCP, Python, the Parquet/CSV/NDJSON exports, OpenMetrics. The schema itself is permissively licensed so anyone can build a connector.
6. **The basis rule everywhere** - every number carries what it was computed over and what was not carried.
7. Self-hosted, single deployment, the operator's own auth.

## What the enterprise pays for

What an organisation buys when the data must be GOVERNED, not merely read:

1. **Identity** - single sign-on (OIDC/JWKS) against the organisation's provider.
2. **Row-level security by classification marking** - who may see what, enforced at the row, not the page.
3. **A record of who pulled what** - the pull audit. Open tools do not know who asked; an enterprise must.
4. **Scheduled delivery into the customer's lake** - managed feeds, not scripts someone forgot to run.
5. **Connectors to the rest of the programme's data** - DOORS, TWC, Jira, test results, cost, keyed by element and requirement id. The inbound joins.
6. **Support and SLA** - the classic paid tier, and the honest one.

## Rulings

1. **THE ENGINEER'S QUESTION IS NEVER GATED.** Anything that answers "is my model good?" is open. Anything that answers "who may see this, and who saw it?" is licensed.
2. **THE BASIS RULE IS NOT A LICENSED FEATURE.** It is the product's honesty, in every tier. A licensed dashboard that hides the basis would be a worse product, not a better one.
3. **THE SCHEMA IS OPEN.** A closed schema would make the open tier a dashboard - the exact failure the analytics-interfaces brief was written to prevent.
4. **THE GRAPH GAPS ANALYTIC LEADS THE OPEN TIER.** Orphans, isolated groups and dangling links are the RFT story. Named, addressable, diffs over baselines.
5. **NOTHING IN THE LICENSED TIER EXISTS TODAY, AND THE PITCH MUST SAY "LATER", NOT "SOON".** SSO, row-level security and the pull audit sit on viewer roles that are not built. The framing stays honest: the boundary is a plan, and the open tier is what ships first.

## Status of the machinery

- Open tier: the readers, the gate, coverage, the graph-gap detection, the CLI, the MCP read tools and the schema transports all exist or are the current build. The trend endpoint and the Python bindings are the remaining pieces.
- Licensed tier: none of it exists. It is the next brief, not this one.
