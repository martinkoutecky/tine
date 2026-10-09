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
    "observe": "s.alive and s.pages.get(p).held and not(s.job.on and s.job.p == p) and table(s.pages.get(p), s.disk.get(p), g.vc + 1) != s.pages.get(p)",
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


GUARDS.update({
    "load": "s.alive and not(s.pages.get(p).held)",
    "wOpTo": "s.alive and p != q and s.w.get(p).on and s.w.get(q).on and not(s.w.get(p).pend) and not(s.w.get(p).sent) and not(s.w.get(q).pend) and not(s.w.get(q).sent) and ds != s.w.get(p).text and dd != s.w.get(q).text",
    "opDelete": "s.alive and opHeld(Set(p)) and opFree(Set(p)) and opClean(s, p) and opCur(s, p) != ABSENT",
    "flushDel": "s.alive and not(s.job.on) and s.pages.get(p).held and not(s.pages.get(p).conflict) and s.pages.get(p).buf == ABSENT and dirty(s.pages.get(p))",
})
# Short s3 witnesses exercise page-operation custody, historical exemptions,
# trash durability, rollback and each operation guard in every role.
WITNESSES["observe-last-read-after-own-save"] = {
    "profile": "base", "actions": SAVED + [("extWriteD", [0, 1, 1]), ("observe", [0])],
    "predicate": "s.pages.get(0).buf == 1 and s.pages.get(0).obs == 1",
}
WITNESSES["cli-rename-trash-and-launch"] = {
    "profile": "base", "actions": [("load",[0]),("load",[1]),("load",[2]),("opRenameRaw", [0,2,0,1,0,3,3,3]),
        ("flushDel",[0]),("check",[]),("rename",[]),("dirSync",[1]),("draftSync",[0]),
        ("crash",[]),("launch",[]),("observe",[1]),("observe",[2]),
        ("flush",[2]),("check",[]),("rename",[]),("dirSync",[1]),
        ("flush",[1]),("check",[]),("rename",[]),("dirSync",[1]),("powerKBits",[0,0,0])],
    "predicate": "s.disk.get(2) == 1 and s.disk.get(1) == 3 and s.trash.get(0) == Set(1)",
}
WITNESSES["cli-delete-r1-unflushed-trash"] = {
    "profile": "R1", "actions": [("load",[0]),("opDelete",[0]),("flushDel",[0]),("check",[]),
        ("extWriteD",[0,3,0]),("rename",[]),("dirSync",[0]),("powerKBits",[1,0,0])],
    "predicate": "s.trash.get(0) == Set(3) and g.removed.get(0) == Set(3)",
}
WITNESSES["delete-rollback-trash"] = {
    "profile": "base", "actions": [("load",[1]),("opDelete",[1]),("flushDel",[1]),("check",[]),
        ("rename",[]),("powerKBits",[0,0,0]),("launch",[]),("observe",[1])],
    "predicate": "s.disk.get(1) == 2 and s.trash.get(1) == Set() and s.pages.get(1).risk",
}
WITNESSES["delete-keep-third-path"] = {
    "profile": "base", "actions": [("extWriteD",[2,3,0]),("load",[2]),("opDelete",[2]),
        ("flushDel",[2]),("check",[]),("rename",[]),("powerKBits",[0,0,1])],
    "predicate": "s.disk.get(2) == ABSENT and s.trash.get(2) == Set(3)",
}
WITNESSES["rename-refs-only-no-source"] = {
    "profile": "base", "actions": [("load",[2]),("load",[1]),("opRenameRaw",[2,0,0,1,0,1,3,1])],
    "predicate": "s.pages.get(1).buf == 3 and s.pages.get(0) == NOPAGE and g.vc == 5",
}
WITNESSES["rename-clean-held-absent-destination"] = {
    "profile": "base", "actions": [("wOpen",[2]),("deliverUp",[1]),("wRecv",[2]),
        ("load",[0]),("load",[1]),
        ("opRenameRaw",[0,2,0,1,0,3,3,3])],
    "predicate": "s.pages.get(2).buf == 1 and s.pages.get(2).base == ABSENT",
}
WITNESSES["rename-clobbered-held-target-fault"] = {
    "profile": "base", "mutant": "MRN3", "actions": OPEN + [("load",[1]),("opRenameRaw",[1,0,0,0,0,3,3,3])],
    "predicate": "g.bad.contains(\"rename-clobbered-target\")",
}
WITNESSES["rename-dirty-referrer-fault"] = {
    "profile": "base", "mutant": "MRN2", "actions": OPEN + EDIT + [("load",[1]),("load",[2]),("opRenameRaw",[1,2,1,0,0,3,3,3])],
    "predicate": "g.bad.contains(\"D4-unclean-page-changed\")",
}
WITNESSES["delete-stale-window"] = {
    "profile": "base", "actions": OPEN + [("wEdit",[0,2]),("opDelete",[0]),
        ("wSend",[0]),("deliverUp",[1])], "predicate": "s.pages.get(0).conflict and guarantee",
}
WITNESSES["one-path-delete-and-restore"] = {
    "profile": "base", "pages": 1,
    "actions": [("load",[0]),("opDelete",[0]),("crash",[]),("launch",[]),("flushDel",[0]),
        ("check",[]),("rename",[]),("dirSync",[1])],
    "predicate": "s.disk.get(0) == ABSENT and s.trash.get(0) == Set(1)", "guards": False,
}
WITNESSES["two-path-rename-and-restore"] = {
    "profile": "base", "pages": 2,
    "actions": [("extWriteD",[1,-1,1]),("load",[0]),("load",[1]),("opRenameRaw",[0,1,0,0,0,3,3,3]),
        ("crash",[]),("launch",[])],
    "predicate": "s.pages.get(0).buf == ABSENT and s.pages.get(1).buf == 1 and g.vc == 6", "guards": False,
}
WITNESSES["five-path-sparse-launch-versions"] = {
    "profile": "base", "pages": 5,
    "actions": [("load",[0]),("load",[1]),("load",[4]),("opRenameRaw",[0,4,0,1,0,3,3,3]),("crash",[]),("launch",[])],
    "predicate": "s.pages.get(0).ver == 9 and s.pages.get(1).ver == 10 and s.pages.get(4).ver == 11 and g.vc == 11", "guards": False,
}
WITNESSES["fault-trash-disk-fallback"] = {
    "profile": "base", "mutant": "MDT",
    "actions": [("load",[0]),("opDelete",[0]),("flushDel",[0]),("check",[]),("rename",[]),
        ("dirSync",[1]),("extWriteD",[0,1,1])],
    "predicate": "trashed and s.trash.get(0) == Set() and g.removed.get(0) == Set(1)",
}
WITNESSES["ended-deletion-promise-watermark-has-old-bytes"] = {
    "profile": "base",
    "actions": [("load",[0]),("opDelete",[0]),("flushDel",[0]),("check",[]),("rename",[]),("dirSync",[1])]
        + OPEN + EDIT + [("wRecv",[0]),("flush",[0]),("check",[]),("rename",[]),("dirSync",[0]),
            ("wDiscard",[0]),("deliverUp",[1]),("flush",[0])],
    "predicate": "s.job.on and not(g.promise.get(0).on) and g.promise.get(0).saved and g.promise.get(0).ver == s.job.ver and g.promise.get(0).bytes != s.job.bytes",
}
WITNESSES["delete-conflict-blocks-flush-del"] = {
    "profile": "base", "actions": [("load",[0]),("opDelete",[0]),("extWriteD",[0,3,1]),("observe",[0])],
    "predicate": "s.pages.get(0).buf == ABSENT and s.pages.get(0).conflict and s.disk.get(0) == 3",
}
for label, k0, k1, expected in [("keeps-second",0,1,3),("reverts-second",0,0,2),("keeps-first-only",1,0,2)]:
    WITNESSES["power-alias-" + label] = {
        "profile": "base", "actions": [("extWriteD",[1,3,0]),("power",[k0,k1])],
        "predicate": f"s.disk.get(1) == {expected} and s.stable.get(1) == {expected}",
    }

def guard_oracle():
    import itertools
    entries = []
    declarations = []
    for name, guard in GUARDS.items():
        domains = ([range(3), [1, 2, 3]] if name in ["wEdit", "wResolve"] else
                   [range(2), [1, 2, 3], [1, 2, 3]] if name == "wOp" else
                   [range(3), range(3), [1,2,3], [1,2,3]] if name == "wOpTo" else
                   [range(3), [-1, 1, 2, 3], [0, 1]] if name == "extWriteD" else
                   [[0, 1], [0, 1]] if name == "power" else
                   [[0, 1]] if name in ["deliverUp", "dirSync"] else
                   [range(3)] if name in ["wOpen", "wSend", "wDiscard", "wClose", "wRecv", "observe", "load", "flush", "draftSync", "opDelete", "flushDel"] else [])
        for args in itertools.product(*domains):
            params = (["p", "v"] if name in ["wEdit", "wResolve", "extWriteD"] else
                      ["p", "ds", "dd"] if name == "wOp" else ["p","q","ds","dd"] if name == "wOpTo" else ["p"])
            free = params[:len(args)] if name not in ["deliverUp", "dirSync", "power"] else []
            declaration = f"  def guard{name}(sys: Sys" + "".join(f", {p}: int" for p in free) + "): bool = " + re.sub(r"\bs\b", "sys", guard) + "\n"
            if declaration not in declarations: declarations.append(declaration)
            expression = f"guard{name}(s" + "".join(f", {v}" for v in args[:len(free)]) + ")"
            entries.append(f'{{ name: "{name}", args: List({", ".join(map(str, args))}), allowed: {expression} }}')
    for src, dst, r0, r1, r2 in itertools.product(range(3), range(3), [0,1], [0,1], [0,1]):
        refs = f"Set({', '.join(str(p) for p, on in enumerate([r0,r1,r2]) if on)})"
        full = f"opCur(s, {src}) != ABSENT"
        named = f"if ({full}) Set({src}, {dst}) else Set()"
        all3 = f"({named}).union({refs})"
        guard = f"guardOpRename(s,{src},{dst},{r0},{r1},{r2})"
        entries.append(f'{{ name: "opRenameRaw", args: List({src},{dst},{r0},{r1},{r2},3,3,3), allowed: {guard} }}')
    for k0,k1,k2 in itertools.product([0,1], repeat=3):
        entries.append(f'{{ name: "powerKBits", args: List({k0},{k1},{k2}), allowed: true }}')
    body = "[" + ",\n".join(entries) + "]"
    helpers = """
  def guardOpRename(sys: Sys, src: int, dst: int, r0: int, r1: int, r2: int): bool = {
    val refs = PAGES.filter(p => (p == 0 and r0 == 1) or (p == 1 and r1 == 1) or (p == 2 and r2 == 1))
    val full = opCur(sys, src) != ABSENT
    val named = if (full) Set(src, dst) else Set()
    val all3 = named.union(refs)
    sys.alive and src != dst and opHeld(all3.union(Set(src))) and opFree(all3) and refs.forall(r => r != src and r != dst)
      and (not(full) or opClean(sys, src))
      and (not(full) or (opCur(sys, dst) == ABSENT and opClean(sys, dst)))
      and refs.forall(r => opClean(sys, r)) and all3 != Set()
  }
  action powerKBits(k0: bool, k1: bool, k2: bool): bool = powerK(PAGES.filter(p => (p == 0 and k0) or (p == 1 and k1) or (p == 2 and k2)))
  action opRenameRaw(src: int, dst: int, r0: bool, r1: bool, r2: bool, t0: int, t1: int, t2: int): bool =
    opRename(src, dst, PAGES.filter(p => (p == 0 and r0) or (p == 1 and r1) or (p == 2 and r2)), Map(0 -> t0, 1 -> t1, 2 -> t2))
"""
    return helpers + "".join(declarations) + "  type GuardChoice = { name: str, args: List[int], allowed: bool }\n  def guardOracle(sys: Sys): List[GuardChoice] = " + re.sub(r"\bs\b", "sys", body) + "\n"


def find(model_dir, quint, name, spec, reuse=False):
    work = ROOT / "scratch/s3/witnesses"
    work.mkdir(parents=True, exist_ok=True)
    model = (model_dir / "storage-s3.qnt").read_text()
    assert hashlib.sha256(model.encode()).hexdigest() == SHA
    pages = spec.get("pages", 3)
    if pages != 3:
        model = model.replace("pure val PAGES = Set(0, 1, 2)", "pure val PAGES = Set(" + ", ".join(map(str, range(pages))) + ")")
    model = re.sub(r"pure val RACES: Set\[str\] = .*",
                   f"pure val RACES: Set[str] = {PROFILES[spec['profile']]}", model, count=1)
    mutant = spec.get("mutant", "none")
    model = re.sub(r'pure val MUTANT: str = ".*"', f'pure val MUTANT: str = "{mutant}"', model, count=1)
    actions = spec.get("cycle", spec["actions"])
    steps = spec.get("steps", len(actions))
    branches = []
    for i, (action, args) in enumerate(actions):
        boolean = {"deliverUp": {0}, "dirSync": {0}, "extWriteD": {2}, "power": {0, 1}, "powerKBits": {0,1,2}, "opRenameRaw": {2,3,4}}
        params = [str(bool(v)).lower() if j in boolean.get(action, set()) else str(v)
                  for j, v in enumerate(args)]
        call = action + ("(" + ", ".join(params) + ")" if args else "")
        stage_guard = f"stage < {steps} and stage % {len(actions)} == {i}" if "cycle" in spec else f"stage == {i}"
        oracle = "guardOracle(s)" if spec.get("guards", True) else "[]"
        branches.append(f'''all {{ {stage_guard}, {call}, stage' = stage + 1,
          traceAction' = {{ name: "{action}", args: List({", ".join(map(str, args))}) }} }}''')
    driver = '''
  var stage: int
  var traceAction: { name: str, args: List[int] }
  var actionEnabled: List[GuardChoice]
  var predicateValues: { loss: bool, accepted: bool, within: bool, trash: bool, guarantee: bool }
  val predicateOracle = { loss: noLoss, accepted: accepted, within: within, trash: trashed, guarantee: guarantee }
  action witnessInit = all { init, stage' = 0, traceAction' = { name: "init", args: List() }, actionEnabled' = [], predicateValues' = { loss: true, accepted: true, within: true, trash: true, guarantee: true } }
  action witnessSchedule = any {
''' + ",\n".join(branches) + f''',
    all {{ stage == {steps}, s' = s, g' = g, stage' = stage + 1, traceAction' = {{ name: "capture", args: List() }} }}
  }}
  action witnessStep = all {{ witnessSchedule, actionEnabled' = {oracle}, predicateValues' = predicateOracle }}
  val witness = not(stage >= {steps + 1} and ({spec['predicate']}))
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
            "--max-steps", steps + 2, "--seed", "20261009", "--verbosity", "1",
            "--out-itf", itf], log, {**os.environ, "TMPDIR": str(work)})
        assert result.returncode == 1 and "Invariant violated" in result.stdout + result.stderr and itf.exists(), result.stdout + result.stderr
    states = json.loads(itf.read_text())["states"]
    assert len(states) == steps + 2
    assert decode(states[-1]["s"]) == decode(states[-2]["s"]) and decode(states[-1]["g"]) == decode(states[-2]["g"])
    print(f"witness {name}: {len(states) - 1} states", flush=True)
    return {"name": name, "profile": spec["profile"], "pages": pages, "mutant": mutant, "predicate": spec["predicate"],
            "states": [{"action": decode(s["traceAction"]),
                        "state": {"s": decode(s["s"]), "g": decode(s["g"])},
                        "enabled": decode(states[i + 1]["actionEnabled"]) if i + 1 < len(states) and mutant == "none" else [],
                        "predicates": decode(states[i + 1]["predicateValues"]) if i + 1 < len(states) else None}
                       for i, s in enumerate(states[:-1])]}


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


def pack(traces, min_states=100):
    paths, actions = [], []
    for trace in traces:
        for entry in trace["states"]:
            if "enabled" in entry:
                entry["enabled"] = [c["allowed"] for c in entry["enabled"]]
        # Lossless state deltas keep long diagnostic threshold witnesses modest.
        if len(trace["states"]) > min_states:
            previous = trace["states"][0]["state"]
            for i, entry in enumerate(trace["states"]):
                action = entry.pop("action")
                if action not in actions:
                    actions.append(action)
                entry["a"] = actions.index(action)
                entry.pop("enabled", None)
                predicates = entry.pop("predicates", None)
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
    return paths, actions


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
    paths, actions = pack(traces)
    path.write_text(json.dumps({"model_sha256": SHA, "choices": choices,
                               "paths": paths, "actions": actions, "traces": traces}, separators=(",", ":")) + "\n")
    print(f"{len(traces)} witness traces, {sum(len(t['states']) for t in traces)} states, {path.stat().st_size} bytes")


if __name__ == "__main__":
    main()
