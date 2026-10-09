#!/usr/bin/env python3
"""Byte-identical isolated host/oracle fixture harness for the 2a census."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import re
import subprocess

ROOT = Path(__file__).resolve().parents[2]
WORK = ROOT / "scratch/page-host/mutation-harness"


def prepare():
    sources = []
    for directory in ["src/page_host", "src/page_state", "tests/fixtures/s2"]:
        for source in sorted((ROOT / "crates/tine-store" / directory).glob("*")):
            if source.is_file():
                relative = source.relative_to(ROOT / "crates/tine-store")
                dest = WORK / relative
                dest.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(source, dest)
                assert source.read_bytes() == dest.read_bytes()
                sources.append((str(relative), hashlib.sha256(source.read_bytes()).hexdigest()))
    (WORK / "src/lib.rs").write_text("mod page_state;\nmod page_host;\n")
    (WORK / "Cargo.toml").write_text(f'''[package]
name = "tine-store"
version = "0.0.0"
edition = "2021"
[workspace]
[dependencies]
serde = {{ version = "=1.0.228", features = ["derive", "rc"] }}
serde_json = "=1.0.150"
sha2 = "0.10"
postcard = {{ version = "1", features = ["use-std"] }}
uuid = {{ version = "1", features = ["v4"] }}
tine-core = {{ path = "{ROOT / 'crates/tine-core'}" }}
[profile.dev]
debug = 0
''')
    import json
    (WORK / "source-hashes.json").write_text(json.dumps(sources, indent=2) + "\n")
    return WORK


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--prepare", action="store_true")
    ap.add_argument("--survivors", type=Path)
    ap.add_argument("--only", help="cargo-mutants name regex")
    ap.add_argument("--test-filter", default="page_host")
    ap.add_argument("--output", type=Path, default=ROOT / "scratch/page-host/cargo-census")
    args = ap.parse_args()
    work = prepare()
    print(work, flush=True)
    if args.prepare:
        return
    binary = ROOT / "scratch/page-host/tools/bin/cargo-mutants"
    command = ["rtk", "proxy", str(binary), "mutants", "--no-config", "--dir", str(work),
               "--file", "src/page_host/*.rs", "--exclude", "**/conformance.rs",
               "--exclude", "**/scheduler.rs", "--exclude", "**/*tests.rs",
               "--exclude", "**/model_fs.rs", "--output", str(args.output.resolve()),
               "--jobs", "3", "--timeout", "90", "--build-timeout", "180",
               "--", "--lib", args.test_filter, "--", "--skip", "scheduler_random_walks",
               "--skip", "committed_witnesses_through_host"]
    filters = []
    if args.survivors:
        for outcome in json.loads(args.survivors.read_text())["outcomes"]:
            if outcome["summary"] == "MissedMutant":
                name = outcome["scenario"]["Mutant"]["name"]
                file, line, column, description = name.split(":", 3)
                filters.append(re.escape(file) + r":\d+:\d+:" + re.escape(description))
    if args.only:
        filters.append(args.only)
    if filters:
        command[command.index("--"):command.index("--")] = ["--re", "(?:" + "|".join(filters) + ")"]
    env = {**os.environ, "TINE_HOST_REPO_ROOT": str(ROOT)}
    # Parallel mutant crates have the same Cargo artifact names. An inherited
    # shared target can run another worker's binary or call a mutation Fresh.
    # Let cargo-mutants give every worker its own build directory instead.
    env.pop("CARGO_TARGET_DIR", None)
    temporary = ROOT / "scratch/page-host/tmp"
    temporary.mkdir(parents=True, exist_ok=True)
    env["TMPDIR"] = str(temporary)
    raise SystemExit(subprocess.run(command, cwd=ROOT, env=env).returncode)


if __name__ == "__main__":
    main()
