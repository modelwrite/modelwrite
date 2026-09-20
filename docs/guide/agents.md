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

Thirteen tools. The first four are document-level: they take an OKF document as an argument,
operate on it in memory, and never touch a repository or the network. The other nine are
repository tools, opt-in and read-and-propose only.

Document tools (always available, no repository, no network):

| Tool | Required arguments | Returns |
|---|---|---|
| okf.validate | okf | The validation report |
| graph.stats | okf | Node, edge, isolated-node and component counts |
| gate.run | reference, candidate | The round-trip gate's evidence record |
| okf.diff | reference, candidate | The semantic diff: elements, edges, attributes |

gate.run also accepts an optional strictCoverage boolean.

Repository tools (opt-in, read-and-propose only):

| Tool | Required arguments | Permission | Returns |
|---|---|---|---|
| repo.projects | | read | The projects the token may see |
| repo.branches | project | read | The project's branches and their tips |
| repo.commits | project | read | The commits on a branch (default main) |
| repo.read | project, hash | read | The OKF model behind a commit, with its commit record |
| repo.importReport | project, artifactHash | read | A migration's loss report and fidelity measurement |
| repo.diff | project, reference, candidate | read | The gate verdict for two commits, run locally, never recorded |
| repo.audit | project | read | The append-only audit log |
| repo.checks | project, hash | review | The gate checks recorded against a commit |
| repo.propose | project, reviewArtifact | review | Record a proposal for a human to decide |

repo.commits accepts an optional branch. repo.diff accepts an optional strictCoverage.
repo.audit accepts an optional limit.

The seven read tools need the read permission, which the viewer role grants. repo.checks and
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
