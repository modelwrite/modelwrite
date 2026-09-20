# MCP surface review: what an agent can see, and what it cannot

## The finding in one line

**The MCP contract was written in Slice 1, before the platform grew composition, coverage, analytics and analyses. An agent can read a MODEL, but it cannot see the SYSTEM OF SYSTEMS, cannot see COVERAGE, and must download an entire document to answer "where is the pump?"**

That matters because the mandate for agents here is specific: *there are not enough engineers who can use these tools, so agents must do the high-volume work.* The high-volume work is triage - read a migration's loss report, find what is uncovered, see whether a platform's references resolve, summarise what changed. **None of those is expressible with the current tool set.**

## What is exposed today (13 tools)

**Document-level (no repository, no network):** `okf.validate`, `graph.stats`, `gate.run`, `okf.diff`.

**Repository (read + one propose):** `repo.projects`, `repo.branches`, `repo.commits`, `repo.read`, `repo.importReport`, `repo.diff`, `repo.audit`, `repo.checks`, `repo.propose`.

That is a good READ surface for a document store. It is a weak surface for an engineering platform.

## The gaps, each against a real job

| Job an agent should do | Route that exists | MCP tool |
|---|---|---|
| "Does this platform's composition resolve?" | `/commits/:hash/references/resolve` | **MISSING** |
| "Which requirements are uncovered?" | `graph::requirement_coverage` via the UI | **MISSING** |
| "Where is the pump?" | `GET /projects/:p/commits/:hash` (whole model) | **MISSING** - and today it means pulling 362 blocks to find one |
| "What are the biggest loss classes?" | The import report returns **45,725 entries** | **PARTIAL** - no aggregation, so an agent must read 45k rows |
| "What proposals are open?" | `GET /projects/:p/proposals` | **MISSING** |
| "Which products meet which specs?" | `/projects/:p/analytics` | **MISSING** |
| "Show me the original I migrated from" | `/import/:artifactHash/artifact` | **MISSING** |
| "What has been analysed?" | analyses slice (in flight) | **MISSING** |

## What to add

**`repo.references`** - a model's typed subsystem references and, with a flag, their resolve state. The system-of-systems view, which is the platform's distinguishing feature and is currently invisible to agents.

**`repo.coverage`** - requirement coverage for a revision: covered and uncovered, each named, computed by the ENGINE. This is the single most useful question in MBSE and there is no tool for it.

**`repo.element`** - one element by id, with its attributes and its edges. The targeted read that stops an agent pulling a 36 MB model to answer a one-line question.

**`repo.find`** - search elements by name, id or stereotype across a revision, returning matches with their ids and kinds. The tool an agent needs before it proposes anything, so it can avoid collisions and reuse what exists.

**`repo.lossSummary`** - a migration's loss report AGGREGATED by construct and verdict, with counts, plus the option to page the full list. The raw report is 45,725 entries: an agent that cannot aggregate cannot triage. This turns the platform's honesty into something usable at scale.

**`repo.proposals`** - the project's proposals with their decisions and who made them, so an agent can see what happened to what it proposed.

**`repo.analytics`** - the portfolio question (which models meet which requirements/specifications).

**`repo.artifact`** - the retained original bytes, so an agent can verify a migration rather than trust a summary.

## The rulings that govern the additions

1. **AN AGENT STILL CANNOT WRITE.** Every new tool is read-only, except that `repo.propose` remains the single write-shaped action and it produces a PROPOSAL. The wire-level test asserting only GET plus the single POST to /proposals must be extended to cover the new tools, not weakened.
2. **NO TOOL COMPUTES ANYTHING THE ENGINE DOES NOT ALREADY COMPUTE.** `repo.coverage` calls the engine's coverage function, `repo.references` calls the same resolve the UI calls. A second implementation would be a second source of truth - the defect this project has caught most often.
3. **AGGREGATION IS A VIEW, NOT A NEW TRUTH.** `repo.lossSummary` groups what the binding reported; it never drops an entry from the underlying list.
4. **EVERY TOOL THE CONTRACT NAMES IS ENFORCED BY A TEST** against the live router and the live tool table, exactly as the existing contract test does - so the contract cannot drift from the service.
5. **AN AGENT'S JOB IS TRIAGE, NOT APPROVAL.** The additions are chosen to let an agent read, find, aggregate and summarise - and to stop there.

## What this changes about the platform's story

The claim "an agent can drive modelwrite" is currently true in the narrow sense that it can read documents and file proposals. With these additions it becomes true in the useful sense: **an agent can triage a migration of 45,725 losses, find the uncovered requirements, check that a platform's four subsystems resolve, and file a proposal a human accepts** - which is the work there are not enough engineers to do.
