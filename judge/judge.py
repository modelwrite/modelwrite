# SPDX-License-Identifier: AGPL-3.0-or-later
"""Gate-as-judge harness: run mw-gate over pairs of OKF files in two directories.

Standalone scoring for model migrations, or the scoring step in an agent
loop. Usage:

    python judge/judge.py --ref-dir DIR --candidate-dir DIR [--gate-bin PATH]
"""
import argparse
import json
import subprocess
import sys
from pathlib import Path


def find_pairs(ref_dir: Path, cand_dir: Path) -> dict[str, tuple[Path, Path]]:
    pairs: dict[str, tuple[Path, Path]] = {}
    for ref in sorted(ref_dir.glob("*.json")):
        cand = cand_dir / ref.name
        if cand.is_file():
            pairs[ref.stem] = (ref, cand)
    return pairs


def run_gate(gate_bin: Path, reference: Path, candidate: Path) -> dict:
    proc = subprocess.run(
        [str(gate_bin), "--reference", str(reference), "--candidate", str(candidate), "--json"],
        capture_output=True,
        text=True,
    )
    try:
        payload = json.loads(proc.stdout.strip())
        return payload
    except json.JSONDecodeError:
        return {
            "passed": False,
            "failures": [],
            "parseError": True,
            "raw": proc.stdout.strip(),
            "exitCode": proc.returncode,
            "stderr": proc.stderr.strip(),
        }


def render_table(results: list[tuple[str, dict]]) -> str:
    lines = []
    for name, payload in results:
        if payload.get("parseError"):
            lines.append(f"FAIL  {name}  (gate did not return JSON: {payload.get('stderr', '')})")
        elif payload.get("passed"):
            lines.append(f"PASS  {name}")
        else:
            reasons = "; ".join(payload.get("failures", []))
            lines.append(f"FAIL  {name}  ({reasons})")
    return "\n".join(lines)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Gate-as-judge harness for OKF migrations")
    parser.add_argument("--ref-dir", required=True, type=Path)
    parser.add_argument("--candidate-dir", required=True, type=Path)
    parser.add_argument("--gate-bin", default="mw-gate", type=Path)
    args = parser.parse_args(argv)
    pairs = find_pairs(args.ref_dir, args.candidate_dir)
    if not pairs:
        print("no matching OKF pairs found", file=sys.stderr)
        return 2
    results: list[tuple[str, dict]] = []
    for name, (ref, cand) in pairs.items():
        results.append((name, run_gate(args.gate_bin, ref, cand)))
    print(render_table(results))
    return 0 if all(payload.get("passed") for _, payload in results) else 1


if __name__ == "__main__":
    sys.exit(main())
