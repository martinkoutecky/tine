#!/usr/bin/env python3
"""Preserve QF2's graph copies, pinned release runs and raw numeric evidence.

Build qf2_native_bench separately with --release --features test-faults, using the
task-authorized target directory. `prepare` never changes its source. `run` writes
checkpoints outside the graph copies. Cold means no checkpoint, not cold OS cache.
"""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess


def text_files(root):
    return sorted(p for p in root.rglob("*") if p.is_file() and p.suffix in (".md", ".org"))


def prepare(source, work):
    source, work = source.resolve(), work.resolve()
    if source == work or source in work.parents or work in source.parents:
        raise ValueError("source and evidence directories must be separate")
    corpora = work / "corpora"
    corpora.mkdir(parents=True, exist_ok=False)
    manifest = {str(p.relative_to(source)): hashlib.sha256(p.read_bytes()).hexdigest()
                for p in sorted(source.rglob("*")) if p.is_file()}
    (work / "source-sha256.json").write_text(json.dumps(manifest, indent=2) + "\n")
    small = corpora / "small"
    shutil.copytree(source, small)
    pages = small / "pages"
    pages.mkdir(exist_ok=True)
    for name, count in (("Hub", 1), ("Small", 1), ("Sixty", 60), ("Big", 2000)):
        with (pages / f"QF2 {name}.md").open("x") as out:
            out.writelines(f"- the qf2rareprobe benchmark block {i} [[QF2 Hub]]\n"
                           for i in range(count))
    for i in range(200):
        with (pages / f"QF2 Ref {i}.md").open("x") as out:
            out.write("- reference [[QF2 Hub]]\n")
    files = text_files(small)
    large = corpora / "13k"
    shutil.copytree(small, large)
    for i in range(13000 - len(files)):
        original = files[i % len(files)]
        shutil.copy2(original, large / "pages" / f"QF2 Scaled {i:05}{original.suffix}")
    print(json.dumps({name: {"files": len(text_files(corpora / name)),
                            "bytes": sum(p.stat().st_size for p in text_files(corpora / name))}
                      for name in ("small", "13k")}))


def run(args):
    work, binary = args.work.resolve(), args.binary.resolve()
    profiles = ["main", "extra"] if args.profile == "all" else [args.profile]
    for profile in profiles:
        suffix = "" if profile == "main" else "-extra"
        with (work / f"{args.label}{suffix}.jsonl").open("x") as log:
            for corpus in ("small", "13k"):
                for trial in range(args.repeats):
                    checkpoint = work / f"{args.label}-{corpus}-{trial}.checkpoint"
                    if profile == "main":
                        phases = [("cold", "launch", "cold"), ("seed", "launch", str(checkpoint)),
                                  ("warm", "launch", str(checkpoint)), ("actions", "actions", str(checkpoint)),
                                  ("memory", "memory", str(checkpoint))]
                    else:
                        if not checkpoint.is_file():
                            raise ValueError("run the main profile first to seed checkpoints")
                        phases = [("sixty-cold", "sixty", "cold"), ("sixty-warm", "sixty", str(checkpoint)),
                                  ("stages", "stages", str(checkpoint)), ("simple", "simple", str(checkpoint)),
                                  ("tql", "tql", str(checkpoint))]
                    for phase, mode, cp in phases:
                        result = subprocess.run(["taskset", "-c", args.cpus, str(binary),
                                                 str(work / "corpora" / corpus), cp, mode],
                                                capture_output=True, text=True, timeout=600)
                        (work / f"{args.label}-{corpus}-{trial}-{phase}.stderr").write_text(result.stderr)
                        result.check_returncode()
                        row = {"corpus": corpus, "trial": trial, "phase": phase,
                               "mode": mode, "cold": cp == "cold", "result": json.loads(result.stdout)}
                        log.write(json.dumps(row) + "\n")
                        log.flush()
                        print(args.label, corpus, trial, phase, round(row["result"]["readyMs"], 2), flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    prep = sub.add_parser("prepare")
    prep.add_argument("--source", type=Path, required=True)
    prep.add_argument("--work", type=Path, required=True)
    bench = sub.add_parser("run")
    bench.add_argument("--work", type=Path, required=True)
    bench.add_argument("--binary", type=Path, required=True)
    bench.add_argument("--label", required=True)
    bench.add_argument("--cpus", default="8,9")
    bench.add_argument("--repeats", type=int, default=5)
    bench.add_argument("--profile", choices=("main", "extra", "all"), default="all")
    args = parser.parse_args()
    if args.command == "prepare":
        prepare(args.source, args.work)
    else:
        run(args)


if __name__ == "__main__":
    main()
