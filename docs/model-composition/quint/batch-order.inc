  pure def receivers(moves: Set[(int, int)]): Set[int] = moves.map(m => m._1)
  pure def sources(moves: Set[(int, int)]): Set[int] = moves.map(m => m._2)

  pure def gates1(gt: Gate, k: int): bool = {
    val dsts = receivers(gt.moves)
    val srcs = sources(gt.moves)
    gt.open.contains(k) and not(dsts.contains(k)) and not(MUTANT == "MRF" and not(srcs.contains(k)))
      and not(MUTANT == "early-source" and gt.moves.exists(m => m._2 == k and gt.open.contains(m._1)))
      and (gt.open.intersect(dsts) != Set()
        or (srcs.contains(k) and gt.open.exclude(srcs) != Set() and MUTANT != "MRD"))
  }
  pure def gated(sys: Sys, k: int): bool = sys.gates.foldl(false, (acc, gt) => acc or gates1(gt, k))
  pure def owned(gt: Gate): Set[int] = PAGES.filter(k => gt.vers.get(k) != NONE)
  pure def touches(sys: Sys, ks: Set[int]): bool = sys.gates.foldl(false, (acc, gt) => acc or owned(gt).intersect(ks) != Set())
  pure def partner(sys: Sys, k: int): int =
    sys.gates.foldl(NONE, (acc, gt) => if (acc != NONE or not(gt.open.contains(k))) acc
      else gt.moves.fold(NONE, (src, m) => if (m._1 == k) m._2 else src))
  pure def discharge(sys: Sys, q: int, ver: int): Sys =
    { ...sys, gates: sys.gates.foldl(List(), (acc, gt) => {
      val ng = if (gt.vers.get(q) != NONE and ver >= gt.vers.get(q)) { ...gt, open: gt.open.exclude(Set(q)) } else gt
      if (ng.open == Set()) acc else acc.append(ng)
    }) }
  pure def discarded(sys: Sys, k: int): Sys =
    { ...sys, gates: sys.gates.foldl(List(), (acc, gt) => {
      val cancel = gt.moves.filter(m => m._1 == k and gt.open.contains(k))
      val moves = gt.moves.exclude(cancel)
      val pending = gt.open.exclude(Set(k).union(sources(cancel)))
      if (moves == Set() or pending == Set()) acc else acc.append({ ...gt, moves: moves, open: pending })
    }) }

  pure def makeGate(moves: Set[(int, int)], ps: Set[int], v: int -> int): Gate =
    { moves: moves, vers: PAGES.mapBy(p => if (ps.contains(p)) v.get(p) else NONE), open: ps }
  pure def makeGates(moves: Set[(int, int)], ps: Set[int], v: int -> int): List[Gate] =
    if (MUTANT == "per-pair") {
      val shared = ps.exclude(sources(moves).union(receivers(moves)))
      moves.fold(List(), (acc, m) => acc.append(makeGate(Set(m), shared.union(Set(m._1, m._2)), v)))
    } else List(makeGate(moves, ps, v))

  // Requirements are recorded from admission, independently of the scheduler's gates.
  pure def requirements(moves: Set[(int, int)], ps: Set[int], v: int -> int, id: int): Set[(int, int, int, int)] =
    ps.exclude(receivers(moves)).fold(Set(), (edges, p) => edges.union(
      ps.filter(q => p != q and (receivers(moves).contains(q)
        or (sources(moves).contains(p) and not(sources(moves).contains(q)))))
        .map(q => (p, q, v.get(q), id))))
  pure def oblige(moves: Set[(int, int)], v: int -> int, id: int): Set[MoveObligation] =
    moves.map(m => { dst: m._1, src: m._2, version: v.get(m._1), id: id, pending: true })
  pure def oblPublished(obl: Set[(int, int, int, int)], q: int, ver: int): Set[(int, int, int, int)] =
    obl.filter(o => not(o._2 == q and ver >= o._3))
  pure def movesPublished(moves: Set[MoveObligation], obl: Set[(int, int, int, int)], q: int, ver: int): Set[MoveObligation] =
    moves.filter(m => obl.exists(o => o._4 == m.id))
      .map(m => if (m.dst == q and ver >= m.version) { ...m, pending: false } else m)
  pure def oblDiscarded(obl: Set[(int, int, int, int)], moves: Set[MoveObligation], k: int): Set[(int, int, int, int)] = {
    val cancel = moves.filter(m => m.dst == k and m.pending)
    obl.filter(o => o._1 != k and o._2 != k and not(cancel.exists(m => m.id == o._4
      and (o._1 == m.src or o._2 == m.src or not(moves.exclude(cancel).exists(other => other.id == m.id))))))
  }

  val gateOwnership = s.gates.foldl({ valid: true, keys: Set() }, (seen, gt) => {
    val slots = owned(gt)
    val endpoints = receivers(gt.moves).union(sources(gt.moves))
    { valid: seen.valid and seen.keys.intersect(slots) == Set()
      and gt.open != Set() and gt.open.subseteq(slots) and gt.moves != Set()
      and endpoints.subseteq(slots) and endpoints.size() == 2 * gt.moves.size(), keys: seen.keys.union(slots) }
  }).valid
  val requirementsBlock = g.obl.forall(o => gated(s, o._1))
