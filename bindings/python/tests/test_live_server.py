# SPDX-License-Identifier: AGPL-3.0-or-later
"""Live tests: a real mw-server on a scratch MW_DB, seeded from e2e/models/*.json.

Approach: start the server binary the way mw-server starts in production (MW_DB
pointing at a scratch SQLite file, loopback, open mode), seed each self-contained
e2e model through the same HTTP write path every real model uses, then assert
every table's row count against an oracle recomputed *here* from the source OKF
JSON - not against the server's own numbers.

cafe-stand.json is deliberately excluded: it pins cross-project revisions that no
longer resolve, so the server refuses it with 422. That fact is pinned by
test_cafe_stand_needs_its_referenced_projects rather than hidden.

The tests are marked "live" and are skipped, never failed, when no server binary
is available (set MW_SERVER_BIN or MW_TEST_BASE_URL).
"""

import os

import pytest

from modelwrite import Client, analytics
from modelwrite._schema import TABLES
from modelwrite.errors import NotAuthorized, NotFound

from conftest import SELF_CONTAINED_MODELS, TEST_TOKEN, load_model

pytestmark = pytest.mark.live

METRIC_COUNT = 25
METRIC_DEFINITION_COUNT = 25


def oracle_counts(model):
    """The row counts the source OKF JSON implies, recomputed independently."""
    elements = sum(
        len(model.get(section) or [])
        for section in ("structure", "interfaces", "signals")
    )
    edges = (model.get("graph") or {}).get("edges") or []
    requirements = model.get("requirements") or []
    requirement_ids = {requirement["id"] for requirement in requirements}

    trace_links = 0
    for edge in edges:
        if edge.get("kind") != "dependency":
            continue
        label = edge.get("label")
        source = edge.get("source")
        target = edge.get("target")
        if label in ("Satisfy", "Refine", "Verify") and target in requirement_ids:
            trace_links += 1
        elif label == "Allocate" and (
            source in requirement_ids or target in requirement_ids
        ):
            trace_links += 1

    return {
        "projects": 1,
        "commits": 1,
        "elements": elements,
        "relationships": len(edges),
        "requirements": len(requirements),
        "trace_links": trace_links,
        "metrics": METRIC_COUNT,
        "metric_definitions": METRIC_DEFINITION_COUNT,
        "import_losses": 0,
    }


@pytest.mark.parametrize("model_name", SELF_CONTAINED_MODELS)
def test_row_counts_match_the_source_model(seed, client, model_name):
    project, model = seed(model_name)
    tables = analytics.tables(project, client=client)
    expected = oracle_counts(model)

    assert list(tables.names) == list(TABLES)
    for name, count in expected.items():
        assert tables[name].num_rows == count, (model_name, name)


@pytest.mark.parametrize("model_name", SELF_CONTAINED_MODELS)
def test_every_table_is_complete_not_one_page(seed, client, model_name):
    project, _model = seed(model_name)
    for name in TABLES:
        table = analytics.tables(project, table=name, client=client)
        # The server's own total for the whole table; limit=1 proves we did not
        # just return the (single) page we happened to request.
        total = client.table(project, name, limit=1)["total"]
        assert table.num_rows == total, (model_name, name)


def test_tables_are_typed_and_convert_to_pandas(seed, client):
    pandas = pytest.importorskip("pandas")
    project, model = seed("purchasing-terminal")

    elements = analytics.tables(project, table="elements", client=client)
    assert "element_id" in elements.column_names
    assert "elementId" not in elements.column_names

    frame = elements.to_pandas()
    assert isinstance(frame, pandas.DataFrame)
    assert len(frame) == oracle_counts(model)["elements"]
    assert "stereotypes" in frame.columns


def test_branch_head_and_explicit_commit_agree(seed, client):
    project, _model = seed("purchasing-terminal")

    commits = analytics.tables(project, table="commits", client=client)
    tip = commits.to_pylist()[0]["hash"]

    by_branch = analytics.tables(project, branch="main", table="elements", client=client)
    by_commit = analytics.tables(project, commit=tip, table="elements", client=client)

    assert by_branch.num_rows == by_commit.num_rows
    assert by_branch.to_pylist() == by_commit.to_pylist()


def test_metrics_and_trend(seed, client):
    project, _model = seed("purchasing-terminal")

    metrics = analytics.metrics(project, client=client)
    assert metrics.num_rows == METRIC_COUNT

    trend = analytics.trend(project, "coverage.covered", "main", client=client)
    assert trend.num_rows == 1  # one commit on main
    assert trend.to_pylist()[0]["metric_id"] == "coverage.covered"


def test_string_filter_is_applied_server_side(seed, client):
    project, _model = seed("purchasing-terminal")

    everything = analytics.tables(project, table="elements", client=client).to_pylist()
    value = everything[0]["kind"]
    expected = sum(1 for row in everything if row["kind"] == value)

    filtered = analytics.tables(
        project, table="elements", filters={"kind": value}, client=client
    )
    assert filtered.num_rows == expected > 0
    assert all(row["kind"] == value for row in filtered.to_pylist())


def test_missing_project_is_a_404(client):
    with pytest.raises(NotFound):
        analytics.tables("no-such-project", table="elements", client=client)


def test_unknown_table_is_a_404(seed, client):
    project, _model = seed("purchasing-terminal")
    with pytest.raises(NotFound):
        analytics.tables(project, table="no_such_table", client=client)


def test_cafe_stand_needs_its_referenced_projects(mw_server):
    """cafe-stand.json is not standalone: its references pin revisions that no
    longer resolve, so the four self-contained models are the live fixtures."""
    model = load_model("cafe-stand")
    project = "cafe-stand-unresolved"
    mw_server.create_project(project)
    response = mw_server.commit_model(project, model)
    assert response.status_code == 422
    assert "does not resolve" in response.text


def test_bearer_auth_is_enforced_end_to_end(mw_server_with_token):
    if os.environ.get("MW_TEST_BASE_URL"):
        pytest.skip("MW_TEST_BASE_URL targets a server whose auth we do not control")

    anonymous = Client(base_url=mw_server_with_token.base_url)
    with pytest.raises(NotAuthorized):
        anonymous.schema()

    wrong = Client(base_url=mw_server_with_token.base_url, token="wrong-token")
    with pytest.raises(NotAuthorized):
        wrong.schema()

    right = Client(base_url=mw_server_with_token.base_url, token=TEST_TOKEN)
    assert right.schema()["schemaVersion"] == "mw-analytics-schema@1"
