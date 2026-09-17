# SPDX-License-Identifier: AGPL-3.0-or-later
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import judge


class JudgeTests(unittest.TestCase):
    def test_find_pairs(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            ref = root / "ref"
            cand = root / "cand"
            ref.mkdir()
            cand.mkdir()
            (ref / "a.json").write_text("{}", encoding="utf-8")
            (ref / "b.json").write_text("{}", encoding="utf-8")
            (cand / "a.json").write_text("{}", encoding="utf-8")
            pairs = judge.find_pairs(ref, cand)
            self.assertEqual(set(pairs), {"a"})

    def test_render_table(self):
        out = judge.render_table([
            ("a", {"passed": True, "failures": []}),
            ("b", {"passed": False, "failures": ["integration: 2 connected components"]}),
        ])
        self.assertIn("PASS  a", out)
        self.assertIn("FAIL  b", out)
        self.assertIn("2 connected components", out)


if __name__ == "__main__":
    unittest.main()
