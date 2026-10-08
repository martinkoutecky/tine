#!/usr/bin/env python3
"""Run cargo-mutants against byte-identical copies of just the page_state tests."""
import argparse
import json
import os
from pathlib import Path
import re
import subprocess
from mutation_harness import ROOT, prepare


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--survivors", type=Path, help="rerun only MissedMutant entries in outcomes.json")
    ap.add_argument("--output", type=Path, default=ROOT / "scratch/s2/cargo-final")
    ap.add_argument("--only", help="regular expression for a final focused rerun")
    args = ap.parse_args()
    work = prepare()
    command = ["rtk", "proxy", "cargo", "mutants", "--no-config", "--in-place",
               "--dir", str(work), "--package", "tine-store", "--file", "src/page_state/mod.rs",
               "--output", str(args.output.resolve())]
    if args.only:
        command += ["--re", args.only]
    elif args.survivors:
        outcomes = json.loads(args.survivors.read_text())["outcomes"]
        names = [o["scenario"]["Mutant"]["name"] for o in outcomes if o["summary"] == "MissedMutant"]
        command += ["--re", "^(?:" + "|".join(re.escape(n) for n in names) + ")$"]
    command += ["--", "--lib", "page_state"]
    env = {**os.environ, "CARGO_TARGET_DIR": str(ROOT / "target/s2-mutants")}
    raise SystemExit(subprocess.run(command, cwd=ROOT, env=env).returncode)


if __name__ == "__main__":
    main()
