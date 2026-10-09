#!/usr/bin/env python3
"""Check the scoped host census; explanations must name each surviving mutant."""
import argparse
from collections import Counter
import json
from pathlib import Path
from page_host_mutations import ROOT
from page_host_hand import MUTATIONS


def outcomes(path):
    return json.loads(path.read_text())["outcomes"]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--extra", type=Path, help="additional new-source census")
    ap.add_argument("--current", type=Path, help="completed full current-source runtime census, without historic artifacts")
    args = ap.parse_args()
    artifact = ROOT / "scratch/page-host"
    if args.current:
        current = json.loads(args.current.read_text())
        assert current["end_time"] is not None, "current census must finish before reconciliation"
        latest = {o["scenario"]["Mutant"]["name"]:o for o in current["outcomes"] if isinstance(o["scenario"], dict)}
        listed = json.loads((args.current.parent / "mutants.json").read_text())
        assert set(latest) == {m["name"] for m in listed}, "every enumerated mutant needs an outcome"
    else:
        initial = outcomes(artifact / "cargo-census/mutants.out/outcomes.json")
        latest = {o["scenario"]["Mutant"]["name"]: o for o in initial if isinstance(o["scenario"], dict)}
        for directory in ["cargo-rerun", "cargo-save-guards"]:
            for o in outcomes(artifact / directory / "mutants.out/outcomes.json"):
                if isinstance(o["scenario"], dict):
                    name = o["scenario"]["Mutant"]["name"]
                    if name in latest:
                        latest[name] = o
    explanations = json.loads((ROOT / "scripts/s2/page_host_equivalents.json").read_text())
    counts = Counter()
    unresolved = []
    for name, o in latest.items():
        if o["summary"] == "MissedMutant" and name in explanations:
            counts["argued"] += 1
        elif o["summary"] == "CaughtMutant":
            counts["killed"] += 1
        elif o["summary"] == "Unviable":
            counts["unviable"] += 1
        else:
            unresolved.append((name,o["summary"]))
    hand = {o["name"]:o for o in json.loads((artifact / "hand-census/outcomes.json").read_text())}
    for rerun in sorted(artifact.glob("hand-rerun-*/outcomes.json")):
        for o in json.loads(rerun.read_text()):
            hand[o["name"]] = o
    assert set(hand) == {m[0] for m in MUTATIONS} and all(o["status"] == "killed" for o in hand.values()), hand
    result = {"core": {"total":len(latest), **counts}, "hand": {"total":len(hand), "killed":len(hand)}, "unresolved":unresolved}
    if args.extra:
        extra = Counter()
        for o in outcomes(args.extra):
            if not isinstance(o["scenario"],dict):
                continue
            name = o["scenario"]["Mutant"]["name"]
            status = o["summary"]
            if status == "MissedMutant" and name in explanations:
                status = "ArguedEquivalent"
            extra[status] += 1
        result["extra"] = dict(extra)
        assert not extra["MissedMutant"] and not extra["Timeout"], extra
    print(json.dumps(result,indent=2))
    (artifact / "final-census.json").write_text(json.dumps(result,indent=2)+"\n")
    assert not unresolved, unresolved


if __name__ == "__main__":
    main()
