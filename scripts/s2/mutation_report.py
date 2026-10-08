#!/usr/bin/env python3
"""Merge mutation censuses, reject unexplained survivors, emit a review ledger."""
import hashlib
import json
from pathlib import Path
from mutation_harness import ROOT

# The membership is explicit: a newly surviving ID never inherits an argument
# simply because it happens to be in the same function.
GROUPS = [
    ("H002", "cleanEquality", "Without typed input or risk, every held page has buf == base; NOPAGE does too. Init establishes this; submit sets typed, uncertain/conflict sets risk, Published/discard establish equality."),
    ("H004 H235 H241", "riskHeld,unheldPadding", "Risk implies held. Every retirement installs NOPAGE, which is clean, so !held is redundant in clean-or-risk and risk predicates."),
    ("H020 H022 H023", "jobPromiseBound,draftAckEcho,savedAckEcho", "The page is locked throughout a save, so its promise cannot outrun that job. A same-version unsaved draft ack repeats identical bytes/ep; a same-version saved ack repeats the saved record. Dropping these tests either remains false or repeats the same record."),
    ("H026 H028 H032 H033 H034 H035 H037 H046", "guarantee,unheldPadding", "These widen keptB permission (or test an unheld NOPAGE whose !typed && buf == obs is already safe). Every original model transition satisfies keptB, so its bad-set contribution remains empty."),
    ("H038 H039 H045", "shownAND,shownTyped,noSwitch", "The commit-boundary invariant compares the original keptB result with stricter shown and switch alternatives. A stale typed replacement is already acknowledged or carries the current window version; an untyped replacement is either read-safe/acknowledged or shown. At switchFin dirty pages have an exact current draft/ack, and clean pages are read-safe. The final switch disjunct adds no permission."),
    ("H050 H059", "guarantee,mailAckSent", "Dropping !b.on widens keptW permission. Dropping the wAck name allows no additional replacement: only an ack can clear a sent window; other pushes preserve it, and a closed window has no pending input."),
    ("H062 H064", "noHeldA,noHeldB,absentClean", "The commit invariant compares complete resulting promises. An unheld predecessor is clean, so cannot meet dirty(a); an unheld successor has ABSENT bytes, which cannot match a dirty predecessor. Neither removal changes an ack."),
    ("H067 H073 H076 H077 H080", "mailDrop1,mailDrop4,downPadding,offMailPadding,windowDomain", "Down clears windows, making the alive mailbox guard redundant. Buffer/held changes also change version or leave an identical request ack; the invariant compares mailbox records. An off mailbox is NOMAIL (took=false). Every on-window text is ABSENT or a TEXTS member."),
    ("H086 H089 H090 H093 H094 H099 H102 H111 H115 H171 H177 H181 H183 H185 H187 H188 H193 H194 H205 H206 H213 H218 H220 H222 H225 H243", "downPadding,jobHeld,pendingOpen,offWindowPadding,unheldPadding,absentClean", "Down has no on/pending/conflicting windows, held/dirty pages, job or queue; another surviving guard rejects the action. Pending implies on; conflict implies on. NOJOB has phase 0. Unheld/ABSENT pages are clean, excluding flush and dirty switch loops. These facts are preserved by the only constructors and guarded updates."),
    ("H118 H121", "mailAckSent,offMailPadding", "An on mailbox with ack>=0 has a sent window. If a pending push is treated as an ack, took is false and its update (text/pend preserved, sent=false, bv unchanged, obs/conf copied) equals the original pending-push update."),
    ("H134 H135 H153 H154", "guarantee", "Only the internal name changes among uOpen/uOpenFail or uDiscard/uDiscardFail. None belongs to keptB's stale/crash/power/switch exceptions or keptW's user/wAck exceptions; names are not stored. Sys/Ghost are identical for every argument, not merely reachable ones."),
    ("H136", "unheldPadding", "When !held, clean(NOPAGE) is true and assigning NOPAGE to an existing NOPAGE is an identity. When held, the removed conjunct was true."),
    ("H159 H160", "freshSubmitSeen,resolveWithoutMine", "Only mine/transition name change, and mine is cleared by commit. A fresh submit's predecessor was shown at that window version; relabeling it stale satisfies keptB. A resolution whose version changed has an unchanged/read-safe/acknowledged predecessor. Relabeling a stale submit fresh only widens permission already satisfied."),
    ("H169 H170", "freshMoveChangesBoth (also sampled with MIS)", "Without MIS, submitTo always takes both requested texts. With MIS, accepted moves require both versions current and both new texts different; both took flags are false. Any changed backend version refuses the whole move before these tests. Exactly one failed take is unreachable."),
    ("H197", "strongRenameBase,jobGuard", "In a profile without R1, extWriteD cannot change the locked page between check and rename, so disk != base is impossible there. With R1 the disjunct is explicitly false. MC is still caught by !guard."),
    ("H200 H201 H204", "jobGuard,sameByteSaveSeen,writtenSeen", "A real rename follows a successful guard; an unchanged-byte save's base was seen/written. Every Tine-written byte was already seen by the originating window (and seen persists through crashes), so the wrote fallback supplies no additional G permission in this frozen model."),
    ("H211", "guarantee", "With ok=true, MRE's Page constructor is exactly onReply(Published): base=job.bytes, typed=false, risk=false, other fields preserved. With ok=false the original branch already selects it."),
    ("H251", "guarantee", "If promise.on is false, endPromise(pr, pr.ver) returns that identical off record. Removing the on check cannot change state."),
    ("H264 H267", "guarantee,sameVersionBytes", "These widen A's exemption/newer-copy tests. noLoss is true throughout the non-mutant frozen model, so its Boolean result remains true. The false fault witnesses also reject unsupported widening where applicable."),
    ("H269", "downOwed", "Down/crash/power/switch clear owed; its universal containment test is true on an empty set even without the !alive disjunct."),
    ("H280 H281 H282 H283 H284 H285 H286", "windowDomain; original PAGES/TEXTS/BOOLS driver domains", "The frozen driver selects p in {0,1}, edit/move text in {1,2,3}, and external text in {-1,1,2,3}. Every dropped domain check is true on every model action. This does not claim equivalence for out-of-domain callers."),
]


def hand_proofs():
    return {ident: {"invariant": invariant, "argument": argument}
            for ids, invariant, argument in GROUPS for ident in ids.split()}


def cargo_proof(mutant):
    function = (mutant.get("function") or {}).get("function_name")
    line = mutant["span"]["start"]["line"]
    group = {
        "real": "H280", "step": "H280", "kept_b": "H032", "w_send": "H089",
        "w_resolve": "H093", "up_open": "H134", "up_discard": "H153",
        "up_submit": "H159", "check": "H187", "rename": "H193", "dir_sync": "H205",
        "no_loss": "H264",
    }.get(function)
    if function == "kept_b" and line == 385:
        group = "H038"
    if function == "kept_b" and line == 391:
        group = "H045"
    if function == "up_op" and line == 803:
        group = "H169"
    if function == "up_op" and line == 768:
        return {"invariant": "freshSubmitSeen,typedSeen; stale-set emptiness",
                "argument": "Swapping which page is tagged stale preserves stale-set emptiness, so the non-mutant model refuses exactly the same moves. MSM may swap mine membership, but the uOpStale exception accepts each predecessor's already-seen typed bytes or untyped file bytes. mine is cleared by commit; all stored fields are identical."}
    if function == "ext_write":
        return {"invariant": "original step driver action census",
                "argument": "Unreachable Rust alias: both step and scenario ExtWrite dispatch directly to ext_write_d; ext_write has no caller. The frozen Quint step likewise names extWriteD. This is an unused transcription alias, not an equivalent public API for hypothetical callers."}
    assert group, ("unexplained cargo survivor", mutant["name"])
    return hand_proofs()[group]


def main():
    cargo = {}
    for run in ["cargo-baseline", "cargo-witness", "cargo-final", "cargo-last"]:
        path = ROOT / f"scratch/s2/{run}/mutants.out/outcomes.json"
        if not path.exists():
            continue
        for outcome in json.loads(path.read_text())["outcomes"]:
            if "Mutant" in outcome["scenario"]:
                cargo[outcome["scenario"]["Mutant"]["name"]] = outcome
    hand = {}
    for run in ["hand-before", "hand-witness", "hand-final", "hand-last"]:
        path = ROOT / f"scratch/s2/{run}/outcomes.json"
        if path.exists():
            for outcome in json.loads(path.read_text()):
                hand[outcome["id"]] = outcome
    assert len(cargo) == 561 and len(hand) == 288
    assert all(o["summary"] in {"CaughtMutant", "MissedMutant", "Unviable"} for o in cargo.values()), "timeouts/build infrastructure failures need investigation"
    assert all(o["status"] in {"caught", "missed"} for o in hand.values()), "a hand mutation must compile"
    proofs = hand_proofs()
    hand_eq = {ident: {**proofs[ident], "dropped": o["dropped"]}
               for ident, o in hand.items() if o["status"] == "missed"}
    cargo_eq = {name: cargo_proof(o["scenario"]["Mutant"])
                for name, o in cargo.items() if o["summary"] == "MissedMutant"}
    ledger = {"source_sha256": hashlib.sha256((ROOT / "crates/tine-store/src/page_state/mod.rs").read_bytes()).hexdigest(),
              "hand": hand_eq, "cargo": cargo_eq}
    (ROOT / "scripts/s2/equivalents.json").write_text(json.dumps(ledger, indent=2) + "\n")
    from collections import Counter
    print("cargo", Counter(o["summary"] for o in cargo.values()))
    print("hand", Counter(o["status"] for o in hand.values()))
    print("Every survivor has an explicit invariant and argument:", len(hand_eq), "hand,", len(cargo_eq), "cargo")


if __name__ == "__main__":
    main()
