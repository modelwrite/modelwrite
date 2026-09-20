# Modelwrite guide

Modelwrite is an open-source platform for systems engineering that turns legacy SysML
models into a portable, provable JSON format called OKF. Every model is a versioned,
content-addressed commit, and every claim about a model traces to a recorded gate run a
reviewer can reproduce. It ships a workbench for authoring and reviewing models, an MCP
server an AI agent drives to read and propose changes it can never commit on its own, and
the mw command line that reaches the service over HTTP or a SQLite store directly for
offline, scripted use.

## Who this guide is for

Three readers, all first class.

- A systems engineer who has never seen modelwrite, and wants to know what it is, what it
  will and will not do, and how to drive it.
- An AI agent, and the person wiring one up, that reads models and records proposals
  through MCP.
- A pipeline operator, CI job, or air-gapped site that drives modelwrite from a shell with
  no server running and no port open.

Start with [concepts](concepts.md) for the mental model, then [tasks](tasks.md) for the
click paths. Agents start at [mcp-agents](mcp-agents.md).

## What modelwrite does not do

Modelwrite does not run numeric or physical simulation. It does not yet migrate a real,
content-rich vendor model without named loss. It does not yet read SysML v2 textual
notation. The full list, each with its consequence, is on [limits](limits.md). Read that
page before you trust anything else in this guide.

## The four URLs

| What | URL |
|---|---|
| Trial | https://trial.modelwrite.org |
| Website | https://modelwrite.org |
| Repository | https://github.com/modelwrite/modelwrite |
| Releases | https://github.com/modelwrite/modelwrite/releases |

## Five-minute tour

Open the trial and follow along.

1. Open https://trial.modelwrite.org. The bare host redirects to the project list: six
   cards, one per model (coffee-machine, sandwich-toaster, purchasing-terminal,
   floor-robot, microduck, cafe-stand).
2. Click the coffee-machine card. You land on its Changes page. In the left rail, under
   Sections, click Overview. You see the coverage count, the element and requirement
   counts, the versions, and below them the structure tree, the requirements table, the
   traceability matrix, and the state and activity view.
3. Switch model. In the top bar, the model switcher (the chip showing the project name)
   opens a menu; choose cafe-stand. Or click cafe-stand under Models in the left rail.
4. In the left rail, under Sections, click Composition. You see the three subsystems
   cafe-stand references at pinned revisions (coffee-machine, sandwich-toaster,
   purchasing-terminal), the measured-versus-asserted boundary, and the business process
   that runs across them.

That is the product in four steps: a model, its proof, and a platform that composes other
models by reference rather than by copying them.

## The rest of the guide

- [concepts.md](concepts.md): the mental model, short.
- [tasks.md](tasks.md): do this, then that.
- [mcp-agents.md](mcp-agents.md): the MCP contract for an agent and the person wiring one up.
- [cli.md](cli.md): the mw command line, HTTP and offline.
- [faq.md](faq.md): the honest answers.
- [limits.md](limits.md): what it does not do, in one place.
- [troubleshooting.md](troubleshooting.md): what a failure means, and what to do.
