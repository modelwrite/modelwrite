# SPDX-License-Identifier: AGPL-3.0-or-later
"""Read every mw-analytics-schema@1 table through the Python bindings and print
the rows as JSON on stdout.

This is the Python transport half of the cross-transport equivalence evidence
(server/tests/equivalence.rs, Package 7). It is deliberately thin: it imports the
public modelwrite package, calls analytics.tables(), and serialises the rows.
It computes nothing - the Python client is itself a thin REST client, so this
driver is not an independent implementation and the record says so.

Environment:
  MW_EQUIV_PYTHON_PATH  directory containing the modelwrite package (bindings/python)
  MW_EQUIV_BASE_URL     the running mw-server base URL
  MW_EQUIV_PROJECT      the project to read
  MW_EQUIV_COMMIT       the commit hash to read
"""

import json
import os
import sys


def main():
    package_path = os.environ["MW_EQUIV_PYTHON_PATH"]
    if package_path not in sys.path:
        sys.path.insert(0, package_path)

    from modelwrite import analytics

    project = os.environ["MW_EQUIV_PROJECT"]
    commit = os.environ["MW_EQUIV_COMMIT"]
    base_url = os.environ["MW_EQUIV_BASE_URL"]

    tables = analytics.tables(project, commit=commit, base_url=base_url)

    out = {}
    for name, table in tables.items():
        rows = table.to_pylist()
        out[name] = rows

    schema_version = getattr(tables, "schema_version", None)
    json.dump(
        {"schemaVersion": schema_version, "tables": out},
        sys.stdout,
        sort_keys=True,
    )
    sys.stdout.write("\n")


if __name__ == "__main__":
    main()
