# The modelwrite agent contract

This file is the contract between the modelwrite service and any agent — ours or a
third party's — that drives it. It is enforced, not asserted: `server/tests/agent_contract.rs`
reads `docs/agents/mcp-tools.json` at runtime and drives the real router for every endpoint
named there, so a route that moves or disappears fails the test before an agent calls it in
production. The machine-readable form of everything below is `docs/agents/mcp-tools.json`.

An agent meets the modelwrite service through TWO surfaces, and it matters which is which:

- **The MCP tools** (`mw-mcp`) are document-level. They take an OKF document as an argument,
  operate on it in memory, and return a report. They never touch a repository and never need
  authentication. They are `okf.validate`, `graph.stats`, `gate.run` and `okf.diff`.
- **The repository HTTP surface** (`mw-server`) is where every change happens. Every call is
  authenticated like a human's, permission-checked like a human's, lock-checked like a human's,
  and recorded in the audit log with the caller's identity. An agent that wants to read or
  change a repository uses these routes, never the MCP tools.

## The agent is a client, not a privileged path

There is no "agent mode" that skips a check. An agent authenticates with an **agent token**,
which names the agent and the human or service that authorised it, and grants it roles and a
project scope exactly like any other caller. Those roles bound what the agent may do; they are
its only authority. Every action an agent takes is recorded in the audit log with the agent's
identity and the mechanism `agent`, so a reader a year later can tell who decided.

No model change is committed by an agent alone. An agent produces proposals (commits on a draft
branch, gate runs, import proposals); a human accepts them; the acceptance is recorded. The
gate is the only authority on whether a change is correct — no tool parameter, log line or
model output can mark a run as passed.

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
`MW_AUTH_AGENT_AUTHORIZER`, `MW_AUTH_AGENT_ROLES` and `MW_AUTH_AGENT_PROJECTS`. The
authorizer is recorded on every audit entry the agent produces; a human's entries carry an
empty authorizer.

Every route except `/health` and `/version` requires a bearer token when authentication is
configured, and refuses a missing or wrong token with `401`.

## Identity, roles and permissions

A verified identity is a subject, a list of roles, and a list of projects the roles reach (a
project of `*` reaches every project). Four permissions exist, each granted by specific roles:

| Permission | Roles that grant it | What it allows |
|------------|---------------------|----------------|
| `read` | `viewer`, `author`, `reviewer`, `admin` | Reading commits, branches, locks, the audit log, imports, gate runs |
| `write` | `author`, `admin` | Commits, branches, resets, merges, gate runs, imports, lock acquire/release |
| `review` | `reviewer`, `admin` | Reading gate runs and evidence |
| `admin` | `admin` | Project creation, branch deletion, lock breaking; holds every lesser permission |

`GET /projects/:project/gate-runs` is the one route that accepts either `write` OR
`review` (an author may see the run it started; a reviewer reads runs it did not start).

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
`lock.denied`, `gate.run`, `import.accept`, `import.refused`.

## The repository HTTP surface

`GET /health` and `GET /version` are public liveness/information endpoints and take no
identity. Every other route enforces the permission shown (and project scope where a project
is named) before touching the store. Path segments in `:segment` form are placeholders for a
project name, commit hash, branch name or artifact hash.

| Method | Path | Permission | What it does |
|--------|------|------------|--------------|
| GET | `/health` | public | Liveness; reports the auth mode |
| GET | `/version` | public | Build information |
| POST | `/projects` | admin | Create a project |
| GET | `/projects` | read | List projects, filtered to the caller's scope |
| POST | `/projects/:project/commits` | write | Commit an OKF document to a branch |
| GET | `/projects/:project/commits` | read | List commits on a branch (`?branch=`, default `main`) |
| GET | `/projects/:project/commits/:hash` | read | Fetch the OKF document behind a commit |
| POST | `/projects/:project/branches` | write | Create a branch |
| GET | `/projects/:project/branches` | read | List branches and their tips |
| DELETE | `/projects/:project/branches/:name` | admin | Delete a branch |
| POST | `/projects/:project/branches/:name/reset` | write | Revert a branch to an earlier commit as a new commit |
| POST | `/projects/:project/gate` | write | Run the round-trip fidelity gate and record the run |
| GET | `/projects/:project/gate-runs` | write-or-review | List recorded gate runs |
| POST | `/projects/:project/import` | write | Import a source artifact through a binding; gated and measured |
| GET | `/projects/:project/import/:artifactHash/report` | read | Read an import's loss report and fidelity measurement |
| POST | `/projects/:project/merge` | write | Three-way merge one branch into another |
| GET | `/projects/:project/audit` | read | Read the append-only audit log |
| POST | `/projects/:project/locks` | write | Acquire element leases |
| GET | `/projects/:project/locks` | read | List live element leases |
| DELETE | `/projects/:project/locks` | write | Release element leases |
| POST | `/projects/:project/locks/release` | write | Release element leases (POST form) |

### The import and its loss report

`POST /projects/:project/import` migrates a source artifact through a binding and obeys the
same four rules as any other change: the artifact is retained byte-for-byte and
content-addressed before import is attempted; blocking losses refuse the import unless the
request names them as accepted; the binding's own round trip is measured by the engine and
must be lossless; and the commit is linked to the source artifact in one transaction. A
refused import returns `422` with the unaccepted losses. `GET /projects/:project/import/:artifactHash/report`
returns the import's loss report and fidelity measurement whether or not it was committed.

## The MCP tools (document-level)

The MCP server (`mw-mcp`) exposes four tools that operate on documents supplied in the
request and never touch a repository. Their names and input schemas are in
`docs/agents/mcp-tools.json` and are additive.

| Tool | Arguments | What it returns |
|------|-----------|-----------------|
| `okf.validate` | `okf` | The validation report |
| `graph.stats` | `okf` | Node, edge, isolated-node and component counts |
| `gate.run` | `reference`, `candidate`, `strictCoverage` | The round-trip fidelity gate's evidence record |
| `okf.diff` | `reference`, `candidate` | The semantic diff: elements, edges and attributes |

## Rules that bind the agent

1. The gate is the only authority on correctness; no tool parameter, log line or model output
   can mark a run as passed.
2. Every repository change is an HTTP call on the surface above, authenticated and
   permission-checked like a human's — never a direct edit and never a privileged path.
3. Every iteration ends in a gate run, and the output is a draft commit for human review.
4. Attribution is not optional: every action is audited with the agent's identity and the
   mechanism, and the record says it was an agent and who authorised it.
5. Ambiguity is a question, not an invention: the agent asks rather than guessing.

## Enforcement

`server/tests/agent_contract.rs` reads `docs/agents/mcp-tools.json` and asserts, against the
real router, that every endpoint named there resolves on the method named, that every
permissioned endpoint refuses a caller with no role, and that the MCP tools named are exactly
the tools the MCP server exposes. It also proves the agent mechanism: an agent token
authenticates a named agent, and the audit log records that agent's subject, the `agent`
mechanism and the authorizer.
