# SPDX-License-Identifier: AGPL-3.0-or-later
"""In-memory doubles shared by the always-on tests (no server needed)."""

BASE = "http://mw.test:8080"


class FakeResponse:
    def __init__(self, status_code, payload=None, text=None):
        self.status_code = status_code
        self._payload = payload
        self.text = text if text is not None else ("" if payload is None else str(payload))

    def json(self):
        if self._payload is None:
            raise ValueError("not json")
        return self._payload


class FakeSession:
    """A requests.Session double: returns queued responses, records every call."""

    def __init__(self, *responses):
        self._responses = list(responses)
        self.calls = []

    def request(self, method, url, params=None, headers=None, timeout=None):
        self.calls.append(
            {
                "method": method,
                "url": url,
                "params": dict(params or {}),
                "headers": dict(headers or {}),
                "timeout": timeout,
            }
        )
        if not self._responses:
            raise AssertionError("FakeSession ran out of responses")
        return self._responses.pop(0)


def envelope(table, rows, total=None, next_cursor=None):
    payload = {
        "schemaVersion": "mw-analytics-schema@1",
        "project": "p",
        "commit": "c",
        "table": table,
        "total": len(rows) if total is None else total,
        "rows": rows,
    }
    if next_cursor is not None:
        payload["nextCursor"] = next_cursor
    return payload


def rows_of(table):
    return table.to_pylist()
