# SPDX-License-Identifier: AGPL-3.0-or-later
"""The HTTP client: base URL, bearer-token auth and page-following."""

import requests

from ._schema import SCHEMA_VERSION, to_snake_rows
from ._table import build_table
from .errors import HTTPError, STATUS_ERRORS

#: Where a locally started mw-server listens by default (server/src/main.rs).
DEFAULT_BASE_URL = "http://127.0.0.1:8080"

#: The server's page-size ceiling (analytics::MAX_LIMIT). The client asks for the
#: maximum and follows nextCursor until it has every row.
MAX_LIMIT = 1000


class Client:
    """A read-only client for the mw-server analytics REST surface.

    base_url is the server root (for example http://127.0.0.1:8080). token is an
    optional bearer token; leave it None for the public showcase, which runs in
    open mode. session is an optional requests.Session (or any object exposing a
    compatible request method); pass one to reuse a connection pool or to inject
    a test double.
    """

    def __init__(self, base_url=DEFAULT_BASE_URL, token=None, session=None, timeout=30):
        self.base_url = (base_url or DEFAULT_BASE_URL).rstrip("/")
        self.token = token
        self.timeout = timeout
        self._session = session if session is not None else requests.Session()

    # -- low-level ---------------------------------------------------------

    def _headers(self):
        headers = {"Accept": "application/json"}
        if self.token:
            headers["Authorization"] = "Bearer " + self.token
        return headers

    def _get(self, path, params=None):
        url = self.base_url + path
        response = self._session.request(
            "GET", url, params=params, headers=self._headers(), timeout=self.timeout
        )
        return self._unwrap(response, url)

    def _unwrap(self, response, url):
        if response.status_code == 200:
            try:
                return response.json()
            except ValueError as exc:
                raise HTTPError(
                    200, "server returned a non-JSON 200 response", url=url
                ) from exc
        error_cls = STATUS_ERRORS.get(response.status_code, HTTPError)
        message = ""
        try:
            message = response.json().get("error", response.text)
        except ValueError:
            message = response.text
        raise error_cls(response.status_code, message, url=url, body=response.text)

    # -- endpoints ---------------------------------------------------------

    def schema(self):
        """The schema document: schemaVersion, tables and metricDefinitions."""
        return self._get("/analytics/schema")

    def table(self, project, table, commit=None, branch=None, filters=None,
              cursor=None, limit=MAX_LIMIT):
        """One PAGE of a table (the raw envelope), for callers who want pages."""
        params = {"format": "json", "limit": str(limit)}
        if commit is not None:
            params["commit"] = commit
        if branch is not None:
            params["branch"] = branch
        if cursor is not None:
            params["cursor"] = str(cursor)
        for key, value in (filters or {}).items():
            params[key] = value
        return self._get("/analytics/" + project + "/tables/" + table, params=params)

    def fetch_table(self, project, table, commit=None, branch=None, filters=None):
        """The WHOLE table: every page followed to completion.

        Returns a pyarrow.Table (or a plain Table without pyarrow) with snake_case
        columns.
        """
        cursor = None
        rows = []
        envelope = None
        while True:
            page = self.table(
                project, table, commit=commit, branch=branch,
                filters=filters, cursor=cursor, limit=MAX_LIMIT,
            )
            envelope = page
            page_rows = page.get("rows", [])
            rows.extend(page_rows)
            next_cursor = page.get("nextCursor")
            if not next_cursor or not page_rows:
                break
            cursor = int(next_cursor)
        schema_version = (
            envelope.get("schemaVersion", SCHEMA_VERSION) if envelope else SCHEMA_VERSION
        )
        return build_table(table, schema_version, to_snake_rows(rows))

    def metrics(self, project, commit=None, branch=None):
        """The metrics table for one commit or branch head (raw envelope)."""
        params = {}
        if commit is not None:
            params["commit"] = commit
        if branch is not None:
            params["branch"] = branch
        return self._get("/analytics/" + project + "/metrics", params=params)

    def fetch_metrics(self, project, commit=None, branch=None):
        """The metrics table as a table (snake_case columns)."""
        envelope = self.metrics(project, commit=commit, branch=branch)
        rows = envelope.get("metrics", [])
        schema_version = envelope.get("schemaVersion", SCHEMA_VERSION)
        return build_table("metrics", schema_version, to_snake_rows(rows))

    def trend(self, project, metric, branch, from_commit=None, to_commit=None):
        """One metric across a branch's commits (raw envelope)."""
        params = {"metric": metric, "branch": branch}
        if from_commit is not None:
            params["from"] = from_commit
        if to_commit is not None:
            params["to"] = to_commit
        return self._get("/analytics/" + project + "/trend", params=params)

    def fetch_trend(self, project, metric, branch, from_commit=None, to_commit=None):
        """A trend as a table (snake_case columns)."""
        envelope = self.trend(
            project, metric, branch, from_commit=from_commit, to_commit=to_commit
        )
        rows = envelope.get("metrics", [])
        schema_version = envelope.get("schemaVersion", SCHEMA_VERSION)
        return build_table("metrics", schema_version, to_snake_rows(rows))
