# SPDX-License-Identifier: AGPL-3.0-or-later
"""Arrow and pandas must be optional extras, never hard dependencies.

If pyarrow is absent the client must still work (a plain Table), and if pandas is
absent only to_pandas() may fail - with a message that names the extra.
"""

import subprocess
import sys
from pathlib import Path

import pytest

from modelwrite._schema import TABLES
from modelwrite._table import Table, Tables, build_table

PYTHON_DIR = Path(__file__).resolve().parents[1]


def test_importing_the_package_does_not_import_the_optional_extras():
    # The package is required to be importable with neither pyarrow nor pandas
    # installed; an eager import would make them hard dependencies.
    code = (
        "import sys, json; import modelwrite; "
        "print(json.dumps(sorted(m for m in ('pyarrow', 'pandas') if m in sys.modules)))"
    )
    output = subprocess.check_output(
        [sys.executable, "-c", code], cwd=str(PYTHON_DIR)
    )
    assert output.decode().strip() == "[]"


def test_without_pyarrow_a_plain_table_is_returned(monkeypatch):
    monkeypatch.setitem(sys.modules, "pyarrow", None)
    table = build_table(
        "elements",
        "mw-analytics-schema@1",
        [{"element_id": "e1", "stereotypes": ["Block"]}],
    )
    assert isinstance(table, Table)
    assert table.column_names == list(TABLES["elements"])
    assert table.num_rows == 1
    assert table.to_pylist()[0]["element_id"] == "e1"


def test_additive_columns_pass_through_without_pyarrow(monkeypatch):
    # The schema is additive-only within a major version; an unknown future column
    # must survive the dependency-free representation too.
    monkeypatch.setitem(sys.modules, "pyarrow", None)
    table = build_table("elements", "v", [{"element_id": "e1", "future_column": "x"}])
    assert "future_column" in table.column_names
    assert table.to_pylist()[0]["future_column"] == "x"


def test_without_pandas_to_pandas_names_the_extra(monkeypatch):
    monkeypatch.setitem(sys.modules, "pandas", None)
    table = Table("elements", "v", TABLES["elements"], [{"element_id": "e1"}])
    with pytest.raises(ImportError) as caught:
        table.to_pandas()
    assert "modelwrite[pandas]" in str(caught.value)


def test_to_pandas_with_pandas_installed():
    pandas = pytest.importorskip("pandas")
    table = build_table(
        "elements",
        "mw-analytics-schema@1",
        [{"element_id": "e1", "name": "One"}, {"element_id": "e2", "name": "Two"}],
    )
    frame = table.to_pandas()
    assert isinstance(frame, pandas.DataFrame)
    assert list(frame["element_id"]) == ["e1", "e2"]
    assert "stereotypes" in frame.columns


def test_tables_mapping_to_pandas():
    pandas = pytest.importorskip("pandas")
    mapping = Tables(
        {
            "elements": Table(
                "elements", "v", TABLES["elements"], [{"element_id": "e1"}]
            )
        }
    )
    frames = mapping.to_pandas()
    assert set(frames) == {"elements"}
    assert isinstance(frames["elements"], pandas.DataFrame)


def test_arrow_types_for_the_typed_columns():
    pa = pytest.importorskip("pyarrow")

    elements = build_table(
        "elements",
        "v",
        [
            {
                "element_id": "e1",
                "stereotypes": ["Block"],
                "attributes": [
                    {"name": "a", "type": "T", "aggregation": "composite", "default": ""}
                ],
            }
        ],
    )
    assert isinstance(elements, pa.Table)
    assert pa.types.is_list(elements.schema.field("stereotypes").type)
    attributes_type = elements.schema.field("attributes").type
    assert pa.types.is_list(attributes_type)
    assert pa.types.is_struct(attributes_type.value_type)
    assert [f.name for f in attributes_type.value_type] == [
        "name",
        "type",
        "aggregation",
        "default",
    ]

    metrics = build_table(
        "metrics",
        "v",
        [
            {
                "metric_id": "coverage.covered",
                "value": 3,
                "basis_element_count": 1,
                "basis_relationship_count": 0,
                "constructs_not_carried": 0,
            }
        ],
    )
    assert pa.types.is_floating(metrics.schema.field("value").type)
    assert pa.types.is_integer(metrics.schema.field("basis_element_count").type)

    requirements = build_table(
        "requirements",
        "v",
        [{"element_id": "r", "covered": True, "covering_link_kinds": ["Satisfy"]}],
    )
    assert pa.types.is_boolean(requirements.schema.field("covered").type)
    assert pa.types.is_list(requirements.schema.field("covering_link_kinds").type)

    commits = build_table("commits", "v", [{"hash": "h", "parents": ["p"]}])
    assert pa.types.is_list(commits.schema.field("parents").type)


def test_arrow_to_pandas_when_both_are_installed():
    pytest.importorskip("pyarrow")
    pandas = pytest.importorskip("pandas")
    table = build_table("elements", "v", [{"element_id": "e1", "name": "One"}])
    frame = table.to_pandas()
    assert isinstance(frame, pandas.DataFrame)
    assert list(frame["element_id"]) == ["e1"]


def test_unknown_tables_fall_back_to_inferred_columns():
    pa = pytest.importorskip("pyarrow")
    table = build_table("future_table", "v", [{"a": 1, "b": "x"}])
    assert isinstance(table, pa.Table)
    assert sorted(table.column_names) == ["a", "b"]
