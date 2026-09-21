# SPDX-License-Identifier: AGPL-3.0-or-later
"""Replay a recorded real-server capture without any server.

tests/fixtures/purchasing-terminal.json was recorded from a live mw-server by
tests/record_fixture.py. This is the fallback for environments that cannot start
the server: the always-on live test in test_live_server.py is the primary proof.
"""

import json
from pathlib import Path

from modelwrite import Client, analytics

from _fakes import BASE, FakeResponse

FIXTURE_PATH = Path(__file__).resolve().parent / "fixtures" / "purchasing-terminal.json"
FIXTURE = json.loads(FIXTURE_PATH.read_text(encoding="utf-8"))


class ReplaySession:
    """Serves the recorded envelopes; honours cursor/limit on the paginated set."""

    def __init__(self, fixture):
        self.fixture = fixture
        self.calls = []

    def request(self, method, url, params=None, headers=None, timeout=None):
        params = dict(params or {})
        self.calls.append({"url": url, "params": params})
        if url.endswith("/analytics/schema"):
            return FakeResponse(200, self.fixture["schema"])
        # The tables endpoint is checked before /metrics: /tables/metrics ends in
        # "/metrics" too, and is a different endpoint.
        if "/tables/" in url:
            table = url.rsplit("/", 1)[-1]
            paginated = self.fixture["paginated"]
            if (
                table == paginated["table"]
                and int(params.get("limit", 1000)) == paginated["limit"]
            ):
                index = int(params.get("cursor", 0)) // paginated["limit"]
                return FakeResponse(200, paginated["pages"][index])
            return FakeResponse(200, self.fixture["tables"][table])
        if "/analytics/" in url and url.endswith("/metrics"):
            return FakeResponse(
                200,
                {
                    "schemaVersion": self.fixture["schemaVersion"],
                    "metrics": self.fixture["metrics"]["rows"],
                },
            )
        if "/analytics/" in url and url.endswith("/trend"):
            return FakeResponse(
                200,
                {
                    "schemaVersion": self.fixture["schemaVersion"],
                    "metrics": self.fixture["metrics"]["rows"][:1],
                },
            )
        raise AssertionError("unexpected request: " + url)


def replay_client():
    return Client(base_url=BASE, session=ReplaySession(FIXTURE))


def test_recorded_tables_return_the_recorded_counts():
    result = analytics.tables("purchasing-terminal", client=replay_client())
    expected = FIXTURE["expected"]
    assert list(result.names) == list(expected)
    for name, count in expected.items():
        assert result[name].num_rows == count, name


def test_recorded_single_table_matches_the_recorded_total():
    table = analytics.tables(
        "purchasing-terminal", table="elements", client=replay_client()
    )
    assert table.num_rows == FIXTURE["tables"]["elements"]["total"] == 13
    assert "element_id" in table.column_names
    assert "elementId" not in table.column_names


def test_recorded_metrics_return_every_engine_measure():
    table = analytics.metrics("purchasing-terminal", client=replay_client())
    assert table.num_rows == FIXTURE["metrics"]["total"] == 25
    assert table.to_pylist()[0]["metric_id"]


def test_recorded_next_cursor_is_a_string_while_pages_remain():
    pages = FIXTURE["paginated"]["pages"]
    assert len(pages) > 1, "the recording must exercise real pagination"
    assert isinstance(pages[0]["nextCursor"], str)
    assert pages[-1]["nextCursor"] is None


def test_recorded_pagination_concatenates_to_the_whole_table():
    client = replay_client()
    recorded = FIXTURE["tables"]["elements"]
    pages = FIXTURE["paginated"]
    rows = []
    cursor = None
    guard = 0
    while True:
        page = client.table(
            "purchasing-terminal", "elements", cursor=cursor, limit=pages["limit"]
        )
        rows.extend(page["rows"])
        cursor = page.get("nextCursor")
        if cursor is None:
            break
        guard += 1
        assert guard < 100, "pagination did not terminate"
    assert len(rows) == recorded["total"]
    assert [row["elementId"] for row in rows] == [
        row["elementId"] for row in recorded["rows"]
    ]
