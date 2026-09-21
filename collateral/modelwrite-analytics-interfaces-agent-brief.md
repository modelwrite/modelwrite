# Modelwrite analytics interfaces: agent brief

Owner: Aco (Alex Kovaceski)
Public repo: `C:\Users\alexk\projects\modelwrite`
Environment: Windows, PowerShell 7
Date written: 21 September 2026
Companion to: "Modelwrite commercial readiness: agent brief (v2, analytics framing)". The guardrails in section 2 of that brief apply here in full. Read them first.

This brief may live in the public repo under `docs/superpowers/plans/` once Aco approves it. It contains no commercial material.

---

## 1. Purpose

Modelwrite is positioned as the open source analytics platform for SysML models. An analytics platform that other analytics tools cannot read from is a dashboard, not a platform. The goal of this work is that a BI tool, a notebook, a data lake job, a monitoring stack or an AI agent can pull the model and its measures out of Modelwrite without anyone writing glue code.

Modelwrite already has the three surfaces this needs. Do not rebuild them:

- a REST API: the OpenAPI-documented JSON API in `mw-server`;
- a CLI: `mw`, which runs against a server (`--server`) or a local store (`--db`);
- an MCP server: `mw-mcp` in `bindings/mcp`, a stdio server with tools aimed at modelling agents;
- plus the Python bindings in `bindings/python` and `bindings/python-agent`.

What is missing is an analytics-facing layer on top of them: the model and its measures as plain tables, in one stable schema, delivered the same way through every surface.

---

## 2. Design decisions (settled, not yours to reopen)

1. **One schema, many transports.** There is a single versioned analytics schema, `mw-analytics-schema@1`. REST, CLI, MCP and Python are transports for the same tables. None of them defines its own shape.
2. **The basis travels with the number.** Every metric row carries what it was computed over and what the reader did not carry, as data, not only as prose on a page. This depends on Workstream H of the companion brief. A metric without a basis is a defect.
3. **Read-only.** Nothing in this work adds a write path. It respects the existing roles and project scoping. The viewer role is sufficient.
4. **Commits are immutable, so metrics cache exactly.** Cache every metric by commit id, metric id and engine version. The cache never needs invalidating. This is what makes a trend across baselines cheap.
5. **Deterministic output.** The same project, commit and table give byte-identical CSV and NDJSON, and row-identical Parquet. No timestamps are added by the export itself.
6. **Lake-friendly files.** File exports use a Hive-style layout, `<out>/<table>/project=<p>/commit=<c>/part-0.parquet`, so DuckDB, Spark, Trino and Iceberg ingestion need no mapping.
7. **Nothing is invented.** A column exists only if OKF or an engine output carries the value. If the model does not carry it, the column is not in the schema.
8. **Prove the transports agree.** A cross-transport equivalence test, recorded as an evidence record, shows REST, CLI and MCP return the same rows for the same request.

---

## 3. Work packages

Order: 1, 2, then 3, 4 and 5 in parallel, then 6, 7, 8. One local branch per package. Do not push.

### Package 1: inventory and schema proposal (report only, Aco approves before any code)

1. Read the current surfaces and list, for each, what it already returns that an analytics consumer could use: the REST endpoints (from the OpenAPI document and `server/src/api/`), the `mw` subcommands, the `mw-mcp` tools, and the Python modules.
2. Map OKF sections and engine outputs to the target tables below. Adjust names to match OKF. Remove any column the model does not carry. Add any the model carries that an analyst would want.
3. Write the proposal as `docs/drafts/analytics-schema-proposal.md` with one section per table: columns, types, keys, and the OKF or engine source of every column.

Target tables for `mw-analytics-schema@1`:

| Table | One row per | Notes |
|---|---|---|
| `projects` | project | id, name, default branch |
| `commits` | commit | id, parents, branch, author, committed time, message, provenance kind, import id if any |
| `elements` | element per commit | id, kind, name, qualified name, owner id, stereotypes, provenance kind, source binding, source artifact hash |
| `relationships` | relationship per commit | id, kind, source id, target id |
| `requirements` | requirement per commit | element id, identifier, text, covered flag, covering link kinds |
| `trace_links` | trace link per commit | requirement id, element id, link kind |
| `metrics` | metric per commit | metric id, value, unit, basis element count, basis relationship count, constructs not carried, basis note, engine version, evidence hash if any |
| `metric_definitions` | metric | id, name, plain description, constructs it depends on, status |
| `findings` | finding per commit | check id, severity, element id if any, message |
| `import_losses` | construct per import | construct, severity, count, example ids |

Done when: the proposal exists and Aco has approved it in writing. Nothing below starts before that.

### Package 2: the schema as a published artefact

1. Put the schema under `spec/analytics/` beside the OKF specification: a prose specification, a JSON Schema per table, and the Arrow schema per table.
2. Versioning rule, stated in the spec: within a major version, changes are additive only. Every response and every exported file carries the schema version.
3. Licence header: follow whatever the OKF specification carries today, and flag it in your report for decision D1 of the companion brief. The intent is that the schema sits on the permissive side so others can build connectors.
4. `metric_definitions` is generated from the engine, not hand-written, so the Analytics page on the site and this table can never disagree.

Done when: the JSON Schemas validate; a test fails if an engine metric has no definition row.

### Package 3: REST

Read-only endpoints under the existing versioned API root:

1. `GET /analytics/schema`: the schema document and version.
2. `GET /analytics/{project}/tables/{table}`: parameters `commit` or `branch`, `format` (`json`, `csv`, `ndjson`), `limit`, `cursor`, and simple equality filters on key columns. Large tables stream. Content negotiation by `Accept` header as well as `format`.
3. `GET /analytics/{project}/metrics`: the `metrics` table for one commit or branch head.
4. `GET /analytics/{project}/trend`: parameters `metric`, `branch`, optional `from` and `to` commits. Returns the metric across the commits of a branch, in commit order, each row with its basis. Served from the cache in design decision 4.
5. `GET /metrics` in OpenMetrics format, off by default, enabled by the operator, behind the normal bearer token. Gauges per project and branch head, for example requirements total, requirements uncovered, coverage ratio, orphans, isolated nodes, blocking losses, elements. Use only measures that exist in the inventory.
6. Update the OpenAPI document. Add contract tests. Add these endpoints to the permissions table in `server/src/auth.rs` at the viewer role.

Done when: contract tests pass; the Thirty Meter Telescope import streams every table without holding it all in memory; a request for a project outside the caller's scope is refused.

### Package 4: CLI

New subcommand group, working in both `--server` and `--db` modes:

1. `mw analytics schema`
2. `mw analytics tables` (lists tables and row counts for a commit)
3. `mw analytics export --project P [--commit C | --branch B | --all-commits] --format csv|ndjson|parquet --out DIR`, using the layout in design decision 6.
4. `mw analytics metrics` and `mw analytics trend --metric M`, with `--format json|csv`.
5. Parquet through the Apache Arrow and Parquet crates. If that adds more than a modest amount to the build, stop and ask before adding it.
6. Exit codes and stderr follow the existing CLI conventions. Output is pipeline-safe: data on stdout, everything else on stderr.

Done when: `mw analytics export` of the sample corpus is byte-identical across two runs for CSV and NDJSON; DuckDB reads the Parquet tree with `hive_partitioning` and the row counts match `mw analytics tables`.

### Package 5: MCP

Add read-only analytics tools to `mw-mcp`. These serve analysis agents, which are a different audience from the modelling agents the existing tools serve.

1. `mw_analytics_schema`: tables, columns and metric definitions, so an agent can plan before it queries.
2. `mw_metrics`: metrics for a project at a commit or branch head, each with its basis.
3. `mw_trend`: one metric across the commits of a branch.
4. `mw_table`: bounded rows from one table, with key filters and a cursor. Default 50 rows, maximum 500, so a result fits in a model's context. Return the total row count with every page.
5. `mw_findings` and `mw_losses`.
6. Every tool result states the schema version, the project, the commit and, for any metric, the basis. An agent must never be handed a bare number.
7. Update `agents/CLAUDE.md`, `docs/guide/mcp-agents.md` and the rule packs: an agent reporting a metric must report its basis with it.
8. Stretch, ask first: a Streamable HTTP transport for `mw-mcp` using the server's token authentication, so a hosted assistant can connect without a local process. Do not start this without Aco's approval.

Done when: each tool has a contract test; the rule pack change is in all generated packs via `agents/generate.py`.

### Package 6: Python

1. In the `modelwrite` package: `analytics.tables(project, commit=None, branch=None)` returning Arrow tables, with `to_pandas()` available when pandas is installed. Arrow and pandas are optional extras, not hard dependencies.
2. The same helpers in the agent client in `bindings/python-agent`, over REST.
3. One worked notebook in `docs/examples/`: load the sample corpus, plot coverage across commits, list uncovered requirements, show the basis for each figure. Every number in it comes from the tables.

Done when: the notebook runs top to bottom from a clean environment in CI.

### Package 7: equivalence evidence

1. A test that, for the sample corpus and for the Thirty Meter Telescope import, fetches every table through REST, through the CLI in both modes, through MCP (paged to completion) and through Python, and asserts the rows are identical.
2. Write the result as a deterministic evidence record, `docs/evidence/analytics-transport-equivalence.json`, including a deliberate-failure case, in the same manner as the existing gate records.
3. Add it to CI alongside the other regenerated evidence.

Done when: the record is committed and CI regenerates it without a diff.

### Package 8: connecting other tools (documentation)

Narrative pages follow the two-step process. Reference tables generated from the schema and the OpenAPI document are exempt.

1. `docs/guide/analytics-interfaces.md`: the schema, the four transports, the basis rule, the versioning rule.
2. Short how-to pages, each ending with a query that reproduces a number shown on the Modelwrite overview page:
   - DuckDB over the Parquet tree;
   - pandas or Polars in a notebook;
   - Power BI and Excel, using the CSV endpoint or the Parquet folder;
   - Grafana, using the OpenMetrics endpoint or the JSON endpoint;
   - a data lake job that runs `mw import` then `mw analytics export` on a schedule.
3. Execute what you can script (DuckDB, pandas, the scheduled job). For the tools you cannot run (Power BI, Excel, Grafana), write the steps from the vendor's documentation and mark the page `NOT EXECUTED: written from vendor documentation` until Aco has run it. Third-party names are used descriptively, with the trade mark attribution line.
4. Add the interfaces to the Analytics page of the website, with status chips, once the packages they describe are merged.

Done when: every executed how-to is covered by a CI job; every unexecuted one carries its marker.

---

## 4. Investigate and report, do not build

1. **Read-only SQL views for BI tools.** Every BI tool speaks PostgreSQL, so a `mw_analytics` schema of views with a read-only database role is attractive. It also bypasses the server's roles, project scoping and audit log. Report how models are stored today, whether views are feasible without materialising tables, and what an operator would be giving up. No code.
2. **Inbound data.** The other half of interoperability is other systems' data joined to the model: test results, cost, schedule, defects, keyed by element or requirement id. Report what `qa_pipeline.rs` does today and propose the smallest registration API for an external CSV or Parquet dataset. No code. This is the next brief.
3. **Standards.** Note where the SysML v2 API and OSLC would sit relative to this schema. One page. No code.

---

## 5. Open and licensed

For Aco's decision, not the agent's. The working assumption, by the rule "free for the engineer, paid for the organisation":

- Open: the schema, the REST endpoints, the CLI export, the MCP tools, the Python helpers, the OpenMetrics endpoint.
- Licensed later: governed feeds for the organisation, meaning single sign-on on the feeds, row-level security by classification marking, a record of who pulled what, scheduled delivery into a customer's lake, and connectors to the proprietary systems that hold the rest of the programme's data.

---

## 6. Reporting

As in the companion brief: after Package 1 and at the end of every package, write a report listing the files changed, what was verified and how, anything marked `UNVERIFIED` or `NOT EXECUTED`, and questions for Aco. Stop and ask rather than guess.
