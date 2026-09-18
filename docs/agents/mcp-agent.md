# The modelwrite agent contract

This file is the contract between the modelwrite service and any agent — ours or a
third party's — that drives it. It is enforced, not asserted: `server/tests/agent_contract.rs`
reads `docs/agents/mcp-tools.json` at runtime, drives the real router for every HTTP endpoint
named there, and drives the MCP server's own `tools/list` handler for the tool names and their
required arguments. A route or tool that moves, disappears or changes fails the test before an
agent calls it in production. The machine-readable form of everything below is
`docs/agents/mcp-tools.json`.

An agent meets the modelwrite service through TWO surfaces, and it matters which is which:

- **The MCP tools** (`mw-mcp`) have TWO modes. The document-level tools take an OKF document as
  an argument, operate on it in memory, and return a report; they never touch a repository and
  never need authentication. The repository tools (`repo.*`) are OPT-IN: they reach the running
  service over its HTTP API, authenticated with a token, and are read-and-propose only. With no
  configuration the MCP server is a pure document processor that makes NO network call.
- **The repository HTTP surface** (`mw-server`) is where every change happens. Every call is
  authenticated like a human's, permission-checked like a human's, lock-checked like a human's,
  and recorded in the audit log with the caller's identity. The `repo.*` MCP tools are a thin,
  read-and-propose client over this same surface — never a second, privileged one.

## The agent is a client, not a privileged path

There is no "agent mode" that skips a check. An agent authenticates with an **agent token**,
which names the agent and the human or service that authorised it, and grants it roles and a
project scope exactly like any other caller. Those roles bound what the agent may do; they are
its only authority.

An agent may **read** and **propose**. It may **not write**: an agent token cannot hold a write
role (`author`) or the `admin` role, and a deployment that configures one is refused at startup
with a message saying that an agent proposes and a human commits.

An agent's proposals ARE persisted and addressable, and there IS a review-and-accept route. The
MCP tool `repo.propose` records the proposal (it needs the `review` role, which an agent token
may hold). Acceptance requires the `Write` permission, which an agent token cannot hold, and the
commit path re-checks the acceptance inside its own transaction. So the acceptance route is for
the human, and the agent's proposals are its input.

The record names both parties. A commit that came from an accepted proposal carries provenance
naming the proposal, the **agent** that proposed it, the **human** who accepted it, and the items
accepted — written in the same transaction as the commit, so nobody can produce such a commit
without the record that justifies it. A refusal is recorded too, because a record that keeps only
the successes is a record that cannot be trusted.

The acceptance route is currently import-scoped: a proposal whose subject is a migration's loss
report, with its retained artifact and binding. Other material is refused with a 400 rather than
accepted into a flow that does not exist for it.

The gate remains the only authority on whether a change is correct — no tool parameter, log line
or model output can mark a run as passed.

## Authentication

Authentication is opt-in. The mode is chosen at startup and reported by `GET /health` (the
mode only — never a token, key or path):

| Mode | How a caller authenticates | Identity produced |
|------|----------------------------|-------------------|
| `open` | No credential; the default | `anonymous` admin over every project |
| `static` | `Authorization: Bearer <MW_AUTH_TOKEN>` | the fixed admin subject |
| `jwt` | A signed JWT verified against the configured JWKS | the verified `sub` plus `roles`/`projects` claims |
| `agent` | `Authorization: Bearer <MW_AUTH_AGENT_TOKEN>` | the agent's subject plus the configured roles and projects |

The agent mechanism is configured with `MW_AUTH_AGENT_TOKEN`, `MW_AUTH_AGENT_SUBJECT`,
`MW_AUTH_AGENT_AUTHORIZER`, `MW_AUTH_AGENT_ROLES` and `MW_AUTH_AGENT_PROJECTS`.
`MW_AUTH_AGENT_ROLES` may hold only `viewer` and/or `reviewer`: a write role (`author`) or the
`admin` role is refused at startup, because an agent proposes and a human commits. The
authorizer names the human or service that authorised the agent.

Every route except `/health` and `/version` requires a bearer token when authentication is
configured, and refuses a missing or wrong token with `401`.

## Identity, roles and permissions

A verified identity is a subject, a list of roles, and a list of projects the roles reach (a
project of `*` reaches every project). Four permissions exist, each granted by specific roles:

| Permission | Roles that grant it | What it allows |
|------------|---------------------|----------------|
| `read` | `viewer`, `author`, `reviewer`, `admin` | Reading commits, branches, locks, the audit log, imports, gate runs |
| `write` | `author`, `admin` | Commits, branches, resets, merges, gate runs, imports, lock acquire/release |
| `review` | `reviewer`, `admin` | Reading gate runs and evidence; recording a proposal |
| `admin` | `admin` | Project creation, branch deletion, lock breaking; holds every lesser permission |

`GET /projects/:project/gate-runs` and `GET /projects/:project/commits/:hash/checks` accept
either `write` OR `review` (an author may see the run it started; a reviewer reads runs it did
not start).

Every route that names a project also enforces **project scope**: a caller whose roles do not
reach the project is refused `403` before the store is touched. A listing is filtered, not
refused — an identity scoped to one project sees only that project, and one scoped to nothing
sees an empty list.

## Refusals

An agent meets exactly the refusals a human meets:

| Status | Meaning |
|--------|---------|
| `401` | Missing or invalid bearer token (when authentication is configured) |
| `403` | The authenticated caller lacks the required permission, or the project is out of scope |
| `404` | The named project, commit, branch or import does not exist |
| `409` | A guarded write would change an element another holder has a live lease on, or a merge/reset conflicts |
| `422` | A well-formed OKF document that fails validation, or an import with blocking losses that were not accepted |

An `author` field that names a different actor than the authenticated subject is refused
`403` before anything is written — a caller cannot put another person's name into the record.

## Element locks (leases)

Every write computes the elements it would change and refuses to change any element another
holder has a live lease on, unless the caller supplies the matching `holder` and holds the
lease itself. Leases always carry an expiry (`ttlSeconds`, 30–86400), so a crashed client
cannot block an element forever. A lock refusal is recorded in the audit log as an attempt to
overwrite someone's work.

## The append-only audit log

`GET /projects/:project/audit` returns the audit log, newest first. It records what was
attempted as well as what succeeded, and it can never be edited: the store exposes only an
append and this read. Every entry carries:

- `actor` — the verified subject (never a name taken from the request body; in open mode,
  `anonymous`)
- `mechanism` — how the caller authenticated: `open`, `static`, `jwt` or `agent`
- `authorizer` — for agents, the human or service the agent acted for; empty for humans
- `action` — one of the fixed vocabulary below
- `subject` and `detail` — what the action touched

The action vocabulary is fixed and lives in `server/src/audit.rs`:
`project.create`, `commit.create`, `commit.refused`, `branch.create`, `branch.delete`,
`branch.reset`, `merge.clean`, `merge.conflict`, `lock.acquire`, `lock.release`,
`lock.denied`, `gate.run`, `import.accept`, `import.refused`, `proposal.record`,
`proposal.accept`, `proposal.refused`.

## The repository HTTP surface

`GET /health` and `GET /version` are public liveness/information endpoints and take no
identity. Every other route enforces the permission shown (and project scope where a project
is named) before touching the store. Path segments in `:segment` form are placeholders for a
project name, commit hash, branch name or artifact hash.

| Method | Path | Permission | What it does |
|--------|------|------------|--------------|
| GET | /health | public | Liveness; reports the auth mode |
| GET | /version | public | Build information |
| POST | /projects | admin | Create a project |
| GET | /projects | read | List projects, filtered to the caller's scope |
| POST | /projects/:project/commits | write | Commit an OKF document to a branch |
| GET | /projects/:project/commits | read | List commits on a branch (?branch=, default main) |
| GET | /projects/:project/commits/:hash | read | Fetch the OKF document behind a commit |
| GET | /projects/:project/commits/:hash/record | read | Fetch a commit's record and provenance |
| GET | /projects/:project/commits/:hash/checks | write-or-review | Read the gate checks recorded against a commit |
| POST | /projects/:project/branches | write | Create a branch |
| GET | /projects/:project/branches | read | List branches and their tips |
| DELETE | /projects/:project/branches/:name | admin | Delete a branch |
| POST | /projects/:project/branches/:name/reset | write | Revert a branch to an earlier commit as a new commit |
| POST | /projects/:project/gate | write | Run the round-trip fidelity gate and record the run |
| GET | /projects/:project/gate-runs | write-or-review | List recorded gate runs |
| POST | /projects/:project/import | write | Import a source artifact through a binding; gated and measured |
| GET | /projects/:project/import/:artifactHash/report | read | Read an import's loss report and fidelity measurement |
| GET | /projects/:project/import/:artifactHash/artifact | read | Fetch the retained source artifact byte for byte |
| POST | /projects/:project/proposals | review | Record an agent's proposal (the body IS the review artifact) |
| GET | /projects/:project/proposals/:id | read | Fetch a proposal by id |
| POST | /projects/:project/proposals/:id/accept | write | A human with write accepts a proposal by id |
| POST | /projects/:project/proposals/:id/refuse | write | A human with write refuses a proposal by id |
| POST | /projects/:project/merge | write | Three-way merge one branch into another |
| GET | /projects/:project/audit | read | Read the append-only audit log |
| POST | /projects/:project/locks | write | Acquire element leases |
| GET | /projects/:project/locks | read | List live element leases |
| DELETE | /projects/:project/locks | write | Release element leases |
| POST | /projects/:project/locks/release | write | Release element leases (POST form) |

### The import and its loss report

`POST /projects/:project/import` migrates a source artifact through a binding and obeys the
same four rules as any other change: the artifact is retained byte-for-byte and
content-addressed before import is attempted; blocking losses refuse the import unless the
request names them as accepted; the binding's own round trip is measured by the engine and
must be lossless; and the commit is linked to the source artifact in one transaction. A
refused import returns `422` with the unaccepted losses. `GET /projects/:project/import/:artifactHash/report`
returns the import's loss report and fidelity measurement whether or not it was committed.

## The MCP tools

The MCP server (`mw-mcp`) exposes thirteen tools. The first four are document-level: they
operate on documents supplied in the request and never touch a repository or the network. The
remaining nine are the repository tools, opt-in and read-and-propose only. Their names and
input schemas are in `docs/agents/mcp-tools.json` and are additive. `gate.run` and `repo.diff`
also accept an optional `strictCoverage` boolean, named in the manifest rather than below.

### Document tools (always available, no repository, no network)

| Tool | Required arguments | What it returns |
|------|--------------------|-----------------|
| `okf.validate` | `okf` | The validation report |
| `graph.stats` | `okf` | Node, edge, isolated-node and component counts |
| `gate.run` | `reference`, `candidate` | The round-trip fidelity gate's evidence record |
| `okf.diff` | `reference`, `candidate` | The semantic diff: elements, edges and attributes |

### Repository tools (opt-in, read-and-propose only)

Repository mode is enabled only when BOTH `MW_MCP_SERVICE_URL` (the running service's base URL,
plain `http`) and `MW_MCP_TOKEN` (the bearer token to present) are set. With neither set, the
repository tools are listed but answer "not configured", and the server makes no network call.
Every repository tool issues a read (`GET`) route or the single propose route; no tool issues a
write route, and a `403` from the service is surfaced as a clear tool error, never a crash and
never a retry.

| Tool | Required arguments | What it returns |
|------|--------------------|-----------------|
| `repo.projects` |  | The projects the token may see |
| `repo.branches` | `project` | The project's branches and their tips |
| `repo.commits` | `project` | The commits on a branch (`branch`, default `main`) |
| `repo.read` | `project`, `hash` | The OKF model behind a commit, with its commit record (provenance) |
| `repo.importReport` | `project`, `artifactHash` | A migration's loss report and fidelity measurement |
| `repo.diff` | `project`, `reference`, `candidate` | The local gate verdict and evidence for two commits (never records a run) |
| `repo.audit` | `project` | The append-only audit log |
| `repo.checks` | `project`, `hash` | The gate checks recorded against a commit |
| `repo.propose` | `project`, `reviewArtifact` | The recorded proposal (the agent's output for a human to decide) |

`repo.diff` reads the two models from the repository and runs the gate LOCALLY with the same
engine the service uses. It returns the verdict and evidence but never records a run, because
recording a run is a write (`POST /projects/:project/gate`). `repo.propose` is the agent's real
job: it persists a proposal (the `review` permission) for a human to accept or refuse by id.

## How to connect your agent

Point your agent at modelwrite by configuring the MCP server with two environment variables:

- **MW_MCP_SERVICE_URL** — the running service's base URL, e.g. http://127.0.0.1:8080 (plain HTTP)
- **MW_MCP_TOKEN** — the bearer token to present — an agent token (the service grants it viewer/reviewer)

With neither set, `mw-mcp` is a pure document processor and makes no network call. A typical
MCP client configuration:

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

## Rules that bind the agent

1. The gate is the only authority on correctness; no tool parameter, log line or model output
   can mark a run as passed.
2. Every repository change is an HTTP call on the surface above, authenticated and
   permission-checked like a human's — never a direct edit and never a privileged path. The
   `repo.*` tools are read-and-propose only; a write is refused by the service with 403 and the
   MCP server surfaces it as a clear refusal, never a crash or a retry.
3. The agent's output is a proposal for a human to read. `repo.propose` records that proposal;
   a human accepts or refuses it by id through the ordinary write routes.
4. Attribution is not optional: the agent token carries the agent's identity and the
   authorizer, so any action it takes is attributable to the agent and who authorised it.
5. Ambiguity is a question, not an invention: the agent asks rather than guessing.

## Enforcement

`server/tests/agent_contract.rs` reads `docs/agents/mcp-tools.json` and asserts, against the
real router, that every HTTP endpoint named there resolves on the method named, that every
permissioned endpoint refuses a caller with no role, and — by driving the MCP server's own
`tools/list` handler — that the manifest names exactly the tools the MCP server exposes with
the same required arguments, and that the toolset is read-and-propose only (no write tool).
The MCP tool list is ENFORCED, and the HTTP surface is ENFORCED. The repository tools' own
tests (`engine/mcp/tests/repository.rs`) prove, over an injected transport and a loopback
server, that the tools read a real project, model and loss report, that the wire carries only
read routes plus the propose route, that a 403 is surfaced once and not retried, and that with
no configuration no network call is made. `server/tests/agent_audit.rs` proves an agent cannot
commit, merge, reset or import, and `server/tests/proposal.rs` proves an agent is refused at
the acceptance endpoint and that an accepted proposal's commit names both parties.
