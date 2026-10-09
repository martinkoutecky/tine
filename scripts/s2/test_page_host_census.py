"""Current mutation evidence must not be overwritten by historic hand reruns."""
import contextlib
import io
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import page_host_census as census


class CurrentCensusTest(unittest.TestCase):
    def test_current_survivor_cannot_be_hidden_by_stale_rerun(self):
        self.check_current(selected=True, survivor=True)

    def test_current_requires_explicit_hand_run(self):
        self.check_current(selected=False, survivor=False)

    def test_current_selected_green_hand_run_ignores_stale_survivor(self):
        self.check_current(selected=True, survivor=False)

    def check_current(self, selected, survivor):
        with tempfile.TemporaryDirectory(dir=census.ROOT / "scratch/page-host") as temporary:
            root = Path(temporary)
            artifact = root / "scratch/page-host"
            hand = artifact / "hand-census/outcomes.json"
            hand.parent.mkdir(parents=True)
            hand.write_text(json.dumps([
                {"name": name, "status": "survived" if survivor and name == "H-draft-backoff" else "killed"}
                for name, *_ in census.MUTATIONS
            ]))
            stale = artifact / "hand-rerun-stale/outcomes.json"
            stale.parent.mkdir()
            stale.write_text(json.dumps([{"name": "H-draft-backoff", "status": "killed" if survivor else "survived"}]))
            current = artifact / "current/outcomes.json"
            current.parent.mkdir()
            current.write_text(json.dumps({"end_time": "finished", "outcomes": []}))
            (current.parent / "mutants.json").write_text("[]")
            ledger = root / "scripts/s2/page_host_equivalents.json"
            ledger.parent.mkdir(parents=True)
            ledger.write_text("{}")
            argv = ["page_host_census.py", "--current", str(current)]
            if selected:
                argv += ["--hand-current", str(hand)]
            with patch.object(census, "ROOT", root), patch("sys.argv", argv), contextlib.redirect_stdout(io.StringIO()):
                if survivor or not selected:
                    with self.assertRaises(AssertionError):
                        census.main()
                else:
                    census.main()
                    result = json.loads((artifact / "final-census.json").read_text())
                    self.assertEqual(result["hand"]["killed"], len(census.MUTATIONS))


if __name__ == "__main__":
    unittest.main()
