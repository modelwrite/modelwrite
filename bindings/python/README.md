# modelwrite (Python)

A thin, read-only Python client for the **modelwrite analytics REST surface**
(`mw-analytics-schema@1`): the nine schema tables, the metrics endpoint and the
trend endpoint, exposed as Arrow tables.

It does one thing: it speaks the HTTP API of a running `mw-server` and hands you
the rows. It is deliberately small, because the honest description of what it can
do is *exactly what the API can do, and no more* (see [What it cannot
do](#what-it-cannot-do)).

## Install

The package lives at `bindings/python` in this repository. Install it from
source with the extras you want. **Arrow and pandas are optional extras, never
hard dependencies** — the only required dependency is `requests`.

```console
# Client only: dependency-free tables plus to_pylist(); no Arrow, no pandas.
pip install ./bindings/python

# Typed Arrow tables (pyarrow.Table) from every call.
pip install "./bindings/python[arrow]"

# Arrow + to_pandas() helpers.
pip install "./bindings/python[all]"

# Just the pandas conversion, without pyarrow.
pip install "./bindings/python[pandas]"
```

The extras are additive:

| Extra    | Adds       | Effect                                                       |
|----------|------------|--------------------------------------------------------------|
| *(none)* | `requests` | `analytics.tables()` returns a plain `Table` (list of rows). |
| `arrow`  | `pyarrow`  | `analytics.tables()` returns a real `pyarrow.Table`.        |
| `pandas` | `pandas`   | `table.to_pandas()` returns a `DataFrame`.                   |
| `all`    | both       | Arrow tables with `to_pandas()`.                             |

> The current working-tree version is `0.1.0`. It is not yet published to PyPI;
> install from this repository as above.

## Start a server

The client needs a running `mw-server`. A local server on a scratch SQLite
database:

```console
MW_DB=scratch.db MW_PORT=8080 mw-server
```

On loopback with no `MW_AUTH_TOKEN`/`MW_AUTH_JWKS` configured the server runs in
open mode. To require a bearer token, set `MW_AUTH_TOKEN` and pass the same
token to the client.

## Usage

```python
from modelwrite import analytics

# Everything for one project at its main-branch head: a Tables mapping of all
# nine tables, each the WHOLE table (pagination is followed to the end).
tables = analytics.tables("purchasing-terminal")
list(tables.names)
# ['projects', 'commits', 'elements', 'relationships', 'requirements',
#  'trace_links', 'metrics', 'metric_definitions', 'import_losses']

# One table, as an Arrow table when pyarrow is installed.
elements = analytics.tables("purchasing-terminal", table="elements")
elements.num_rows
elements.column_names          # snake_case, shared with the CSV/Parquet artefacts
elements.to_pylist()           # plain dicts, no Arrow needed

# pandas, when installed.
frame = elements.to_pandas()
every = analytics.tables("purchasing-terminal").to_pandas()   # {name: DataFrame}

# Metrics (the engine's 25 measurements, each carrying its basis).
metrics = analytics.metrics("purchasing-terminal")

# One metric across a branch's commits, in commit order.
trend = analytics.trend("purchasing-terminal", "coverage.covered", "main")

# The schema document (raw dict): schemaVersion, tables, metricDefinitions.
analytics.schema()
```

Select a specific commit, choose a branch, or filter on a string column:

```python
analytics.tables("purchasing-terminal", commit="86a14a81...")
analytics.tables("purchasing-terminal", branch="release")
analytics.tables("purchasing-terminal", table="elements", filters={"kind": "block"})

# Authenticated server.
analytics.tables("purchasing-terminal", token="my-token")
```

A reusable client (connection pooling, custom timeout, or a test double):

```python
from modelwrite import Client, analytics

client = Client(base_url="http://127.0.0.1:8080", token="my-token", timeout=60)
analytics.tables("purchasing-terminal", client=client)
```

### Errors

Failures are typed, mapped from the HTTP status the server returns:

```python
from modelwrite.errors import NotAuthorized, Forbidden, NotFound, BadRequest

try:
    analytics.tables("purchasing-terminal")
except NotAuthorized:      # 401 - bearer token missing or invalid
    ...
except Forbidden:          # 403 - read permission or project scope
    ...
except NotFound:           # 404 - project, commit/branch or table
    ...
```

## What it cannot do

This is a **thin client over the REST surface**. Every limitation of the API is
a limitation of the client, and the client invents no capability of its own:

- **It is read-only.** There is no call to create a project, commit or import a
  model, merge branches, run a gate, re-export, or write anything at all.
- **It exposes exactly the server's endpoints** — `GET /analytics/schema`,
  `GET /analytics/{project}/tables/{table}`, `GET /analytics/{project}/metrics`,
  `GET /analytics/{project}/trend` — one project at a time. There is no
  cross-project query and no server-side aggregation, join or SQL.
- **It computes nothing.** It does not evaluate a metric, coverage or cost; it
  only returns the rows the server already computed.
- **Filters are string equality only.** `filters={"kind": "block"}` becomes the
  query parameter the server applies; the server compares each named column as a
  string, so a numeric, boolean or list column cannot be filtered through it.
- **It holds a whole table in memory.** `tables()` follows the server's
  pagination to completion and accumulates every row. The `import_losses` table
  of a large import can be tens of thousands of rows; if that matters, read it
  page-by-page with `Client.table()` instead.
- **JSON only.** The server can also emit CSV and NDJSON (and the schema version
  as a header); this client does not expose those formats.
- **It does not wrap the OpenMetrics `/metrics` endpoint.**
- **Commit selection is the server's rule**: an explicit `commit` hash wins,
  otherwise the named `branch` tip (`main` by default on the server).
- **The schema is pinned to `mw-analytics-schema@1`.** Additive columns in a
  later minor revision are passed through as extra columns, but the typed
  columns of *known* columns come from the v1 schema.

## Development

```console
pip install "./bindings/python[all]" pytest
python -m pytest bindings/python/tests -q
```

The test suite is in two layers:

1. **Always-on unit tests** that need no server: the camelCase <-> snake_case
   wire mapping, pagination, bearer auth, error mapping and the proof that Arrow
   and pandas are truly optional.
2. **Live tests** that start a real `mw-server` on a scratch `MW_DB`, seed a
   project from `e2e/models/*.json` over the HTTP write path, and assert the
   returned row counts against an oracle recomputed from the source model. They
   are marked `live`, run automatically when an `mw-server` binary is found
   (`MW_SERVER_BIN`, or `target/debug/mw-server`), and skip otherwise. Set
   `MW_TEST_BASE_URL` to point them at an already-running server instead.

## Licence

AGPL-3.0-or-later.
