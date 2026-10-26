#!/usr/bin/env python3
"""Run cargo-mutants against byte-identical copies of just the page_state tests."""
import argparse
import json
import os
from pathlib import Path
import re
import subprocess
from mutation_harness import ROOT, prepare, SOURCES


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--survivors", type=Path, help="rerun only MissedMutant entries in outcomes.json")
    ap.add_argument("--output", type=Path, default=ROOT / "scratch/s3/cargo-final")
    ap.add_argument("--only", help="regular expression for a final focused rerun")
    ap.add_argument("--skip-witnesses", action="store_true", help="baseline census before s3 witnesses are regenerated")
    args = ap.parse_args()
    work = prepare()
    command = ["rtk", "proxy", "cargo", "mutants", "--no-config", "--in-place",
               "--dir", str(work), "--package", "tine-store",
               "--output", str(args.output.resolve())]
    for name in SOURCES:
        command += ["--file", name]
    if args.only:
        command += ["--re", args.only]
    elif args.survivors:
        outcomes = json.loads(args.survivors.read_text())["outcomes"]
        names = [o["scenario"]["Mutant"]["name"] for o in outcomes if o["summary"] == "MissedMutant"]
        command += ["--re", "^(?:" + "|".join(re.escape(n) for n in names) + ")$"]
    command += ["--", "--lib", "page_state"]
    if args.skip_witnesses:
        command += ["--", "--skip", "targeted_quint_witnesses"]
    env = {**os.environ, "CARGO_TARGET_DIR": str(ROOT / "target/s3-mutants")}
    raise SystemExit(subprocess.run(command, cwd=ROOT, env=env).returncode)


if __name__ == "__main__":
    main()
