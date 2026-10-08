#!/usr/bin/env python3
"""Find short branch witnesses in scratch copies of the hash-pinned Quint model.

The staged driver uses only original actions; it changes scheduling, not rules.
An invariant rejects the requested reachable state, producing a counterexample.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
from generate import SHA, PROFILES, command, decode

ROOT = Path(__file__).resolve().parents[2]

OPEN = [("wOpen", [0]), ("deliverUp", [1]), ("wRecv", [0])]
EDIT = [("wEdit", [0, 2]), ("wSend", [0]), ("deliverUp", [1])]
WITNESSES = {
    "discard-draft-equals-disk-not-buffer": {
        "profile": "base",
        "actions": OPEN + EDIT + [("switchReq", []), ("draftSync", [0]),
            ("wRecv", [0]), ("wEdit", [0, 3]), ("wSend", [0]),
            ("deliverUp", [1]), ("wRecv", [0]), ("extWriteD", [0, 2, 0]),
            ("wDiscard", [0]), ("deliverUp", [1])],
        "predicate": "s.pages.get(0).risk and not(s.pages.get(0).typed) and s.pages.get(0).buf == s.drafts.get(0).bytes",
    },
}
WITNESSES["resolve-after-held-discard"] = {
    "profile": "base",
    "actions": WITNESSES["discard-draft-equals-disk-not-buffer"]["actions"] + [
        ("wRecv", [0]), ("extWriteD", [0, 1, 1]), ("observe", [0]),
        ("wRecv", [0]), ("wResolve", [0, 3]), ("deliverUp", [1])],
    "predicate": "s.pages.get(0).typed and s.pages.get(0).buf == 3 and s.pages.get(0).risk",
}
WITNESSES["resolve-after-backend-cleaned-stale-conflict"] = {
    "profile": "base", "actions": OPEN + EDIT + [("wRecv", [0]),
        ("extWriteD", [0, 3, 1]), ("observe", [0]), ("wRecv", [0]),
        ("extWriteD", [0, 1, 1]), ("observe", [0]), ("flush", [0]), ("check", []),
        ("rename", []), ("dirSync", [1]), ("wResolve", [0, 2]), ("deliverUp", [1])],
    "predicate": "s.pages.get(0).risk and s.pages.get(0).conflict and s.pages.get(0).base == 3",
}
WITNESSES["move-with-destination-save-lock"] = {
    "profile": "base",
    "actions": OPEN + [("wOpen", [1]), ("deliverUp", [1]), ("wRecv", [1])] +
        EDIT + [("wRecv", [0]), ("flush", [0]), ("check", []), ("wOp", [1, 3, 1]),
        ("saveFail", []), ("deliverUp", [1]), ("wRecv", [0]), ("wRecv", [1])],
    "predicate": "s.drafts.get(0).bytes == 1 and s.pages.get(1).buf == 3",
}
WITNESSES["move-draft-write-refused"] = {
    "profile": "base", "actions": OPEN + [("wOpen", [1]), ("deliverUp", [1]),
        ("wRecv", [1]), ("wOp", [0, 3, 3]), ("deliverUp", [0])],
    "predicate": "s.pages.get(0).buf == 1 and s.pages.get(1).buf == 2 and s.drafts.get(1).bytes == NONE",
}
WITNESSES["save-phases-and-r1-race"] = {
    "profile": "R1",
    "actions": OPEN + EDIT + [("wRecv", [0]), ("flush", [0]), ("check", []),
        ("extWriteD", [0, 3, 0]), ("rename", []), ("extWriteD", [0, 3, 0]),
        ("dirSync", [1])],
    "predicate": "s.disk.get(0) == 3 and s.stable.get(0) == 1 and g.promise.get(0).saved",
}
WITNESSES["ended-promise-same-version-draft"] = {
    "profile": "base",
    "actions": OPEN + EDIT + [("switchReq", []), ("draftSync", [0]),
        ("wRecv", [0]), ("wEdit", [0, 3]), ("wSend", [0]), ("deliverUp", [1]),
        ("wRecv", [0]), ("extWriteD", [0, 3, 0]), ("wDiscard", [0]),
        ("deliverUp", [1]), ("draftSync", [0])],
    "predicate": "not(g.promise.get(0).on) and s.drafts.get(0).bytes == 3",
}
WITNESSES["save-after-ended-promise-same-version"] = {
    "profile": "base",
    "actions": WITNESSES["ended-promise-same-version-draft"]["actions"] +
        [("flush", [0]), ("check", []), ("rename", []), ("dirSync", [1])],
    "predicate": "not(g.promise.get(0).on) and s.disk.get(0) == 3",
}
WITNESSES["discard-must-not-revive-ended-promise"] = {
    "profile": "base",
    "actions": WITNESSES["save-after-ended-promise-same-version"]["actions"] +
        [("wRecv", [0]), ("extWriteD", [0, 2, 1]), ("wDiscard", [0]), ("deliverUp", [1])],
    "predicate": "not(g.promise.get(0).on) and g.promise.get(0).ver == g.vc",
}
SAVED = OPEN + EDIT + [("wRecv", [0]), ("flush", [0]), ("check", []),
                      ("rename", []), ("dirSync", [1])]
WITNESSES["saved-promise-live-discard"] = {
    "profile": "base", "actions": SAVED + [("wDiscard", [0]), ("deliverUp", [1])],
    "predicate": "g.promise.get(0).on and g.promise.get(0).saved and g.ext.get(0) == 0",
}
WITNESSES["saved-promise-exempt-discard"] = {
    "profile": "base", "actions": SAVED + [("extWriteD", [0, 3, 1]),
        ("extWriteD", [0, 2, 0]), ("wDiscard", [0]), ("deliverUp", [1])],
    "predicate": "not(g.promise.get(0).on) and g.ext.get(0) == 2",
}
WITNESSES["weak-power-and-relaunch"] = {
    "profile": "weak", "actions": SAVED + [("power", [0, 0]), ("launch", [])],
    "predicate": "s.alive and not(g.promise.get(0).on) and s.disk.get(0) == 1",
}
WITNESSES["strong-power-retains-exempt-promise"] = {
    "profile": "base", "actions": SAVED + [("extWriteD", [0, 3, 1]), ("power", [0, 0])],
    "predicate": "g.promise.get(0).on and g.promise.get(0).saved and s.disk.get(0) == 3",
}
WITNESSES["clean-close-reopen"] = {
    "profile": "base", "actions": OPEN + [("wClose", [0]), ("deliverUp", [1]),
        ("wOpen", [0]), ("deliverUp", [1]), ("wRecv", [0])],
    "predicate": "s.pages.get(0).ver == 2 and s.w.get(0).on",
}
WITNESSES["closed-window-ignores-push"] = {
    "profile": "base", "actions": OPEN + [("extWriteD", [0, 3, 1]), ("observe", [0]),
        ("wClose", [0]), ("wRecv", [0])],
    "predicate": "s.w.get(0) == NOW and s.pages.get(0).held",
}
WITNESSES["typing-while-submit-is-sent"] = {
    "profile": "base", "actions": OPEN + [("wEdit", [0, 2]), ("wSend", [0]),
        ("wEdit", [0, 3]), ("deliverUp", [1]), ("wRecv", [0]), ("wSend", [0]),
        ("deliverUp", [1])],
    "predicate": "s.pages.get(0).buf == 3 and not(s.pages.get(0).conflict)",
}
WITNESSES["discard-queued-behind-save"] = {
    "profile": "base", "actions": OPEN + EDIT + [("wRecv", [0]), ("flush", [0]),
        ("wDiscard", [0]), ("check", []), ("rename", []), ("dirSync", [1]),
        ("deliverUp", [1])],
    "predicate": "s.pages.get(0).ver == 3 and not(s.pages.get(0).risk)",
}
WITNESSES["drafted-switch-blocked-by-save"] = {
    "profile": "base", "actions": OPEN + EDIT + [("wRecv", [0]), ("flush", [0]),
        ("switchReq", []), ("draftSync", [0]), ("check", []), ("rename", []),
        ("dirSync", [1])],
    "predicate": "s.disk.get(0) == 2 and s.drafts.get(0).bytes == 2",
}
WITNESSES["stale-discard-version-push"] = {
    "profile": "base", "actions": OPEN + [("wDiscard", [0]), ("windowCrash", []),
        ("wOpen", [0]), ("deliverUp", [1])],
    "predicate": "s.mb.get(0).on and s.mb.get(0).ack == -1 and s.pages.get(0).ver == 2",
}
WITNESSES["conflict-observation-only-push"] = {
    "profile": "base", "actions": OPEN + EDIT + [("extWriteD", [0, 3, 1]),
        ("observe", [0]), ("extWriteD", [0, -1, 1]), ("observe", [0])],
    "predicate": "s.pages.get(0).conflict and s.mb.get(0).obs == ABSENT",
}
WITNESSES["conflict-only-guard-mismatch-push"] = {
    "profile": "base", "actions": [("wOpen", [1]), ("deliverUp", [1]), ("wRecv", [1]),
        ("wEdit", [1, 3]), ("wSend", [1]), ("wEdit", [1, 1]), ("deliverUp", [0]),
        ("flush", [1]), ("wRecv", [1]), ("wSend", [1]), ("wEdit", [1, 2]),
        ("switchReq", []), ("check", []), ("rename", []), ("wEdit", [1, 3]),
        ("dirSync", [1]), ("deliverUp", [1]), ("wRecv", [1]), ("flush", [1]),
        ("wDiscard", [1]), ("check", []), ("rename", []), ("wEdit", [1, 1]),
        ("dirSync", [0]), ("flush", [1]), ("draftSync", [1]), ("extWriteD", [1, 2, 0]),
        ("check", [])],
    "predicate": "s.pages.get(1).conflict and s.mb.get(1).on and s.mb.get(1).conf",
}
WITNESSES["fault-C"] = {
    "profile": "base", "mutant": "MC",
    "actions": OPEN + EDIT + [("flush", [0]), ("extWriteD", [0, 3, 1]),
        ("check", []), ("rename", [])],
    "predicate": 'g.bad.contains("C-overwrote-external")',
}
WITNESSES["fault-B"] = {
    "profile": "base", "mutant": "MLO",
    "actions": OPEN + EDIT + [("extWriteD", [0, 3, 1]), ("observe", [0])],
    "predicate": 'g.bad.contains("B-unsaved-replaced")',
}
WITNESSES["fault-Bprime"] = {
    "profile": "base", "mutant": "MWA",
    "actions": OPEN + [("wEdit", [0, 2]), ("extWriteD", [0, 3, 1]),
        ("observe", [0]), ("wRecv", [0])],
    "predicate": 'g.bad.contains("B\'-window-replaced")',
}
WITNESSES["fault-G"] = {
    "profile": "base", "mutant": "MRB",
    "actions": OPEN + [("wEdit", [0, 2]), ("extWriteD", [0, 3, 1]), ("observe", [0]),
        ("wSend", [0]), ("deliverUp", [1]), ("draftSync", [0]), ("crash", []),
        ("launch", []), ("flush", [0]), ("check", []), ("rename", [])],
    "predicate": 'g.bad.contains("G-overwrote-unseen")',
}
WITNESSES["fault-submit-not-taken"] = {
    "profile": "base", "mutant": "MIS", "actions": OPEN + EDIT,
    "predicate": 'g.bad.contains("B\'-submit-not-taken")',
}
WITNESSES["fault-accepted"] = {
    "profile": "base", "mutant": "MDQ", "actions": OPEN + [("wEdit", [0, 2]),
        ("wSend", [0]), ("windowCrash", []), ("extWriteD", [0, 3, 1])],
    "predicate": "not(accepted)",
}
WITNESSES["fault-loss"] = {
    "profile": "base", "mutant": "MOD", "actions": OPEN + [("wOpen", [1]),
        ("deliverUp", [1]), ("wRecv", [1]), ("wOp", [0, 3, 3]), ("deliverUp", [1]),
        ("crash", []), ("extWriteD", [0, 2, 1])],
    "predicate": "not(noLoss)",
}
WITNESSES["fault-weak-unsaved-power"] = {
    "profile": "weak", "mutant": "MOD", "actions": OPEN + [("wOpen", [1]),
        ("deliverUp", [1]), ("wRecv", [1]), ("wOp", [0, 3, 3]), ("deliverUp", [1]),
        ("power", [0, 0])],
    "predicate": "g.promise.get(1).on and not(g.promise.get(1).saved) and not(noLoss)",
}
WITNESSES["newer-write-custody-with-identical-disk-and-draft"] = {
    "profile": "base", "actions": OPEN + [("wEdit", [0, 3]), ("wSend", [0]),
        ("deliverUp", [1]), ("wRecv", [0]), ("switchReq", []), ("draftSync", [0]),
        ("flush", [0]), ("check", []), ("rename", []), ("dirSync", [1]),
        ("wEdit", [0, 2]), ("wSend", [0]), ("deliverUp", [1]), ("wRecv", [0]),
        ("flush", [0]), ("check", []), ("rename", []), ("dirSync", [1]),
        ("wEdit", [0, 3]), ("wSend", [0]), ("deliverUp", [1]), ("flush", [0]),
        ("check", []), ("rename", []), ("crash", []), ("extWriteD", [1, 1, 1])],
    "predicate": "noLoss and s.disk.get(0) == 3 and s.drafts.get(0).bytes == 3 and g.promise.get(0).bytes == 2",
}
WITNESSES["fault-MSM-stale-move"] = {
    "profile": "base", "mutant": "MSM", "actions": OPEN + [("wOpen", [1]),
        ("deliverUp", [1]), ("wRecv", [1]), ("extWriteD", [1, 1, 1]), ("observe", [1]),
        ("wOp", [0, 3, 3]), ("deliverUp", [1])],
    "predicate": "s.pages.get(1).base == UNKNOWN and s.pages.get(0).buf == 3",
}
WITNESSES["fault-MIS-resolve"] = {
    "profile": "base", "mutant": "MIS", "actions": OPEN + [("wOpen", [1]),
        ("deliverUp", [1]), ("wRecv", [1]), ("wOp", [0, 3, 3]), ("deliverUp", [1]),
        ("wRecv", [0]), ("wRecv", [1]), ("extWriteD", [1, 3, 1]), ("observe", [1]),
        ("wRecv", [1]), ("wResolve", [1, 1]), ("deliverUp", [1])],
    "predicate": "s.pages.get(1).buf == 2 and s.pages.get(1).conflict",
}
WITNESSES["fault-MCL-risky-close"] = {
    "profile": "base", "mutant": "MCL", "actions": OPEN + EDIT + [("wRecv", [0]),
        ("switchReq", []), ("wClose", [0]), ("deliverUp", [1])],
    "predicate": "s.pages.get(0).held and s.pages.get(0).risk",
}
WITNESSES["fault-MRE-risky-uncertain-save"] = {
    "profile": "base", "mutant": "MRE", "actions": OPEN + EDIT + [("switchReq", []),
        ("flush", [0]), ("check", []), ("rename", []), ("dirSync", [0])],
    "predicate": "not(s.pages.get(0).risk) and clean(s.pages.get(0))",
}
for src in [0, 1]:
    WITNESSES[f"fault-MDO-unheld-move-{src}"] = {
        "profile": "base", "mutant": "MDO", "actions": OPEN + [("wOpen", [1]),
            ("deliverUp", [1]), ("wRecv", [1])] + EDIT + [("wRecv", [0]),
            ("wOp", [src, 3, 3]), ("deliverUp", [1])],
        "predicate": 'g.bad.contains("request-to-unheld")',
    }
WITNESSES["diagnostic-external-counter"] = {
    "profile": "base", "cycle": [("extWriteD", [0, 2, 1]), ("extWriteD", [0, 1, 1])],
    "steps": 1002, "actions": [], "predicate": "g.ext.get(0) > EMAX", "guards": False,
}
WITNESSES["diagnostic-version-counter"] = {
    "profile": "base", "cycle": OPEN + [("wClose", [0]), ("deliverUp", [1])],
    "steps": 5005, "actions": [], "predicate": "g.vc > VMAX", "guards": False,
}
WITNESSES["diagnostic-queue-counter"] = {
    "profile": "base", "cycle": [("wOpen", [0]), ("windowCrash", [])],
    "steps": 2002, "actions": [], "predicate": "s.up.length() > QMAX", "guards": False,
}

# Literal outer action guards from the frozen model, before each commit.
# This oracle also exercises rejected choices: successful traces alone cannot
# detect a weakened guard. Domain selection remains exactly PAGES/TEXTS/BOOLS.
GUARDS = {
    "wOpen": "s.alive and not(s.w.get(p).on) and not(s.w.get(p).sent)",
    "wEdit": "s.alive and s.w.get(p).on and v != s.w.get(p).text",
    "wSend": "s.alive and s.w.get(p).on and s.w.get(p).pend and not(s.w.get(p).sent)",
    "wResolve": "s.alive and s.w.get(p).on and s.w.get(p).conf and not(s.w.get(p).sent)",
    "wDiscard": "s.alive and s.w.get(p).on and not(s.w.get(p).sent)",
    "wOp": "s.alive and s.w.get(p).on and s.w.get(1-p).on and not(s.w.get(p).pend) and not(s.w.get(p).sent) and not(s.w.get(1-p).pend) and not(s.w.get(1-p).sent) and ds != s.w.get(p).text and dd != s.w.get(1-p).text",
    "wClose": "s.alive and s.w.get(p).on and not(s.w.get(p).pend) and not(s.w.get(p).sent)",
    "wRecv": "s.alive and s.mb.get(p).on",
    "deliverUp": "s.alive and s.up.length() > 0 and (if (s.up.length() > 0) not(s.job.on and s.job.p == s.up.head().p) and (s.up.head().kind != \"op\" or not(s.job.on and s.job.p == s.up.head().q)) else false)",
    "observe": "s.alive and s.pages.get(p).held and not(s.job.on and s.job.p == p) and s.disk.get(p) != s.pages.get(p).obs",
    "flush": "s.alive and not(s.job.on) and s.pages.get(p).held and not(s.pages.get(p).conflict) and s.pages.get(p).buf != ABSENT and dirty(s.pages.get(p))",
    "check": "s.alive and s.job.on and s.job.phase == 1",
    "rename": "s.alive and s.job.on and s.job.phase == 2",
    "dirSync": "s.alive and s.job.on and s.job.phase == 3",
    "saveFail": "s.alive and s.job.on and s.job.phase <= 2",
    "draftSync": "s.alive and s.drafts.get(p) != draftEntry(s.pages.get(p))",
    "switchReq": "s.alive and PAGES.exists(p => { val pg = s.pages.get(p) pg.held and dirty(pg) and not(pg.risk) })",
    "switchFin": "s.alive and not(s.job.on) and s.up.length() == 0 and PAGES.forall(p => { val pg = s.pages.get(p) val a = s.w.get(p) not(a.pend) and not(a.sent) and (not(pg.held) or clean(pg) or pg.risk) and s.drafts.get(p) == draftEntry(pg) })",
    "extWriteD": "v != s.disk.get(p) and (not(s.job.on and s.job.phase == 2 and s.job.p == p) or RACES.contains(\"R1\"))",
    "crash": "s.alive",
    "windowCrash": "s.alive",
    "power": "true",
    "launch": "not(s.alive)",
}


def guard_oracle():
    import itertools
    entries = []
    for name, guard in GUARDS.items():
        domains = ([range(2), [1, 2, 3]] if name in ["wEdit", "wResolve"] else
                   [range(2), [1, 2, 3], [1, 2, 3]] if name == "wOp" else
                   [range(2), [-1, 1, 2, 3], [0, 1]] if name == "extWriteD" else
                   [[0, 1], [0, 1]] if name == "power" else
                   [[0, 1]] if name in ["deliverUp", "dirSync"] else
                   [range(2)] if name in ["wOpen", "wSend", "wDiscard", "wClose", "wRecv", "observe", "flush", "draftSync"] else [])
        for args in itertools.product(*domains):
            params = (["p", "v"] if name in ["wEdit", "wResolve", "extWriteD"] else
                      ["p", "ds", "dd"] if name == "wOp" else ["p"])
            expression = guard
            for p, v in zip(params, args):
                # Substitute free p only: switchReq/switchFin have bound p.
                expression = re.sub(rf"(?<![.\w]){p}\b", str(v), expression)
            entries.append(f'{{ name: "{name}", args: List({", ".join(map(str, args))}), allowed: {expression} }}')
    body = "[" + ",\n".join(entries) + "]"
    return "  type GuardChoice = { name: str, args: List[int], allowed: bool }\n  def guardOracle(sys: Sys): List[GuardChoice] = " + re.sub(r"\bs\b", "sys", body) + "\n"


def find(model_dir, quint, name, spec, reuse=False):
    work = ROOT / "scratch/s2/witnesses"
    work.mkdir(parents=True, exist_ok=True)
    model = (model_dir / "storage-s2.qnt").read_text()
    assert hashlib.sha256(model.encode()).hexdigest() == SHA
    model = re.sub(r"pure val RACES: Set\[str\] = .*",
                   f"pure val RACES: Set[str] = {PROFILES[spec['profile']]}", model, count=1)
    mutant = spec.get("mutant", "none")
    model = re.sub(r'pure val MUTANT: str = ".*"', f'pure val MUTANT: str = "{mutant}"', model, count=1)
    actions = spec.get("cycle", spec["actions"])
    steps = spec.get("steps", len(actions))
    branches = []
    for i, (action, args) in enumerate(actions):
        boolean = {"deliverUp": {0}, "dirSync": {0}, "extWriteD": {2}, "power": {0, 1}}
        params = [str(bool(v)).lower() if j in boolean.get(action, set()) else str(v)
                  for j, v in enumerate(args)]
        call = action + ("(" + ", ".join(params) + ")" if args else "")
        stage_guard = f"stage % {len(actions)} == {i}" if "cycle" in spec else f"stage == {i}"
        oracle = "guardOracle(s)" if spec.get("guards", True) else "[]"
        branches.append(f'''all {{ {stage_guard}, {call}, stage' = stage + 1,
          traceAction' = {{ name: "{action}", args: List({", ".join(map(str, args))}) }},
          actionEnabled' = {oracle}, predicateValues' = predicateOracle }}''')
    driver = '''
  var stage: int
  var traceAction: { name: str, args: List[int] }
  var actionEnabled: List[GuardChoice]
  var predicateValues: { loss: bool, accepted: bool, within: bool }
  val predicateOracle = { loss: noLoss, accepted: accepted, within: within }
  action witnessInit = all { init, stage' = 0, traceAction' = { name: "init", args: List() }, actionEnabled' = [], predicateValues' = { loss: true, accepted: true, within: true } }
  action witnessStep = any {
''' + ",\n".join(branches) + f'''
  }}
  val witness = not(stage >= {steps} and ({spec['predicate']}))
'''
    path = work / f"{name}.qnt"
    text = model.replace("  // @@SCENARIOS@@", guard_oracle() + driver)
    itf = work / f"{name}.itf.json"
    log = work / f"{name}.log"
    if not (reuse and path.exists() and path.read_text() == text and itf.exists()
            and log.exists() and "Invariant violated" in log.read_text()):
        path.write_text(text)
        result = command([quint, "run", path, "--backend", "typescript", "--init", "witnessInit",
            "--step", "witnessStep", "--invariant", "witness", "--max-samples", "1",
            "--max-steps", steps + 1, "--seed", "20261008", "--verbosity", "1",
            "--out-itf", itf], log, {**os.environ, "TMPDIR": str(work)})
        assert result.returncode == 1 and "Invariant violated" in result.stdout + result.stderr and itf.exists(), result.stdout + result.stderr
    states = json.loads(itf.read_text())["states"]
    assert len(states) == steps + 1
    return {"name": name, "profile": spec["profile"], "mutant": mutant, "predicate": spec["predicate"],
            "states": [{"action": decode(s["traceAction"]),
                        "state": {"s": decode(s["s"]), "g": decode(s["g"])},
                        "enabled": decode(states[i + 1]["actionEnabled"]) if i + 1 < len(states) and mutant == "none" else [],
                        "predicates": decode(states[i + 1]["predicateValues"]) if i + 1 < len(states) else None}
                       for i, s in enumerate(states)]}


def delta(before, after, path=()):
    if before == after:
        return []
    if isinstance(before, dict) and isinstance(after, dict) and before.keys() == after.keys():
        return [change for k in after for change in delta(before[k], after[k], path + (k,))]
    if isinstance(before, list) and isinstance(after, list) and len(before) <= len(after):
        changes = [change for i, v in enumerate(before) for change in delta(v, after[i], path + (i,))]
        if len(before) < len(after):
            changes.append({"path": path, "append": after[len(before):]})
        return changes
    return [{"path": path, "value": after}]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("model_dir", type=Path)
    ap.add_argument("--quint", type=Path)
    ap.add_argument("--reuse", action="store_true", help="reuse only identical scratch models with successful witness logs")
    args = ap.parse_args()
    quint = args.quint or args.model_dir.parent / "model/tools/node_modules/.bin/quint"
    traces = [find(args.model_dir, quint, name, spec, args.reuse) for name, spec in WITNESSES.items()]
    path = ROOT / "crates/tine-store/tests/fixtures/s2/witnesses.json"
    choices = [{"name": c["name"], "args": c["args"]} for c in traces[0]["states"][0]["enabled"]]
    paths, actions = [], []
    for trace in traces:
        for entry in trace["states"]:
            entry["enabled"] = [c["allowed"] for c in entry["enabled"]]
        # Lossless state deltas keep long diagnostic threshold witnesses modest.
        if len(trace["states"]) > 100:
            previous = trace["states"][0]["state"]
            for i, entry in enumerate(trace["states"]):
                action = entry.pop("action")
                if action not in actions:
                    actions.append(action)
                entry["a"] = actions.index(action)
                entry.pop("enabled")
                predicates = entry.pop("predicates")
                if predicates:
                    entry["within"] = predicates["within"]
                if i:
                    current = entry.pop("state")
                    changes = []
                    for change in delta(previous, current):
                        p = list(change["path"])
                        if p not in paths:
                            paths.append(p)
                        encoded = [paths.index(p), change.get("value", change.get("append"))]
                        if "append" in change:
                            encoded.append(True)
                        changes.append(encoded)
                    entry["delta"] = changes
                    previous = current
    path.write_text(json.dumps({"model_sha256": SHA, "choices": choices,
                               "paths": paths, "actions": actions, "traces": traces}, separators=(",", ":")) + "\n")
    print(f"{len(traces)} witness traces, {sum(len(t['states']) for t in traces)} states, {path.stat().st_size} bytes")


if __name__ == "__main__":
    main()
