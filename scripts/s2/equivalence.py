#!/usr/bin/env python3
"""Check structural reachability facts in scratch copies of the frozen model.

Sampling supports the accompanying inductive arguments; it is not a proof.
"""
import argparse
import hashlib
import os
from pathlib import Path
import re
from generate import SHA, PROFILES, command, traced_model

ROOT = Path(__file__).resolve().parents[2]
FACTS = {
    "riskHeld": "PAGES.forall(p => s.pages.get(p).risk implies s.pages.get(p).held)",
    "unheldPadding": "PAGES.forall(p => not(s.pages.get(p).held) implies s.pages.get(p) == NOPAGE)",
    "cleanEquality": "PAGES.forall(p => { val a = s.pages.get(p) (not(a.typed) and not(a.risk)) implies a.buf == a.base })",
    "downPadding": "not(s.alive) implies (not(s.job.on) and s.up == List() and PAGES.forall(p => s.w.get(p) == NOW and s.mb.get(p) == NOMAIL))",
    "jobHeld": "s.job.on implies (s.alive and s.pages.get(s.job.p).held)",
    "windowDomain": "PAGES.forall(p => s.w.get(p).on implies (s.w.get(p).text == ABSENT or TEXTS.contains(s.w.get(p).text)))",
    "pendingOpen": "PAGES.forall(p => s.w.get(p).pend implies s.w.get(p).on)",
    "jobGuard": "(s.job.on and s.job.phase == 2) implies g.guard",
    "versionBound": "PAGES.forall(p => s.pages.get(p).ver <= g.vc)",
    "ghostMineEmpty": "g.mine == Set()",
    "mailAckSent": "PAGES.forall(p => (s.mb.get(p).on and s.mb.get(p).ack >= 0) implies s.w.get(p).sent)",
    "operationBytesWritten": "PAGES.forall(p => s.pages.get(p).typed implies (g.seen.get(p).contains(s.pages.get(p).buf) or g.wrote.get(p).exists(w => w._1 == s.pages.get(p).buf)))",
    "strongRenameBase": "(s.job.on and s.job.phase == 2 and not(RACES.contains(\"R1\"))) implies s.disk.get(s.job.p) == s.job.base",
    "sameByteSaveSeen": "(s.job.on and s.job.bytes == s.job.base) implies (g.seen.get(s.job.p).contains(s.job.base) or g.wrote.get(s.job.p).exists(w => w._1 == s.job.base))",
    "freshMoveChangesBoth": "if (s.alive and s.up.length() > 0) { val m = s.up.head() val a = s.pages.get(m.p) val b = s.pages.get(m.q) (m.kind == \"op\" and a.held and b.held and m.bv == a.ver and m.bv2 == b.ver) implies (m.t != a.buf and m.t2 != b.buf) } else true",
    "jobPromiseBound": "s.job.on implies g.promise.get(s.job.p).ver <= s.job.ver",
    "draftAckEcho": "PAGES.forall(p => { val a = s.pages.get(p) val pr = g.promise.get(p) (a.held and a.risk and a.ver == pr.ver and pr.on and not(pr.saved)) implies (pr.bytes == a.buf and pr.ep == 0) })",
    "savedAckEcho": "s.job.on implies { val j = s.job val pr = g.promise.get(j.p) (j.ver == pr.ver and pr.on and pr.saved) implies (pr.bytes == j.bytes and (j.phase != 3 or pr.ep == j.ep)) }",
    "freshSubmitSeen": "if (s.alive and s.up.length() > 0) { val m = s.up.head() val a = s.pages.get(m.p) (m.kind == \"submit\" and m.ro == NONE and a.held and m.bv == a.ver) implies (g.seen.get(m.p).contains(a.buf) or not(a.typed)) } else true",
    "resolveWithoutMine": "if (s.alive and s.up.length() > 0) { val m = s.up.head() val a = s.pages.get(m.p) val pr = g.promise.get(m.p) (m.kind == \"submit\" and m.ro != NONE and a.held and m.bv != a.ver) implies (a.buf == m.t or (not(a.typed) and a.buf == a.obs) or (pr.ver >= a.ver and pr.bytes == a.buf)) } else true",
    "offMailPadding": "PAGES.forall(p => not(s.mb.get(p).on) implies s.mb.get(p) == NOMAIL)",
    "offWindowPadding": "PAGES.forall(p => { val w = s.w.get(p) not(w.on) implies (w.text == ABSENT and w.bv == -1 and not(w.pend) and w.obs == ABSENT and not(w.conf)) })",
    "sameVersionBytes": "PAGES.forall(p => { val pr = g.promise.get(p) pr.on implies g.wrote.get(p).forall(w => w._2 == pr.ver implies w._1 == pr.bytes) })",
    "cleanNoConflict": "PAGES.forall(p => (s.pages.get(p).held and clean(s.pages.get(p))) implies not(s.pages.get(p).conflict))",
    "downOwed": "not(s.alive) implies g.owed == Set()",
    "saveBaseSeenOrRead": "(s.job.on and s.job.phase == 2 and g.guard and s.job.bytes != s.job.base) implies (g.seen.get(s.job.p).contains(s.job.base) or g.opRead.get(s.job.p).contains(s.job.base))",
    "jobDirty": "s.job.on implies (s.pages.get(s.job.p).held and dirty(s.pages.get(s.job.p)))",
    "removedInTrash": "PAGES.forall(p => g.removed.get(p).subseteq(s.trash.get(p)))",
}


def transition_facts(text):
    """Compare suspect guard alternatives at the original commit boundary.

    Equality of predicates is insufficient when an existing ack makes a push
    idempotent. Compare the mailbox records themselves in that case.
    """
    kb_start = text.index("  def keptB(")
    kb_end = text.index("  // B'.", kb_start)
    body = text[kb_start:kb_end]
    alternatives = {
        "shownAND": ('g.seen.get(p).contains(a.buf) or not(a.typed)', 'g.seen.get(p).contains(a.buf) and not(a.typed)'),
        "shownTyped": ('g.seen.get(p).contains(a.buf) or not(a.typed)', 'g.seen.get(p).contains(a.buf) or a.typed'),
        "noSwitch": ('or (name == "switchFin" and nsys.drafts.get(p).bytes == a.buf)', ''),
    }
    clones = []
    for label, (old, new) in alternatives.items():
        assert old in body
        clones.append(body.replace("def keptB(", f"def keptB{label}(").replace(old, new))
    text = text.replace("  // @@SCENARIOS@@", "\n".join(clones) + "  // @@SCENARIOS@@")
    beginning = text.index("  action commit(")
    end = text.index("  // ================================================================ init", beginning)
    commit = text[beginning:end]
    guards = {
        "noHeldA": "not(ng.mine.contains(p)) and dirty(a) and b.held and clean(b) and b.buf == a.buf",
        "noHeldB": "not(ng.mine.contains(p)) and a.held and dirty(a) and clean(b) and b.buf == a.buf",
        "noEqualBuf": "not(ng.mine.contains(p)) and a.held and dirty(a) and b.held and clean(b)",
    }
    # All three guards are redundant at the promise boundary if the resulting
    # ack is unchanged. This is stronger than merely keeping guarantee true.
    checks = []
    for label in alternatives:
        expression = f"PAGES.forall(p => keptB(p, nsys, np, ng.mine, name) == keptB{label}(p, nsys, np, ng.mine, name))"
        checks.append(f'(if ({expression}) Set() else Set("{label}"))')
    original = "not(ng.mine.contains(p)) and a.held and dirty(a) and b.held and clean(b) and b.buf == a.buf"
    for name, guard in guards.items():
        expression = f"PAGES.forall(p => {{ val a = s.pages.get(p) val b = nsys.pages.get(p) val pr = ng.promise.get(p) (if ({original}) ack(pr, b.buf, b.ver, false, 0) else pr) == (if ({guard}) ack(pr, b.buf, b.ver, false, 0) else pr) }})"
        checks.append(f'(if ({expression}) Set() else Set("{name}"))')
    # Mailbox trigger terms can be redundant because another term fires, or
    # because the request already installed exactly the mailbox being pushed.
    terms = ["a.ver != b.ver", "a.buf != b.buf", "a.obs != b.obs", "a.conflict != b.conflict", "a.held != b.held"]
    mail = '{ on: true, held: b.held, ver: b.ver, text: b.buf, obs: b.obs, conf: b.conflict, ack: if (m.on) m.ack else -1, took: m.on and m.took }'
    for i in [1, 4]:
        broad = " or ".join(terms)
        narrow = " or ".join(t for j, t in enumerate(terms) if i != j)
        expression = f"PAGES.forall(p => {{ val a = s.pages.get(p) val b = nsys.pages.get(p) val m = nsys.mb.get(p) val w = nsys.w.get(p) (if (nsys.alive and (w.on or w.sent) and ({broad})) {mail} else m) == (if (nsys.alive and (w.on or w.sent) and ({narrow})) {mail} else m) }})"
        checks.append(f'(if ({expression}) Set() else Set("mailDrop{i}"))')
    injected = "      reachabilityChecks' = " + ".union(".join(checks) + ")" * (len(checks) - 1) + ",\n"
    commit = commit.replace("      s' = { ...nsys, mb: mb },", injected + "      s' = { ...nsys, mb: mb },")
    text = text[:beginning] + commit + text[end:]
    text = text.replace("  action init = all {", "  var reachabilityChecks: Set[str]\n  action init = all { reachabilityChecks' = Set(),")
    return text


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("model_dir", type=Path)
    ap.add_argument("--samples", type=int, default=4096)
    ap.add_argument("--steps", type=int, default=80)
    ap.add_argument("--output", type=Path, default=ROOT / "scratch/s3/equivalence")
    ap.add_argument("--only", help="comma-separated structural facts, excluding commit checks and guarantee")
    ap.add_argument("--mutant", default="none")
    ap.add_argument("--profiles", help="comma-separated profiles for a correction of invalidated evidence")
    args = ap.parse_args()
    model = (args.model_dir / "storage-s3.qnt").read_text()
    assert hashlib.sha256(model.encode()).hexdigest() == SHA
    quint = args.model_dir.parent / "model/tools/node_modules/.bin/quint"
    work = args.output.resolve()
    work.mkdir(parents=True, exist_ok=True)
    for i, (profile, races) in enumerate(PROFILES.items()):
        if args.profiles and profile not in args.profiles.split(","):
            continue
        text = re.sub(r"pure val RACES: Set\[str\] = .*", f"pure val RACES: Set[str] = {races}", model, count=1)
        text = re.sub(r'pure val MUTANT: str = ".*"', f'pure val MUTANT: str = "{args.mutant}"', text, count=1)
        text = traced_model(text)
        if not args.only:
            text = transition_facts(text)
        selected = args.only.split(",") if args.only else list(FACTS)
        declarations = "\n".join(f"  val {name} = {FACTS[name]}" for name in selected)
        prefix = "" if args.only else "guarantee and reachabilityChecks == Set() and "
        declarations += "\n  val equivalentReachability = " + prefix + " and ".join(selected)
        path = work / f"{profile}.qnt"
        path.write_text(text.replace("  // @@SCENARIOS@@", declarations))
        result = command([quint, "run", path, "--backend", "typescript", "--init", "traceInit",
                          "--step", "traceStep", "--invariant", "equivalentReachability",
                          "--max-samples", args.samples, "--max-steps", args.steps,
                          "--seed", 20261009 + i * 7919, "--verbosity", "1",
                          "--out-itf", work / f"{profile}-failure.itf.json"],
                         work / f"{profile}.log", {**os.environ, "TMPDIR": str(work)})
        assert result.returncode == 0, result.stdout + result.stderr
        print(profile, args.samples, args.steps, "all reachability invariants hold", flush=True)


if __name__ == "__main__":
    main()
