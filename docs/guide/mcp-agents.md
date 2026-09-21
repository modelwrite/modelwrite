# For agents, and for the people who wire them up

> **Do not rename this file to `agents.md`.** On a case-insensitive filesystem
> (Windows, macOS) that collides with the `AGENTS.md` convention and agent
> harnesses pick it up as INSTRUCTIONS TO THE AGENT rather than as a page of
> this guide. It is documentation, not a rule pack.
# Agents

For an AI agent driving modelwrite through MCP, and for the person wiring one up. Everything
here is the enforced contract, not prose. The machine-readable form is
docs/agents/mcp-tools.json, and server/tests/agent_contract.rs reads that file at runtime and
drives the real router and the MCP server's own tools/list handler. A tool or route that
moves or changes fails that test before an agent calls it in production.

## Connect an MCP client

Point your client at the modelwrite MCP server with two environment variables:

- MW_MCP_SERVICE_URL: the running service's base URL, for example http://127.0.0.1:8080 for a
  local service or https://trial.modelwrite.org for the trial. https URLs are fully
  certificate-verified; there is no way to disable that.
- MW_MCP_TOKEN: the bearer token to present. This is an agent token, which the service grants
  the viewer and reviewer roles, never write or admin.

The exact client configuration:

    {
      "mcpServers": {
        "modelwrite": {
          "command": "mw-mcp",
          "env": {
            "MW_MCP_SERVICE_URL": "http://127.0.0.1:8080",
            "MW_MCP_TOKEN": "<the agent token from the service deployment>"
          }
        }
      }
    }

With neither variable set, the MCP server is a pure document processor and makes no network
call. The repository tools are still listed, so the contract stays stable, but they answer
"not configured". Set both and the repository tools reach the running service over the same
HTTP surface a human uses, authenticated with the token.

## The tools

Twenty-six tools. The first four are document-level: they take an OKF document as an argument,
operate on it in memory, and never touch a repository or the network. The other twenty-two are
repository tools, opt-in and read-and-propose only.

<!-- generated:mcp-tools -->
The MCP server exposes **26** tools. The names and schemas are contract-enforced against the live tool table and live in [docs/agents/mcp-tools.json](docs/agents/mcp-tools.json):

| Tool | Required arguments | Summary |
|---|---|---|
| `okf.validate` | okf | Validate an OKF document supplied in the request and return the validation report (document-level; no repository, no network) |
| `graph.stats` | okf | Graph health of an OKF document supplied in the request: nodes, edges, isolated nodes, components (document-level; no repository, no network) |
| `gate.run` | reference, candidate | Run the round-trip fidelity gate between two OKF documents supplied in the request and return its evidence record (document-level; no repository, no network) |
| `okf.diff` | reference, candidate | Semantic diff between two OKF documents supplied in the request: elements, edges and attributes (document-level; no repository, no network) |
| `repo.projects` | — | List the projects the configured token may see. Repository mode (opt-in): requires MW_MCP_SERVICE_URL and MW_MCP_TOKEN; read permission. |
| `repo.branches` | project | List a project's branches and their tips. Repository mode (opt-in): read permission. |
| `repo.commits` | project | List the commits on a project's branch (default main). Repository mode (opt-in): read permission. |
| `repo.read` | project, hash | Read the OKF model behind a commit and its commit record (provenance). Repository mode (opt-in): read permission. |
| `repo.find` | project, hash, query | Search a revision's elements by name, id or stereotype fragment, returning each match's id, name and kind. Repository mode (opt-in): read permission. |
| `repo.element` | project, hash, id | Read one element by id from a revision: its attributes and its graph edges. Repository mode (opt-in): read permission. |
| `repo.coverage` | project, hash | Requirement coverage for a revision: covered and uncovered, each named, from the engine's requirement_coverage. Repository mode (opt-in): read permission. |
| `repo.references` | project, hash | A model's typed subsystem references, with an optional resolve flag for each reference's resolve state (the same resolution the UI and API use). Repository mode (opt-in): read permission. |
| `repo.importReport` | project, artifactHash | Read a migration's loss report and the engine's fidelity measurement, by retained artifact hash. Repository mode (opt-in): read permission. |
| `repo.lossSummary` | project, artifactHash | A migration's loss report aggregated by construct and verdict with counts, plus paging over the full list. Repository mode (opt-in): read permission. |
| `repo.artifact` | project, artifactHash | The retained original bytes of a migrated artifact, byte for byte. Repository mode (opt-in): read permission. |
| `repo.diff` | project, reference, candidate | Diff two commits and run the round-trip gate LOCALLY from the two models read from the repository, returning the verdict and the evidence; never records a run (recording is a write). Repository mode (opt-in): read permission. |
| `repo.audit` | project | Read a project's append-only audit log. Repository mode (opt-in): read permission. |
| `repo.checks` | project, hash | Read the gate checks recorded against a commit (what has been checked about this model). Repository mode (opt-in): review permission. |
| `repo.proposals` | project | The project's proposals with their decisions and who made them. Repository mode (opt-in): read permission. |
| `repo.analytics` | project, requirements | The portfolio question: which models meet which requirements, read from the project analytics route. Repository mode (opt-in): read permission. |
| `repo.propose` | project, reviewArtifact | Record a proposal for a human to review and decide. The agent's output: it persists a proposal (review permission) and never writes a change - a human accepts it. Repository mode (opt-in). |
| `repo.analyticsSchema` | — | The analytics schema (mw-analytics-schema@1): its tables, their columns and the metric definitions, so an agent can plan before it queries. Repository mode (opt-in): read permission. |
| `repo.metrics` | project | The metrics for a project at a commit or branch head, each WITH its basis (basis_element_count, basis_relationship_count, constructs_not_carried, basis_note, engine_version, evidence_hash). Repository mode (opt-in): read permission. |
| `repo.trend` | project, metric, branch | One metric across the commits of a branch, in commit order, each value WITH its basis. Repository mode (opt-in): read permission. |
| `repo.table` | project, table | Bounded rows from one analytics table, with key filters and a cursor. Default 50 rows, maximum 500; every page returns the TOTAL row count. Repository mode (opt-in): read permission. |
| `repo.losses` | project | The paged full import-loss list (repo.lossSummary aggregates; this pages the underlying entries). Repository mode (opt-in): read permission. |
<!-- /generated -->

gate.run also accepts an optional strictCoverage boolean.

repo.commits accepts an optional branch. repo.diff accepts an optional strictCoverage.
repo.audit accepts an optional limit.

The twenty read tools need the read permission, which the viewer role grants. repo.checks and
repo.propose need the review permission, which the reviewer role grants. An agent token may
hold viewer, reviewer, or both; it may not hold author or admin, and a deployment that
configures one is refused at startup.

## The rule an agent author must read

An agent may read and propose; it may not write. An agent token cannot hold a write role
(author) or the admin role. The acceptance route requires the write permission, which an
agent token cannot hold, and the commit path re-checks the acceptance inside its own
transaction. So there is no agent-only path that changes a model, and no tool parameter or
log line that grants one.

The repository tools are a thin client over the same HTTP surface a human uses. Every
repository tool issues a read (GET) route or the single propose route
(POST /projects/:project/proposals, review permission). No tool calls a write route; a write
attempt is refused by the service with 403 and surfaced as a clear tool error, never a crash
and never a retry.

What an agent can therefore accomplish: read a model (repo.read), read a migration's loss
report and its fidelity measurement (repo.importReport), run the gate locally between two
commits and read the verdict and evidence (repo.diff, which never records a run), read the
audit log and the recorded checks (repo.audit, repo.checks), and record a proposal
(repo.propose) that a human then accepts or refuses by id through the ordinary write routes.
The agent's output is a proposal, never a change.

When a human accepts a proposal, the commit's provenance names both parties: the agent that
proposed and the human that accepted, plus the accepted items. A refusal is recorded too. A
record that kept only the successes could not be trusted.

## Configuring the agent on the service

On the service side, the agent mechanism is configured with MW_AUTH_AGENT_TOKEN,
MW_AUTH_AGENT_SUBJECT, MW_AUTH_AGENT_AUTHORIZER, MW_AUTH_AGENT_ROLES and
MW_AUTH_AGENT_PROJECTS. MW_AUTH_AGENT_ROLES may hold only viewer and reviewer. The authorizer
names the human or service that authorised the agent, and every action the agent takes is
recorded in the audit log with that authorizer.

The full contract, including every HTTP endpoint and its permission, is in
docs/agents/mcp-tools.json and docs/agents/mcp-agent.md.
