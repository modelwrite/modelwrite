# modelwrite browser e2e tests

Headless-browser tests for the workbench's three-pane enhancement
(`server/src/ui/app.js`). They prove the enhancement's RUNTIME behaviour - the
containment tree builds, clicking a tree node updates the properties panel, the
search box filters, keyboard navigation works, the diagram carries the selection
hook, and `?select=<id>` deep links highlight the right entry. The Rust tests
assert the page's `data-*` attributes structurally; a syntactically valid JS
file could still do nothing, and this directory exists so that cannot slip.

## Dependency boundary

The product itself has NO build step and NO runtime dependencies. Everything in
this directory is a **devDependency used only in CI** (and optional local runs):

- never served by mw-server;
- never fetched at page load (`app.js` is embedded in the binary and served by
  the same router);
- never in the Docker image (`deploy/Dockerfile` copies only `Cargo.toml`,
  `engine`, `server` and `cli`).

## How it works

`global-setup` starts `mw-server` against a throwaway SQLite database, seeds it
with the coffee-machine corpus over the SAME JSON API the Rust test
`server/tests/corpus.rs` uses (the fixture is
`sample/corpus/coffee-machine/okf/expected/coffee_machine_model.json`), and
leaves it running. `global-teardown` stops it.

## Running locally

```
cargo build -p mw-server
cd e2e
npm ci
npx playwright install chromium
npx playwright test
```

The server binds `127.0.0.1:8099` by default; override with `MW_E2E_PORT` (and
`MW_E2E_BASE_URL` to point at an already-running, already-seeded server).
