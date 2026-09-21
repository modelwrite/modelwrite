# SPDX-License-Identifier: AGPL-3.0-or-later
"""Table objects: Arrow tables when pyarrow is installed, a dependency-free
fallback otherwise. Both expose to_pandas() when pandas is installed."""

from ._schema import ATTRIBUTE_FIELDS, COLUMN_TYPES, table_columns


def build_table(name, schema_version, rows):
    """Build a table for a set of snake_case rows.

    Returns a pyarrow.Table when pyarrow is importable, else a plain Table.
    Both expose to_pandas() (which needs pandas) and column metadata.
    """
    try:
        import pyarrow  # noqa: F401
    except ImportError:
        return Table(name, schema_version, _columns(name, rows), rows)
    return _to_arrow(name, rows)


def _columns(name, rows):
    """The logical columns for a set of rows.

    The schema's columns in schema order, then any additive column first seen in
    the rows (the schema is additive-only within a major version). Shared by the
    Arrow and dependency-free representations so both agree on the column set.
    """
    columns = list(table_columns(name))
    seen = set(columns)
    for row in rows:
        for key in row:
            if key not in seen:
                seen.add(key)
                columns.append(key)
    return columns


def _to_arrow(name, rows):
    import pyarrow as pa

    columns = _columns(name, rows)

    arrays = []
    for col in columns:
        values = [row.get(col) for row in rows]
        col_type = _arrow_type(name, col)
        if col_type is None:
            arrays.append(pa.array(values))
        else:
            arrays.append(pa.array(values, type=col_type))
    return pa.Table.from_arrays(arrays, names=columns)


def _arrow_type(name, col):
    import pyarrow as pa

    type_name = COLUMN_TYPES.get(name, {}).get(col)
    if type_name is None:
        # A known column not listed in COLUMN_TYPES is a plain utf8 string; an
        # unknown (additive future) column is left for pyarrow to infer.
        return pa.string() if col in table_columns(name) else None
    if type_name == "bool":
        return pa.bool_()
    if type_name == "int64":
        return pa.int64()
    if type_name == "float64":
        return pa.float64()
    if type_name == "list<utf8>":
        return pa.list_(pa.string())
    if type_name == "list<struct<attribute>>":
        return pa.list_(pa.struct([pa.field(field, pa.string()) for field in ATTRIBUTE_FIELDS]))
    return pa.string()


class Table:
    """A dependency-free table used when pyarrow is not installed.

    Holds the snake_case logical columns and the rows as a list of dicts. It is a
    best-effort plain representation; install pyarrow for typed Arrow tables.
    """

    def __init__(self, name, schema_version, columns, rows):
        self.name = name
        self.schema_version = schema_version
        self._columns = list(columns)
        self._rows = rows

    @property
    def column_names(self):
        return list(self._columns)

    @property
    def num_rows(self):
        return len(self._rows)

    @property
    def num_columns(self):
        return len(self._columns)

    def to_pylist(self):
        """The rows as a list of plain dicts (snake_case keys)."""
        return [dict(row) for row in self._rows]

    def to_dicts(self):
        """Alias for to_pylist."""
        return self.to_pylist()

    def to_pandas(self):
        """The rows as a pandas DataFrame (requires pandas)."""
        try:
            import pandas as pd
        except ImportError as exc:
            raise ImportError(
                "pandas is required for Table.to_pandas(); install it with "
                "pip install modelwrite[pandas]"
            ) from exc
        return pd.DataFrame(self._rows, columns=self._columns)

    def __len__(self):
        return len(self._rows)

    def __iter__(self):
        return iter(self._rows)

    def __getitem__(self, index):
        return self._rows[index]

    def __repr__(self):
        return (
            "Table(name=" + repr(self.name) + ", rows=" + str(len(self._rows))
            + ", columns=" + repr(self._columns) + ")"
        )


class Tables(dict):
    """A mapping of table name -> table, with a to_pandas() convenience.

    analytics.tables(project) returns one of these: all nine tables at once.
    """

    def __init__(self, tables, schema_version=None):
        super().__init__(tables)
        self.schema_version = schema_version

    @property
    def names(self):
        return list(self.keys())

    def to_pandas(self):
        """A mapping of table name -> pandas DataFrame (requires pandas)."""
        return {name: table.to_pandas() for name, table in self.items()}

    def __repr__(self):
        return "Tables(" + ", ".join(repr(name) for name in self.keys()) + ")"
