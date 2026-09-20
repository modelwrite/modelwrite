# The `mw` command line

`mw` is the modelwrite command line. It does the same work the HTTP API does, from a
shell: create projects, commit models, inspect history, run the gate, merge and reset
branches, hold and release element locks, and read the audit trail. It prints JSON on
stdout, an error on stderr, and exits non-zero on failure, so it fits a scripted pipeline
or a CI job the way a GUI cannot.

There are two ways to reach a repository, and you choose exactly one per invocation:

- `--server <url>` — talk HTTP/1.1 to a running `mw-server`.
- `--db <path>` — open a local SQLite store directly. No server, no network.

`mw` is deliberately small: no CLI framework, no HTTP client dependency, and the bearer
token for `--server` is always read from an environment variable you name — never passed
on the command line, where it would land in the shell history and the process list.

## Install

From crates.io (published):

```console
cargo install mw-cli
```

Or from this repository:

```console
cargo install --path cli
```

Or download a prebuilt binary from the GitHub releases page; the `mw` assets are named
`mw-linux-x64`, `mw-linux-arm64`, `mw-darwin-x64`, `mw-darwin-arm64`, and
`mw-windows-x64.exe`.

The engine layer is published alongside the CLI (`mw-okf`, `mw-graph`, `mw-gate`,
`mw-binding`, `mw-capi`, `mw-binding-xmi`, `mw-agent`, `mw-analytics`,
`mw-mcp`, `mw-test-support`, `mw-server`), all at 0.2.0. The engine's 1.75 MSRV
floor is pinned in the engine crates' own manifests (indexmap =2.11.4, zeroize =1.8.2).

## HTTP mode

Point `mw` at a running service. The base URL goes in `--server`; if the service
requires a bearer token, name the environment variable that holds it with `--token`
(never the token itself):

```console
mw --server http://localhost:8080 --token MW_AUTH_TOKEN project list
```

The client speaks plain HTTP on purpose — TLS belongs at the reverse proxy. Give it an
`https://` URL and it refuses, telling you to point at the proxy's `http://` address.

Every command below works in HTTP mode except `artifact`, which is offline only.

## Offline mode

```console
mw --db modelwrite.db commit coffee --branch main --message "first commit" --file model.json
```

`--db` opens the SQLite database directly. No `mw-server` process is running, no port
is open, and nothing leaves the machine. This is the mode for an air-gapped site, a CI
job, or any machine that must not expose a service.

Offline is not a lesser mode. The same store is doing the work, so a commit, merge, reset
or gate run offline is enforced exactly as it would be behind the server: every write
validates the OKF document, derives the true summary at the commit convergence point, and
honours element locks; the gate records the same verdict and evidence the server returns
(the offline-vs-server parity test asserts this byte for byte). Each offline write is
recorded in the audit trail with actor `anonymous` and mechanism `offline`.

## Commands

One example each. `--db` and `--server` are interchangeable everywhere except
`artifact`.

### project

```console
mw --db modelwrite.db project create coffee
mw --db modelwrite.db project list
```

### commit

Commit an OKF document to a branch:

```console
mw --db modelwrite.db commit coffee --branch main --message "first commit" --file model.json
```

`--file` must be a valid OKF document; an invalid one is refused with the validator's
errors and the branch tip does not move. `--holder <name>` records whose element lease
authorises the write.

### log

```console
mw --db modelwrite.db log coffee --branch main
```

### gate

Run the fidelity gate between two committed revisions and record the run:

```console
mw --db modelwrite.db gate coffee --reference <ref-hash> --candidate <cand-hash>
```

### merge

Three-way merge another branch into the current one:

```console
mw --db modelwrite.db merge coffee --branch main --other feature --message "merge feature"
```

A conflicting merge is refused and writes nothing.

### branch

```console
mw --db modelwrite.db branch list coffee
mw --db modelwrite.db branch create coffee --name feature --from <commit-hash>
mw --db modelwrite.db branch delete coffee --name feature
```

### reset

Move a branch to an earlier commit, recorded as a new commit:

```console
mw --db modelwrite.db reset coffee --branch main --to <commit-hash> --message "roll back"
```

### artifact

Fetch the retained source artifact for an import — offline only:

```console
mw --db modelwrite.db artifact coffee --hash <import-hash>
```

### lock

```console
mw --db modelwrite.db lock acquire coffee --branch main --elements block1,block2 --holder alice --ttl 300
mw --db modelwrite.db lock release coffee --holder alice --ids <lock-id>
mw --db modelwrite.db lock list coffee
```

### audit

```console
mw --db modelwrite.db audit coffee --limit 50
```

## What offline mode does not do

Offline mode is the store, directly, and nothing else. Read these before you rely on it
anywhere they matter:

1. **No identity, no authorisation.** Offline writes are recorded as actor `anonymous`.
   There is no login, no roles, no signing, and no way to tell two people apart. Anyone
   who can open the SQLite file can write to it, and the audit trail records *what*
   happened, not *who* did it. If you need attribution or access control, run the server
   and configure `MW_AUTH_TOKEN` or `MW_AUTH_JWKS`.

2. **Single writer, assumed.** `--db` opens the file directly and does not coordinate
   with a running service or another process. Do not point it at a database a live
   `mw-server` is also writing to.

3. **SQLite only.** There is no PostgreSQL backend offline; the server is where the
   PostgreSQL store lives.

4. **`artifact` is text only.** The offline reader returns a retained artifact only when
   it is UTF-8 text (a source XMI export, for example). A non-text artifact must be
   fetched over the server route.

5. **Nothing beyond the store.** Offline mode has no HTTP API, no MCP surface, no
   workbench UI, and no analytics or assist endpoints. If the surface you are integrating
   against is one of those, you need the server.

The gate, the validator, the lock enforcement and the audit trail are the same offline as
online. What offline lacks is not correctness — it is identity, concurrency coordination,
and every network surface.
