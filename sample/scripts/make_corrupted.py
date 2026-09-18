# SPDX-License-Identifier: AGPL-3.0-or-later
"""Produce the corrupted OKF fixture the gate must reject.

Deterministic breakage, applied to the reference corpus:
- every edge touching requirement 3.1.5 is removed, isolating that node;
- requirement 1.1 is deleted entirely (a lost element).
"""
import json
import sys
from pathlib import Path


def main() -> int:
    src = Path("sample/corpus/coffee-machine/okf/expected/coffee_machine_model.json")
    dst = Path("sample/corpus/coffee-machine/okf/corrupted/coffee_machine_model.json")
    data = json.loads(src.read_text(encoding="utf-8"))
    target = next(r["id"] for r in data["requirements"] if r["reqId"] == "3.1.5")
    before = len(data["graph"]["edges"])
    data["graph"]["edges"] = [
        e for e in data["graph"]["edges"]
        if e["source"] != target and e["target"] != target
    ]
    removed = before - len(data["graph"]["edges"])
    data["requirements"] = [r for r in data["requirements"] if r["reqId"] != "1.1"]
    dst.parent.mkdir(parents=True, exist_ok=True)
    dst.write_text(json.dumps(data, indent=2), encoding="utf-8")
    print(f"wrote {dst}: removed {removed} edges around REQ 3.1.5, deleted REQ 1.1")
    return 0


if __name__ == "__main__":
    sys.exit(main())
