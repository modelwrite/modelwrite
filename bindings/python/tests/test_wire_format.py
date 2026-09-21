# SPDX-License-Identifier: AGPL-3.0-or-later
"""The v1 schema mirror and the camelCase <-> snake_case wire mapping.

The server serialises the at-rest snake_case rows in camelCase on the JSON wire
(server/src/analytics/format.rs). These tests pin the client's mirror of that
mapping and of server/src/analytics/projection.rs column for column, so a server
schema change cannot silently drift the Python package.
"""

import pytest

from modelwrite._schema import (
    ATTRIBUTE_FIELDS,
    COLUMN_TYPES,
    SCHEMA_VERSION,
    TABLES,
    camel_to_snake,
    snake_to_camel,
    to_snake_row,
    to_snake_rows,
)

EXPECTED_TABLES = [
    "projects",
    "commits",
    "elements",
    "relationships",
    "requirements",
    "trace_links",
    "metrics",
    "metric_definitions",
    "import_losses",
]


def test_schema_version_is_the_v1_string():
    assert SCHEMA_VERSION == "mw-analytics-schema@1"


def test_the_nine_tables_are_in_stable_order():
    assert list(TABLES) == EXPECTED_TABLES


def test_columns_mirror_the_server_projection_column_for_column():
    assert TABLES["projects"] == ["project", "created_at"]
    assert TABLES["elements"] == [
        "project",
        "commit",
        "element_id",
        "section",
        "name",
        "kind",
        "stereotypes",
        "attributes",
        "documentation",
    ]
    assert TABLES["metrics"] == [
        "project",
        "commit",
        "metric_id",
        "value",
        "value_state",
        "unit",
        "trust",
        "basis_element_count",
        "basis_relationship_count",
        "constructs_not_carried",
        "basis_note",
        "engine_version",
        "evidence_hash",
    ]
    assert TABLES["import_losses"] == [
        "import_artifact_hash",
        "project",
        "binding_id",
        "binding_version",
        "construct",
        "subject",
        "severity",
        "note",
    ]


@pytest.mark.parametrize(
    "table,column,wire",
    [
        ("metrics", "basis_element_count", "basisElementCount"),
        ("metrics", "basis_relationship_count", "basisRelationshipCount"),
        ("metrics", "constructs_not_carried", "constructsNotCarried"),
        ("metrics", "value_state", "valueState"),
        ("metrics", "metric_id", "metricId"),
        ("metrics", "evidence_hash", "evidenceHash"),
        ("elements", "element_id", "elementId"),
        ("commits", "okf_hash", "okfHash"),
        ("commits", "provenance_kind", "provenanceKind"),
        ("commits", "import_artifact_hash", "importArtifactHash"),
        ("commits", "committed_at", "committedAt"),
        ("requirements", "covering_link_kinds", "coveringLinkKinds"),
        ("import_losses", "import_artifact_hash", "importArtifactHash"),
        ("trace_links", "requirement_id", "requirementId"),
        ("projects", "created_at", "createdAt"),
        ("metric_definitions", "depends_on", "dependsOn"),
    ],
)
def test_snake_to_camel_matches_the_server(table, column, wire):
    assert column in TABLES[table]
    assert snake_to_camel(column) == wire
    assert camel_to_snake(wire) == column


def test_every_v1_column_round_trips():
    for columns in TABLES.values():
        for column in columns:
            assert camel_to_snake(snake_to_camel(column)) == column


def test_single_word_columns_pass_through_unchanged():
    for columns in TABLES.values():
        for column in columns:
            if "_" not in column:
                assert snake_to_camel(column) == column
                assert camel_to_snake(column) == column


def test_generic_fallback_for_an_additive_future_column():
    assert camel_to_snake("someFutureColumn") == "some_future_column"


def test_nested_rows_map_recursively():
    wire = {
        "elementId": "e1",
        "stereotypes": ["Block"],
        "attributes": [
            {"name": "a", "type": "T", "aggregation": "composite", "default": ""}
        ],
    }
    assert to_snake_row(wire) == {
        "element_id": "e1",
        "stereotypes": ["Block"],
        "attributes": [
            {"name": "a", "type": "T", "aggregation": "composite", "default": ""}
        ],
    }


def test_to_snake_rows_maps_each_row():
    rows = to_snake_rows([{"metricId": "a"}, {"metricId": "b"}])
    assert rows == [{"metric_id": "a"}, {"metric_id": "b"}]


def test_attribute_struct_fields_are_single_words():
    # Only single-word fields would pass through the camelCase wire unchanged.
    assert ATTRIBUTE_FIELDS == ["name", "type", "aggregation", "default"]


def test_typed_columns_are_only_the_documented_ones():
    assert COLUMN_TYPES == {
        "commits": {"parents": "list<utf8>"},
        "elements": {
            "stereotypes": "list<utf8>",
            "attributes": "list<struct<attribute>>",
        },
        "requirements": {
            "covered": "bool",
            "covering_link_kinds": "list<utf8>",
        },
        "metrics": {
            "value": "float64",
            "basis_element_count": "int64",
            "basis_relationship_count": "int64",
            "constructs_not_carried": "int64",
        },
        "metric_definitions": {"depends_on": "list<utf8>"},
    }
