# SPDX-License-Identifier: AGPL-3.0-or-later
"""HTTP behaviour of the client, against an in-memory fake requests.Session.

No server is needed here: these tests exercise the request shape (auth header,
query parameters, pagination), the wire-to-logical mapping and the error
mapping. The live server tests in test_live_server.py prove the same behaviour
against the real binary.
"""

import pytest

from modelwrite import Client, analytics
from modelwrite._client import MAX_LIMIT
from modelwrite._schema import TABLES
from modelwrite._table import Tables
from modelwrite.errors import (
    BadRequest,
    Forbidden,
    HTTPError,
    NotAuthorized,
    NotFound,
    ServiceUnavailable,
)

from _fakes import BASE, FakeResponse, FakeSession, envelope, rows_of


def test_single_table_maps_wire_rows_to_snake_case():
    session = FakeSession(
        FakeResponse(
            200,
            envelope(
                "elements",
                [
                    {
                        "elementId": "e1",
                        "stereotypes": ["Block"],
                        "attributes": [
                            {"name": "a", "type": "T", "aggregation": "c", "default": ""}
                        ],
                    }
                ],
            ),
        )
    )
    client = Client(base_url=BASE, session=session)
    table = analytics.tables("p", table="elements", client=client)

    assert not isinstance(table, Tables)
    assert table.num_rows == 1
    assert "element_id" in table.column_names
    assert "elementId" not in table.column_names
    row = rows_of(table)[0]
    assert row["element_id"] == "e1"
    assert row["stereotypes"] == ["Block"]


def test_single_table_does_not_fetch_the_others():
    session = FakeSession(FakeResponse(200, envelope("elements", [])))
    client = Client(base_url=BASE, session=session)
    analytics.tables("p", table="elements", client=client)
    assert len(session.calls) == 1
    assert session.calls[0]["url"] == BASE + "/analytics/p/tables/elements"


def test_all_nine_tables_are_fetched_once_each():
    session = FakeSession(*[FakeResponse(200, envelope(name, [])) for name in TABLES])
    client = Client(base_url=BASE, session=session)
    result = analytics.tables("p", client=client)

    assert isinstance(result, Tables)
    assert list(result.names) == list(TABLES)
    assert len(session.calls) == 9
    assert all(call["params"]["limit"] == str(MAX_LIMIT) for call in session.calls)
    assert all(call["params"]["format"] == "json" for call in session.calls)


def test_fetch_table_follows_pagination_to_the_end():
    first = envelope(
        "elements",
        [{"elementId": "a"}, {"elementId": "b"}],
        total=3,
        next_cursor="2",
    )
    second = envelope("elements", [{"elementId": "c"}], total=3)
    session = FakeSession(FakeResponse(200, first), FakeResponse(200, second))
    client = Client(base_url=BASE, session=session)

    table = client.fetch_table("p", "elements")

    assert table.num_rows == 3
    assert [row["element_id"] for row in rows_of(table)] == ["a", "b", "c"]
    assert len(session.calls) == 2
    assert session.calls[0]["params"]["limit"] == str(MAX_LIMIT)
    assert "cursor" not in session.calls[0]["params"]
    assert session.calls[1]["params"]["cursor"] == "2"
    assert session.calls[1]["url"] == BASE + "/analytics/p/tables/elements"


def test_a_numeric_next_cursor_is_tolerated():
    # The server emits nextCursor as a JSON string, but a number must not break us.
    first = envelope("elements", [{"elementId": "a"}], total=2, next_cursor=1)
    second = envelope("elements", [{"elementId": "b"}], total=2)
    session = FakeSession(FakeResponse(200, first), FakeResponse(200, second))
    client = Client(base_url=BASE, session=session)
    assert client.fetch_table("p", "elements").num_rows == 2


def test_an_empty_page_terminates_pagination():
    session = FakeSession(
        FakeResponse(200, envelope("elements", [], total=0, next_cursor="0"))
    )
    client = Client(base_url=BASE, session=session)
    assert client.fetch_table("p", "elements").num_rows == 0
    assert len(session.calls) == 1


def test_filters_are_forwarded_as_snake_case_query_parameters():
    session = FakeSession(FakeResponse(200, envelope("elements", [])))
    client = Client(base_url=BASE, session=session)
    client.fetch_table(
        "p", "elements", filters={"kind": "block", "section": "structure"}
    )
    params = session.calls[0]["params"]
    assert params["kind"] == "block"
    assert params["section"] == "structure"
    assert params["format"] == "json"


def test_commit_and_branch_selectors_are_forwarded():
    session = FakeSession(FakeResponse(200, envelope("elements", [])))
    client = Client(base_url=BASE, session=session)
    client.fetch_table("p", "elements", commit="deadbeef", branch="dev")
    params = session.calls[0]["params"]
    assert params["commit"] == "deadbeef"
    assert params["branch"] == "dev"


def test_bearer_token_is_sent():
    session = FakeSession(FakeResponse(200, {"schemaVersion": "mw-analytics-schema@1"}))
    client = Client(base_url=BASE, token="s3cret", session=session)
    client.schema()
    assert session.calls[0]["headers"]["Authorization"] == "Bearer s3cret"


def test_no_token_means_no_authorization_header():
    session = FakeSession(FakeResponse(200, {"schemaVersion": "mw-analytics-schema@1"}))
    client = Client(base_url=BASE, session=session)
    client.schema()
    assert "Authorization" not in session.calls[0]["headers"]


def test_base_url_trailing_slash_is_normalised():
    session = FakeSession(FakeResponse(200, {"schemaVersion": "mw-analytics-schema@1"}))
    client = Client(base_url=BASE + "/", session=session)
    client.schema()
    assert session.calls[0]["url"] == BASE + "/analytics/schema"


@pytest.mark.parametrize(
    "status,error_type",
    [
        (400, BadRequest),
        (401, NotAuthorized),
        (403, Forbidden),
        (404, NotFound),
        (503, ServiceUnavailable),
        (409, HTTPError),
        (500, HTTPError),
    ],
)
def test_http_status_maps_to_a_typed_error(status, error_type):
    session = FakeSession(FakeResponse(status, {"error": "boom"}))
    client = Client(base_url=BASE, session=session)
    with pytest.raises(error_type) as caught:
        client.schema()
    assert caught.value.status_code == status
    assert "boom" in str(caught.value)


def test_a_non_json_success_response_raises_http_error():
    session = FakeSession(FakeResponse(200, None, text="<html>not json</html>"))
    client = Client(base_url=BASE, session=session)
    with pytest.raises(HTTPError) as caught:
        client.schema()
    assert caught.value.status_code == 200


def test_metrics_endpoint_maps_camel_case_rows():
    payload = {
        "schemaVersion": "mw-analytics-schema@1",
        "project": "p",
        "commit": "c",
        "metrics": [
            {
                "metricId": "coverage.covered",
                "value": 15,
                "valueState": "present",
                "basisElementCount": 25,
                "evidenceHash": "h",
            }
        ],
    }
    session = FakeSession(FakeResponse(200, payload))
    client = Client(base_url=BASE, session=session)
    table = analytics.metrics("p", client=client)

    assert table.num_rows == 1
    assert session.calls[0]["url"] == BASE + "/analytics/p/metrics"
    row = rows_of(table)[0]
    assert row["metric_id"] == "coverage.covered"
    assert row["basis_element_count"] == 25


def test_trend_forwards_metric_branch_and_range():
    payload = {
        "schemaVersion": "mw-analytics-schema@1",
        "metrics": [{"metricId": "coverage.covered"}],
    }
    session = FakeSession(FakeResponse(200, payload))
    client = Client(base_url=BASE, session=session)
    analytics.trend(
        "p", "coverage.covered", "main", from_commit="a", to_commit="b", client=client
    )
    assert session.calls[0]["url"] == BASE + "/analytics/p/trend"
    assert session.calls[0]["params"] == {
        "metric": "coverage.covered",
        "branch": "main",
        "from": "a",
        "to": "b",
    }


def test_schema_returns_the_raw_document():
    document = {
        "schemaVersion": "mw-analytics-schema@1",
        "tables": [{"name": "elements", "columns": ["element_id"]}],
        "metricDefinitions": [],
    }
    session = FakeSession(FakeResponse(200, document))
    client = Client(base_url=BASE, session=session)
    assert analytics.schema(client=client) == document
