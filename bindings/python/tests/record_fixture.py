# SPDX-License-Identifier: AGPL-3.0-or-later
"""Record real mw-server analytics responses into tests/fixtures/.

Run it with a server binary available (MW_SERVER_BIN, or a cargo target):

    python bindings/python/tests/record_fixture.py

It starts a scratch mw-server, seeds e2e/models/purchasing-terminal.json over the
HTTP write path, captures every table's JSON envelope plus a genuinely paginated
page sequence for the elements table, and writes
tests/fixtures/purchasing-terminal.json. test_recorded_fixture.py replays that
capture, so the wire contract is still tested on a machine that cannot start the
server.
"""

import json
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

from conftest import MODELS_DIR, _server_binary, _start_server  # noqa: E402

SCHEMA_TABLES = [
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

MODEL = "purchasing-terminal"
PAGE_LIMIT = 5


def record():
    if _server_binary() is None:
        raise SystemExit(
            "no mw-server binary found: set MW_SERVER_BIN or build target/debug/mw-server"
        )
    with tempfile.TemporaryDirectory(prefix="mw-record-") as tmp:
        server = _start_server(Path(tmp))
        try:
            project, _model = server.seed(MODEL)

            tables = {}
            for table in SCHEMA_TABLES:
                response = server.get(
                    "/analytics/{}/tables/{}".format(project, table),
                    format="json",
                    limit=1000,
                )
                response.raise_for_status()
                envelope = response.json()
                tables[table] = {
                    "total": envelope["total"],
                    "nextCursor": envelope.get("nextCursor"),
                    "rows": envelope["rows"],
                }

            metrics_response = server.get("/analytics/{}/metrics".format(project))
            metrics_response.raise_for_status()
            metrics = metrics_response.json()

            schema = server.get("/analytics/schema").json()

            pages = []
            cursor = None
            while True:
                params = {"format": "json", "limit": PAGE_LIMIT}
                if cursor is not None:
                    params["cursor"] = cursor
                response = server.get(
                    "/analytics/{}/tables/elements".format(project), **params
                )
                response.raise_for_status()
                page = response.json()
                pages.append(
                    {
                        "total": page["total"],
                        "nextCursor": page.get("nextCursor"),
                        "rows": page["rows"],
                    }
                )
                cursor = page.get("nextCursor")
                if cursor is None:
                    break
                if len(pages) > 100:
                    raise RuntimeError("pagination did not terminate")

            fixture = {
                "_comment": (
                    "Recorded from a real mw-server by tests/record_fixture.py. "
                    "Rows are the camelCase JSON wire exactly as the server sent "
                    "them."
                ),
                "schemaVersion": schema["schemaVersion"],
                "model": MODEL,
                "expected": {name: tables[name]["total"] for name in SCHEMA_TABLES},
                "tables": tables,
                "metrics": {"total": len(metrics["metrics"]), "rows": metrics["metrics"]},
                "paginated": {"table": "elements", "limit": PAGE_LIMIT, "pages": pages},
                "schema": schema,
            }

            out = HERE / "fixtures" / (MODEL + ".json")
            out.parent.mkdir(parents=True, exist_ok=True)
            out.write_text(json.dumps(fixture, indent=2) + "\n", encoding="utf-8")
            print(
                "wrote {} ({} bytes); totals: {}".format(
                    out, out.stat().st_size, fixture["expected"]
                )
            )
        finally:
            server.stop()


if __name__ == "__main__":
    record()
