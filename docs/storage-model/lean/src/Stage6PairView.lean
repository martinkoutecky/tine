import Stage6GateSafety
import PairActions11

namespace Stage6
open Storage21

private def atEndpoint (m : Move P) (p : Slot P) : Bool := p == m.src || p == m.dst

def pairGate (gt : Gate P) (m : Move P) : Storage32.Gate P :=
  ⟨m.dst, m.src, fun p => if atEndpoint m p then (gt.vers p).getD noneBytes else noneBytes,
    [m.src, m.dst].filter gt.pending.contains⟩

/-- Endpoint-only view for applying the singleton pair lemmas. The batch
gate and its complete footprint continue to govern admission and saves. -/
def State.pairView (x : State P) : Storage35.State P :=
  { x.legacyView with s := { x.legacyView.s with
    gates := x.gates.flatMap fun gt => gt.moves.map (pairGate gt) } }

theorem pairGate_mem {x : State P} {g : Storage32.Gate P} :
    g ∈ x.pairView.s.gates ↔ ∃ gt ∈ x.gates, ∃ m ∈ gt.moves, pairGate gt m = g := by
  simp [State.pairView, State.legacyView]

theorem pairGate_pending {gt : Gate P} {m : Move P} {p : Slot P} :
    p ∈ (pairGate gt m).pending ↔ p ∈ gt.pending ∧ (p = m.src ∨ p = m.dst) := by
  simp [pairGate, and_comm]

theorem pairGate_vers {gt : Gate P} {m : Move P} {p : Slot P}
    (ep : p = m.src ∨ p = m.dst) :
    (pairGate gt m).vers p = (gt.vers p).getD noneBytes := by
  simp [pairGate, atEndpoint, ep]

private theorem pairGate_owns {gt : Gate P} {m : Move P} {p : Slot P}
    (h : (pairGate gt m).vers p ≠ noneBytes) :
    (p = m.src ∨ p = m.dst) ∧ (gt.vers p).isSome = true := by
  by_cases ep : p = m.src ∨ p = m.dst
  · rw [pairGate_vers ep] at h
    cases v : gt.vers p <;> simp_all
  · simp [pairGate, atEndpoint, ep] at h

theorem endpoint_version {gt : Gate P} (shape : gt.Shape)
    {m : Move P} (mm : m ∈ gt.moves) {p : Slot P} (ep : p = m.src ∨ p = m.dst) :
    ∃ v, gt.vers p = some v ∧ v ≠ noneBytes := by
  have member : p ∈ endpoints gt.moves := by
    rcases ep with rfl | rfl
    · exact (endpoint_mem mm).2
    · exact (endpoint_mem mm).1
  obtain ⟨v, ver⟩ := Option.isSome_iff_exists.mp (shape.2.1 p member)
  have nonneg := shape.2.2.2 p v ver
  refine ⟨v, ver, ?_⟩
  unfold noneBytes
  omega

theorem pairView_owns (x : State P) (own : GateOwnership x.gates) :
    Storage32.GateOwnership11 x.pairView.s.gates := by
  have valid (gt : Gate P) (gm : gt ∈ x.gates) (m : Move P) (mm : m ∈ gt.moves)
      (p : Slot P) (ep : p = m.src ∨ p = m.dst) : (pairGate gt m).vers p ≠ noneBytes := by
    obtain ⟨_, ver, ne⟩ := endpoint_version (own.1 gt gm) mm ep
    simpa [pairGate_vers ep, ver] using ne
  constructor
  · intro g gm
    obtain ⟨gt, gtmem, m, mm, rfl⟩ := pairGate_mem.mp gm
    exact ⟨endpoint_distinct (own.1 gt gtmem).1 mm,
      valid gt gtmem m mm _ (Or.inl rfl), valid gt gtmem m mm _ (Or.inr rfl),
      fun p pm => valid gt gtmem m mm p (pairGate_pending.mp pm).2⟩
  · intro g gm h hm p gp hp
    obtain ⟨gt, gtmem, m, mm, rfl⟩ := pairGate_mem.mp gm
    obtain ⟨ht, htmem, n, nm, rfl⟩ := pairGate_mem.mp hm
    obtain ⟨ep, gv⟩ := pairGate_owns gp
    obtain ⟨eq, hv⟩ := pairGate_owns hp
    have same := own.2 gt gtmem ht htmem p gv hv
    subst ht
    have me := endpoint_unique (own.1 gt gtmem).1 mm nm ep.symm eq.symm
    subst n
    exact ⟨rfl, rfl⟩

theorem pairView_partner (x : State P) (p : Slot P) :
    Storage32.partner x.pairView.s.to32 p = Storage32.partner x.legacyView.s.to32 p := by
  let f := fun (acc : Option (Slot P)) (g : Storage32.Gate P) =>
    if acc.isNone && g.dst == p && g.pending.contains p then some g.src else acc
  let full := fun (gt : Gate P) (m : Move P) =>
    (⟨m.dst, m.src, fun q => (gt.vers q).getD noneBytes, gt.pending⟩ : Storage32.Gate P)
  have single (gt : Gate P) (m : Move P) (a : Option (Slot P)) :
      f a (pairGate gt m) = f a (full gt m) := by
    by_cases dst : m.dst = p
    · subst p
      simp [f, pairGate, full]
    · simp [f, pairGate, full, dst]
  have moveFold (gt : Gate P) (ms : List (Move P)) (a : Option (Slot P)) :
      (ms.map (pairGate gt)).foldl f a = (ms.map (full gt)).foldl f a := by
    induction ms generalizing a with
    | nil => rfl
    | cons m ms ih => simp only [List.map_cons, List.foldl_cons, single, ih]
  change (x.gates.flatMap fun gt => gt.moves.map (pairGate gt)).foldl f none =
    (x.gates.flatMap fun gt => gt.moves.map (full gt)).foldl f none
  generalize (none : Option (Slot P)) = a
  induction x.gates generalizing a with
  | nil => rfl
  | cons gt gs ih => simp only [List.flatMap_cons, List.foldl_append, moveFold, ih]

private theorem pairView_ungated (x : State P) (c : Config P) (p : Slot P)
    (normal : c.mutant = "none") (own : GateOwnership x.gates)
    (free : (x.gates.any fun gt => gt.gated p) = false) :
    Storage32.gated c x.pairView.s.to32 p = false := by
  apply List.any_eq_false.mpr
  intro g gm gated
  obtain ⟨gt, gtmem, m, mm, rfl⟩ := pairGate_mem.mp gm
  have shape := own.1 gt gtmem
  have data : p ∈ (pairGate gt m).pending ∧ p ≠ m.dst := by
    have parts := gated
    simp only [Storage32.gates1, Bool.and_eq_true] at parts
    exact ⟨List.contains_iff_mem.mp parts.1.1.1, bne_iff_ne.mp parts.1.1.2⟩
  have ep := pairGate_pending.mp data.1
  have src : p = m.src := ep.2.resolve_right data.2
  have dm : m.dst ∈ gt.pending := by
    by_cases nd : m.dst ∈ gt.pending
    · exact nd
    · have sm : m.src ∈ gt.pending := src ▸ ep.1
      have ne := endpoint_distinct shape.1 mm
      simp [Storage32.gates1, normal, src, pairGate, sm, nd, Ne.symm ne] at gated
  have recvFalse : (gt.moves.any fun n => p == n.dst) = false := by
    apply List.any_eq_false.mpr
    intro n nm hit
    exact source_not_receiver shape ⟨m, mm, src⟩ ⟨n, nm, beq_iff_eq.mp hit⟩
  have waits : (gt.pending.any fun q => gt.moves.any fun n => q == n.dst) = true :=
    List.any_eq_true.mpr ⟨m.dst, dm, List.any_eq_true.mpr ⟨m, mm, by simp⟩⟩
  exact (List.any_eq_false.mp free gt gtmem) (by simp [Gate.gated, ep.1, recvFalse, waits])

private theorem pairView_untouched (x : State P) (ps : List (Slot P)) (free : touches x.gates ps = false) :
    Storage32.touches x.pairView.s.to32 ps = false := by
  apply List.any_eq_false.mpr
  intro g gm hit
  obtain ⟨p, pm, ver⟩ := List.any_eq_true.mp hit
  obtain ⟨gt, gtmem, m, mm, rfl⟩ := pairGate_mem.mp gm
  have some := (pairGate_owns (bne_iff_ne.mp ver)).2
  simp [untouched free gtmem pm] at some

theorem pairView_enabled (c : Config P) (x : State P) (a : Storage32.Action P)
    (normal : c.mutant = "none") (own : GateOwnership x.gates)
    (en : enabledPrior c x a = true) : Storage35.enabled c x.pairView (.prior a) = true := by
  obtain ⟨base, gate⟩ := Bool.and_eq_true_iff.mp en
  apply Bool.and_eq_true_iff.mpr
  refine ⟨base, ?_⟩
  cases a with
  | opRename src dst refs rt =>
    change (!Storage32.touches x.pairView.s.to32 (union refs [src, dst])) = true
    simp [pairView_untouched x _ (by simpa using gate)]
  | opDelete p =>
    change (!Storage32.touches x.pairView.s.to32 [p]) = true
    simp [pairView_untouched x _ (by simpa using gate)]
  | flushDel p =>
    change (c.mutant == "MDF" || !Storage32.gated c x.pairView.s.to32 p) = true
    simp [pairView_ungated x c p normal own (by simpa using gate)]
  | load p => rfl
  | old a =>
    cases a <;> try rfl
    case flush p =>
      change (!Storage32.gated c x.pairView.s.to32 p) = true
      simp [pairView_ungated x c p normal own (by simpa using gate)]
    case deliverUp ok =>
      change (match x.core.s.up with
        | m :: _ => m.kind != "discard" ||
          !(Storage32.partner x.pairView.s.to32 m.p).any (locked x.core.s.toSys)
        | [] => true) = true
      have raw : (match x.core.s.up with
        | m :: _ => m.kind != "discard" ||
          !(Storage32.partner x.legacyView.s.to32 m.p).any (locked x.core.s.toSys)
        | [] => true) = true := gate
      cases hq : x.core.s.up with
      | nil => rfl
      | cons m tail =>
        change (m.kind != "discard" || !(Storage32.partner x.pairView.s.to32 m.p).any
          (locked x.core.s.toSys)) = true
        rw [pairView_partner]
        simpa only [hq] using raw

end Stage6
