# SPDX-License-Identifier: AGPL-3.0-or-later
"""The mw-analytics-schema@1 tables, columns and the name mapping.

This module is the Python mirror of the server's projection.rs: the nine tables
and their logical (snake_case) columns in schema order, plus the snake_case <->
camelCase mapping the REST wire applies.

Logical column names are snake_case at rest (CSV, Parquet, Arrow). The JSON wire
serialises those same columns in camelCase; the mapping is mechanical and lives in
server/src/analytics/format.rs. This client converts wire rows back to the
snake_case logical names, so a notebook sees the same column names as the CLI
export and the Arrow/Parquet artefacts.
"""

import re

#: The schema version every response and export carries.
SCHEMA_VERSION = "mw-analytics-schema@1"

#: The nine tables, in stable order, with their columns in schema order. This is
#: projection::TABLES, column for column.
TABLES = {
    "projects": ["project", "created_at"],
    "commits": [
        "hash",
        "project",
        "branch",
        "parents",
        "author",
        "message",
        "committed_at",
        "okf_hash",
        "provenance_kind",
        "import_artifact_hash",
        "binding_id",
        "binding_version",
    ],
    "elements": [
        "project",
        "commit",
        "element_id",
        "section",
        "name",
        "kind",
        "stereotypes",
        "attributes",
        "documentation",
    ],
    "relationships": ["project", "commit", "source", "target", "kind", "label"],
    "requirements": [
        "project",
        "commit",
        "element_id",
        "identifier",
        "text",
        "name",
        "kind",
        "covered",
        "covering_link_kinds",
    ],
    "trace_links": [
        "project",
        "commit",
        "requirement_id",
        "element_id",
        "link_kind",
        "source_id",
        "target_id",
    ],
    "metrics": [
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
    ],
    "metric_definitions": ["id", "name", "description", "depends_on", "status"],
    "import_losses": [
        "import_artifact_hash",
        "project",
        "binding_id",
        "binding_version",
        "construct",
        "subject",
        "severity",
        "note",
    ],
}

#: Column types for the columns that are NOT plain utf8 strings. Types use a small
#: Arrow vocabulary: "bool", "int64", "float64", "list<utf8>" and
#: "list<struct<attribute>>". Every other known column is utf8. This mirrors
#: spec/analytics/arrow/*.arrow.json; _table._arrow_type translates to pyarrow.
COLUMN_TYPES = {
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
        "value": "float64",  # nullable: null when value_state is unknown/uncosted
        "basis_element_count": "int64",
        "basis_relationship_count": "int64",
        "constructs_not_carried": "int64",
    },
    "metric_definitions": {"depends_on": "list<utf8>"},
}

#: The attribute struct fields (elements.attributes), all utf8. Single-word names,
#: so the camelCase wire passes them through unchanged.
ATTRIBUTE_FIELDS = ["name", "type", "aggregation", "default"]


def table_columns(name):
    """The snake_case columns of one table in schema order, or () if unknown."""
    return tuple(TABLES.get(name, ()))


def snake_to_camel(name):
    """snake_case -> camelCase, byte-for-byte the server's camel_case."""
    out = []
    upper = False
    for char in name:
        if char == "_":
            upper = True
        elif upper:
            out.append(char.upper())
            upper = False
        else:
            out.append(char)
    return "".join(out)


#: Exact reverse map for every v1 column: camelCase wire key -> snake_case.
_CAMEL_TO_SNAKE = {
    snake_to_camel(col): col for cols in TABLES.values() for col in cols
}


def camel_to_snake(name):
    """camelCase -> snake_case.

    Exact for every v1 column; a generic fallback covers additive future columns
    (the schema is additive-only within a major version).
    """
    exact = _CAMEL_TO_SNAKE.get(name)
    if exact is not None:
        return exact
    # Insert an underscore before each uppercase letter that follows a lowercase
    # letter or digit, then lower-case everything.
    with_underscores = re.sub(
        r"([a-z0-9])([A-Z])", lambda m: m.group(1) + "_" + m.group(2), name
    )
    return with_underscores.lower()


def to_snake_row(value):
    """Recursively map every object key of a wire row from camelCase to snake_case."""
    if isinstance(value, dict):
        return {camel_to_snake(k): to_snake_row(v) for k, v in value.items()}
    if isinstance(value, list):
        return [to_snake_row(item) for item in value]
    return value


def to_snake_rows(rows):
    """Map a list of wire rows (camelCase) to snake_case logical rows."""
    return [to_snake_row(row) for row in rows]
