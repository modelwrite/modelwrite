# modelwrite-mcp

The modelwrite MCP server, distributed so an MCP client can start it as a
plain `mw-mcp` command. It exposes thirteen tools, split in two:

- **document tools** — `okf.validate`, `graph.stats`, `gate.run`, `okf.diff` — take an
  OKF document as an argument, run entirely in memory, and never touch the network.
- **repository tools** — `repo.*` — read a running modelwrite repository and record a
  proposal for a human to accept. Opt-in only (see the environment variables below).

## Install

```sh
npm install modelwrite-mcp
```

At install time the package downloads the `mw-mcp` binary built for your platform
(Windows, macOS or Linux). If that download fails the install fails loudly; it
never leaves a package that half-works.

## Configure a client

Point any MCP client (Claude Desktop, Cursor, …) at the `mw-mcp` command:

```json
{
  "mcpServers": {
    "modelwrite": {
      "command": "mw-mcp"
    }
  }
}
```

Configured this way it is a **pure document processor**: it validates, gates and
diffs OKF documents entirely in memory and makes **no network call**.

To also read a repository, set the two environment variables that enable
repository mode:

```json
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
```

- `MW_MCP_SERVICE_URL` — the running service's base URL, plain `http://`.
- `MW_MCP_TOKEN` — the bearer token to present (an agent token; the service grants it viewer/reviewer).

Set **both or neither**. With neither set, the server is a pure document
processor that makes no network call; with only one set it refuses to start
rather than guess. The full agent contract (tools, permissions, and what an
agent may do) is `docs/agents/mcp-agent.md` in the repository.
