"""A renamed file counts at both ends: its base lines are deducted (REVIEW-3a V6)."""
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

SCRIPT = Path(__file__).with_name("net_lines.py")


def git(root, *args):
    subprocess.run(["git", "-C", root, *args], check=True, capture_output=True)


class RenameTest(unittest.TestCase):
    def test_a_renamed_test_file_is_not_counted_as_new(self):
        with tempfile.TemporaryDirectory() as root:
            git(root, "init", "-q")
            git(root, "config", "user.email", "t@example.invalid")
            git(root, "config", "user.name", "t")
            body = "".join(f"fn t{i}() {{}}\n" for i in range(40))
            (Path(root) / "old_tests.rs").write_text(body)
            git(root, "add", "-A")
            git(root, "commit", "-qm", "base")
            git(root, "mv", "old_tests.rs", "new_tests.rs")
            git(root, "commit", "-qm", "rename")
            out = subprocess.run(
                [sys.executable, SCRIPT, "--base", "HEAD~1", "--rev", "HEAD"],
                cwd=root,
                check=True,
                capture_output=True,
                text=True,
            ).stdout
            self.assertIn("production +0 ", out)
            self.assertIn("test +0 ", out, out)


if __name__ == "__main__":
    unittest.main()
