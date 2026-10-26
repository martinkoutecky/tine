#!/usr/bin/env python3
"""Check complete s3 port-mutation accounting against an exact-source ledger."""
import argparse
from collections import Counter
import json
from pathlib import Path
from mutation_harness import ROOT, source_sha256


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--ledger", type=Path, default=ROOT / "scripts/s2/equivalents.json")
    ap.add_argument("--hand-run", type=Path, action="append")
    ap.add_argument("--cargo-run", type=Path, action="append")
    args = ap.parse_args()
    ledger = json.loads(args.ledger.read_text())
    assert ledger["source_sha256"] == source_sha256(), "review ledger after source changes"
    hand, cargo = {}, {}
    hand_runs = args.hand_run or [ROOT / f"scratch/s3/hand-{n}" for n in ["baseline", "witness", "final"]]
    cargo_runs = args.cargo_run or [ROOT / f"scratch/s3/cargo-{n}" for n in ["baseline", "witness", "final"]]
    for run in hand_runs:
        path = run / "outcomes.json"
        if path.exists():
            for outcome in json.loads(path.read_text()):
                hand[outcome["id"]] = outcome
    for run in cargo_runs:
        path = run / "mutants.out/outcomes.json"
        if path.exists():
            for outcome in json.loads(path.read_text())["outcomes"]:
                if "Mutant" in outcome["scenario"]:
                    cargo[outcome["scenario"]["Mutant"]["name"]] = outcome
    assert len(hand) == ledger["census"]["hand"], ("incomplete hand census", len(hand))
    assert len(cargo) == ledger["census"]["cargo"], ("incomplete cargo census", len(cargo))
    for ident, outcome in hand.items():
        assert outcome["status"] in ["caught", "missed"], (ident, outcome["status"])
        if outcome["status"] == "missed":
            proof = ledger["hand"][ident]
            assert proof["dropped"] == outcome["dropped"] and proof["function"] == outcome["function"]
            assert proof["invariant"] and proof["argument"]
    for name, outcome in cargo.items():
        assert outcome["summary"] in ["CaughtMutant", "MissedMutant", "Unviable"], (name, outcome["summary"])
        if outcome["summary"] == "MissedMutant":
            proof = ledger["cargo"][name]
            assert proof["invariant"] and proof["argument"]
    # Stale permissions also fail: every reviewed survivor must still survive.
    assert set(ledger["hand"]) == {i for i, o in hand.items() if o["status"] == "missed"}
    assert set(ledger["cargo"]) == {n for n, o in cargo.items() if o["summary"] == "MissedMutant"}
    print("hand", dict(Counter(o["status"] for o in hand.values())))
    print("cargo", dict(Counter(o["summary"] for o in cargo.values())))
    print("Zero unexplained survivors; every permission is pinned to this source and exact mutation.")


if __name__ == "__main__":
    main()
