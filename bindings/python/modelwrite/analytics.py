# SPDX-License-Identifier: AGPL-3.0-or-later
"""High-level read-only helpers over the modelwrite analytics REST surface.

analytics.tables(project, commit=None, branch=None) returns Arrow tables (or a
dependency-free fallback) for the mw-analytics-schema@1 tables, with to_pandas()
available when pandas is installed. Arrow and pandas are optional extras, never
hard dependencies.

Every returned table carries the snake_case logical column names shared with the
CSV, Parquet and Arrow artefacts; the REST wire uses camelCase and this module
converts back.
"""

from ._client import Client, DEFAULT_BASE_URL
from ._schema import SCHEMA_VERSION, TABLES
from ._table import Tables

__all__ = [
    "tables",
    "metrics",
    "trend",
    "schema",
    "Client",
    "DEFAULT_BASE_URL",
    "SCHEMA_VERSION",
]


def _client(client, base_url, token, session, timeout):
    if client is not None:
        return client
    if any(value is not None for value in (base_url, token, session, timeout)):
        return Client(
            base_url=DEFAULT_BASE_URL if base_url is None else base_url,
            token=token,
            session=session,
            timeout=30 if timeout is None else timeout,
        )
    return Client()


def schema(base_url=None, token=None, client=None, session=None, timeout=None):
    """The schema document: schemaVersion, tables and metricDefinitions (raw dict)."""
    return _client(client, base_url, token, session, timeout).schema()


def tables(project, commit=None, branch=None, *, table=None, filters=None,
           base_url=None, token=None, client=None, session=None, timeout=None):
    """Read one table - or every table - for a project at a commit or branch head.

    commit and branch select the commit to read (commit wins; branch defaults to
    main). filters is an optional mapping of snake_case column name to value; the
    server applies each as an equality filter.

    table=None returns a Tables mapping of all nine tables; table="elements"
    returns just that one. Every table is the WHOLE table: the client follows the
    server's pagination to completion, so a call never returns one page.
    """
    c = _client(client, base_url, token, session, timeout)
    names = [table] if table is not None else list(TABLES)
    result = {}
    for name in names:
        result[name] = c.fetch_table(
            project, name, commit=commit, branch=branch, filters=filters
        )
    if table is not None:
        return result[table]
    return Tables(result, schema_version=SCHEMA_VERSION)


def metrics(project, commit=None, branch=None, *, base_url=None, token=None,
            client=None, session=None, timeout=None):
    """The metrics table for one commit or branch head, each row with its basis."""
    c = _client(client, base_url, token, session, timeout)
    return c.fetch_metrics(project, commit=commit, branch=branch)


def trend(project, metric, branch, *, from_commit=None, to_commit=None,
          base_url=None, token=None, client=None, session=None, timeout=None):
    """One metric across a branch's commits, in commit order, each row with basis."""
    c = _client(client, base_url, token, session, timeout)
    return c.fetch_trend(
        project, metric, branch, from_commit=from_commit, to_commit=to_commit
    )
