const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");

const storage = path.resolve(__dirname, "../../storage-model/quint");
const output = path.resolve(__dirname, "../../../target/compose-stage6");
const read = (dir, name) => fs.readFileSync(path.join(dir, name), "utf8");

function model(mutant, wrappers) {
  let source = read(storage, "storage-s36.qnt");
  const replace = (old, next) => {
    assert.equal(source.split(old).length, 2, old);
    source = source.replace(old, () => next);
  };
  const replaceSpan = (start, end, next) => {
    const first = source.indexOf(start);
    const last = source.indexOf(end, first + start.length);
    assert(first >= 0 && last > first, start);
    replace(source.slice(first, last), next);
  };

  replace('pure val RACES: Set[str] = Set("crash", "power")', 'pure val RACES: Set[str] = Set("crash", "power", "R1")');
  replace('pure val MUTANT: str = "none"', `pure val MUTANT: str = "${mutant}"`);
  replace("pure val PAGES = Set(0, 1, 2)", "pure val PAGES = Set(0, 1, 2, 3, 4)");
  replace("type Gate = { dst: int, src: int, vers: int -> int, open: Set[int] }",
    `type Gate = { moves: Set[(int, int)], vers: int -> int, open: Set[int] }
  type MoveObligation = { dst: int, src: int, version: int, id: int, pending: bool }`);
  replace("    obl: Set[(int, int, int, int)],", `    obl: Set[(int, int, int, int)],
    orderMoves: Set[MoveObligation],
    discardOk: bool,`);
  replace("guard: false, obl: Set(), renamed:", "guard: false, obl: Set(), orderMoves: Set(), renamed:");
  replace("obl: Set(), renamed: Set(), orderOk:", "obl: Set(), orderMoves: Set(), discardOk: true, renamed: Set(), orderOk:");
  replaceSpan("  // s3.2: whether gt holds back", "  pure def unrename", read(__dirname, "batch-order.inc") + "\n");

  // Keep both old admission entry points using the same gate constructor and edge oracle.
  replaceSpan("    val gate = { dst: dst, src: src, vers:", "    all {", `    val moves = Set((dst, src))
    val gate = makeGate(moves, all3, v)
    val obl = requirements(moves, all3, v, g.vc + 1)
`);
  replace("obl: if (full) g.obl.union(obl) else g.obl,", `obl: if (full) g.obl.union(obl) else g.obl,
          orderMoves: if (full) g.orderMoves.union(oblige(moves, v, g.vc + 1)) else g.orderMoves,`);
  replace(`  action opApply(eff: int -> int, plan: int -> int, dst: int, src: int): bool = {`,
    `  action opApply(eff: int -> int, plan: int -> int, dst: int, src: int): bool = all {
    (dst == NONE) == (src == NONE),
    opBatch(eff, plan, if (dst == NONE) List() else List((dst, src))),
  }
  action opBatch(eff: int -> int, plan: int -> int, bindings: List[(int, int)]): bool = {
    val moves = bindings.foldl(Set(), (acc, m) => acc.union(Set(m)))`);
  replace("val mv = dst != NONE", "val mv = moves != Set()");
  replaceSpan("    val others = ps.exclude(Set(dst, src))", "    val stale = ",
    `    val obl = requirements(moves, ps, v, g.vc + 1)
    val endpoints = sources(moves).union(receivers(moves))
`);
  replaceSpan("      if (mv) all { dst != src", "      commit(",
    `      endpoints.size() == 2 * bindings.length(), endpoints.subseteq(ps),
      moves.forall(m => eff.get(m._2) == ABSENT and eff.get(m._1) != ABSENT and plan.get(m._2) != ABSENT),
`);
  replace("val claimedCreates = ps.filter(p => plan.get(p) == ABSENT", "val claimedCreates = ps.filter(p => plan.keys().contains(p) and plan.get(p) == ABSENT");
  replace("all { s.alive, ps != Set(), opFree(ps)", "all { s.alive, ps != Set(), ps.subseteq(PAGES), opFree(ps)");
  replace("if (mv and MUTANT != \"MAM\") s.gates.append(gate) else s.gates",
    `if (mv and MUTANT != "MAM") s.gates.concat(makeGates(moves, ps, v)) else s.gates`);
  replace("renamed: unrename(g.renamed, ps).union(if (mv and s.disk.get(src) != ABSENT) Set((src, dst)) else Set()),",
    `orderMoves: g.orderMoves.union(oblige(moves, v, g.vc + 1)),
          renamed: unrename(g.renamed, ps).union(moves.filter(m => s.disk.get(m._2) != ABSENT).map(m => (m._2, m._1))),`);
  replace("    // s3.2: Published (ok, on a weak filesystem too)",
    "    val edges = if (ok) oblPublished(g.obl, p, j.ver) else g.obl\n    // s3.2: Published (ok, on a weak filesystem too)");
  replace("obl: if (ok) oblPublished(g.obl, p, j.ver) else g.obl,", `obl: edges,
              orderMoves: if (ok) movesPublished(g.orderMoves, edges, p, j.ver) else g.orderMoves,`);

  replace("  action flush(p: int): bool = {", `  action flush(p: int): bool = flushWrite(p, true)
  action flushWrite(p: int, checkClaimAtStart: bool): bool = {`);
  replace("val claimedCreate = pg.base == ABSENT and s.claimed.contains(p)",
    "val claimedCreate = checkClaimAtStart and pg.base == ABSENT and s.claimed.contains(p)");

  // The stock Discard reads a singleton partner. Keep that behavior and inject the second restore only for its mutant.
  replaceSpan("    val src = if (partner(sys, m.p)", "    if (not(pg.held)) commit", `    val src = partner(sys, m.p)
    val other = sys.gates.foldl(Set(), (acc, gt) =>
      if (gt.open.contains(m.p) and receivers(gt.moves).contains(m.p)) sources(gt.moves) else acc)
    val candidates = if (src == NONE) Set() else if (MUTANT == "discard-both") other else Set(src)
    val reverting = candidates.filter(p =>
      (sys.pages.get(p).held or MUTANT == "MRH") and sys.pages.get(p).buf == ABSENT)
    val after = reverting.fold({ pages: sys.pages.set(m.p, nw), promise: g.promise.set(m.p, npr), ver: v1 }, (acc, p) => {
      val sp = sys.pages.get(p)
      val sd = s.disk.get(p)
      val v2 = acc.ver + 1
      val sprm = g.promise.get(p)
      val shold = sp.risk and (sd == sp.buf or sd == s.drafts.get(p).bytes) and MUTANT != "MDE"
      val slive = sprm.on and not(sprm.saved and g.ext.get(p) > sprm.ep)
      { pages: acc.pages.set(p, { ...sp, buf: sd, base: sd, ver: v2, obs: sd, conflict: false, risk: shold, typed: false }),
        promise: acc.promise.set(p, if (slive and sprm.bytes == sd) sprm else endPromise(sprm, v2)), ver: v2 }
    })
    val restored = PAGES.exclude(Set(m.p)).filter(p => after.pages.get(p) != sys.pages.get(p))
    val expected = PAGES.filter(p => p == src and sys.pages.get(p).held and sys.pages.get(p).buf == ABSENT)
    val edges = oblDiscarded(g.obl, g.orderMoves, m.p)
`);
  replaceSpan("    else commit(ackMail(discarded({ ...sys, pages: pages2 }", "  // B'(2):", `    else commit(ackMail(discarded({ ...sys, pages: after.pages }, m.p), m.p, ok, v1, false),
      { ...g, vc: after.ver, promise: after.promise, mine: reverting.union(Set(m.p)),
        discardOk: g.discardOk and restored == expected, obl: edges,
        orderMoves: g.orderMoves.filter(move => not(move.dst == m.p and move.pending) and edges.exists(o => o._4 == move.id)) }, "uDiscard") }

`);

  // A selected power image becomes the next stable baseline for the removal ghost as well.
  replace("delDurable: PAGES.mapBy(p => g.delDurable.get(p).intersect(nr.get(p)))", "delDurable: nr");
  const helpers = read(storage, "scenarios-s36.inc").split("  // ---- maximality");
  assert.equal(helpers.length, 2);
  replace("  // @@SCENARIOS@@", helpers[0] + read(__dirname, "batch-scenarios.inc") + wrappers);
  return source;
}

module.exports = { model, output, storage };
