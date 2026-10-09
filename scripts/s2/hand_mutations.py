#!/usr/bin/env python3
"""Enumerate/drop each operand of every Rust &&/|| expression, including nesting.

Requires tree-sitter 0.26.0 and tree-sitter-rust 0.24.2 (parser only).
Run --list to inspect; --run tests each change and restores the original in finally.
Never runs alongside cargo-mutants --in-place.
"""
import argparse
import json
import os
from pathlib import Path
import subprocess
import shutil
from mutation_harness import prepare, SOURCES, source_sha256
from tree_sitter import Language, Parser
import tree_sitter_rust

ROOT = Path(__file__).resolve().parents[2]


def mutations(source):
    tree = Parser(Language(tree_sitter_rust.language())).parse(source)
    assert not tree.root_node.has_error
    result = []

    def operator(node):
        if node.type == "binary_expression":
            op = node.child_by_field_name("operator")
            if op and op.text in (b"&&", b"||"):
                return op.text

    def visit(node, function=""):
        if node.type == "function_item":
            function = node.child_by_field_name("name").text.decode()
        op = operator(node)
        # Flatten only same-operator AST chains, preserving explicit grouping.
        if op and not (node.parent and operator(node.parent) == op):
            parts = []

            def flatten(n):
                if operator(n) == op:
                    flatten(n.child_by_field_name("left"))
                    flatten(n.child_by_field_name("right"))
                else:
                    parts.append(n)

            flatten(node)
            for i, part in enumerate(parts):
                replacement = b"(" + (b" " + op + b" ").join(
                    source[p.start_byte:p.end_byte]
                    for j, p in enumerate(parts) if j != i
                ) + b")"
                result.append({
                    "id": f"H{len(result) + 1:03}", "function": function,
                    "line": part.start_point.row + 1,
                    "dropped": part.text.decode(),
                    "start": node.start_byte, "end": node.end_byte,
                    "replacement": replacement.decode(),
                })
        for child in node.named_children:
            visit(child, function)

    visit(tree.root_node)
    return result


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--run", action="store_true")
    ap.add_argument("--skip-witnesses", action="store_true", help="baseline census before s3 witnesses are regenerated")
    ap.add_argument("--only", help="comma-separated IDs")
    ap.add_argument("--output", type=Path, default=ROOT / "scratch/s3/hand")
    ap.add_argument("--baseline-tests", type=Path, help="use saved step-1 tests for fail-before census")
    ap.add_argument("--allow-equivalents", type=Path, help="reviewed same-source equivalence ledger")
    args = ap.parse_args()
    originals = {name: (ROOT / "crates/tine-store" / name).read_bytes() for name in SOURCES}
    equivalents = {}
    if args.allow_equivalents:
        ledger = json.loads(args.allow_equivalents.read_text())
        assert ledger["source_sha256"] == source_sha256(), "equivalence ledger must be reviewed after source changes"
        equivalents = ledger["hand"]
    entries = []
    for name, original in originals.items():
        for entry in mutations(original):
            entry["id"] = f"H{len(entries) + 1:03}"
            entries.append({**entry, "file": name})
    assert any(m["function"] == "up_discard" and m["dropped"] == "d == x.s.drafts[p].bytes" for m in entries)
    args.output.mkdir(parents=True, exist_ok=True)
    (args.output / "mutants.json").write_text(json.dumps({
        "source_sha256": source_sha256(),
        "mutants": entries}, indent=2) + "\n")
    print(f"{len(entries)} hand mutants", flush=True)
    if not args.run:
        return
    work = prepare(args.output / "harness")
    if args.baseline_tests:
        shutil.copyfile(args.baseline_tests, work / "src/page_state/tests.rs")
    env = {**os.environ, "CARGO_TARGET_DIR": str(ROOT / "target/s3-hand-mutants")}
    outcomes = []
    try:
        for entry in entries:
            if args.only and entry["id"] not in args.only.split(","):
                continue
            for name, original in originals.items():
                (work / name).write_bytes(original)
            source = work / entry["file"]
            original = originals[entry["file"]]
            source.write_bytes(original[:entry["start"]] +
                               entry["replacement"].encode() + original[entry["end"]:])
            log = args.output / f"{entry['id']}.log"
            with log.open("w") as output:
                build = subprocess.run(["rtk", "proxy", "cargo", "test", "-p",
                                        "tine-store", "--lib", "--no-run"],
                                       cwd=work, env=env, stdout=output, stderr=output)
                if build.returncode:
                    status = "unviable"
                else:
                    test = subprocess.run(["rtk", "proxy", "cargo", "test", "-p",
                                           "tine-store", "--lib", "page_state"] + (["--", "--skip", "targeted_quint_witnesses"] if args.skip_witnesses else []),
                                          cwd=work, env=env, stdout=output, stderr=output)
                    status = "caught" if test.returncode else "missed"
            outcomes.append({**entry, "status": status})
            (args.output / "outcomes.json").write_text(json.dumps(outcomes, indent=2) + "\n")
            print(entry["id"], status, entry["function"], entry["dropped"], flush=True)
    finally:
        for name, original in originals.items():
            (work / name).write_bytes(original)
    for outcome in outcomes:
        if outcome["status"] == "missed" and outcome["id"] in equivalents:
            proof = equivalents[outcome["id"]]
            assert proof["dropped"] == outcome["dropped"] and proof["invariant"] and proof["argument"]
            outcome["equivalent"] = proof
    (args.output / "outcomes.json").write_text(json.dumps(outcomes, indent=2) + "\n")
    print("accounted:", sum(o["status"] == "caught" for o in outcomes), "caught,",
          sum("equivalent" in o for o in outcomes), "reviewed equivalents,",
          sum(o["status"] != "caught" and "equivalent" not in o for o in outcomes),
          "unexplained", flush=True)
    if any(o["status"] != "caught" and "equivalent" not in o for o in outcomes):
        raise SystemExit(1)


if __name__ == "__main__":
    main()
