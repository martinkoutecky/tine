import Stage6CoreSafety

namespace Stage6
open Storage21

theorem mem_exclude {p : Slot P} {ps qs : List (Slot P)} :
    p ∈ exclude ps qs ↔ p ∈ ps ∧ p ∉ qs := by
  simp [exclude]

theorem endpoint_mem {moves : List (Move P)} {m : Move P} (hm : m ∈ moves) :
    m.dst ∈ endpoints moves ∧ m.src ∈ endpoints moves := by
  exact ⟨List.mem_flatMap.mpr ⟨m, hm, by simp⟩, List.mem_flatMap.mpr ⟨m, hm, by simp⟩⟩

theorem batch_endpoints_in_keys (b : Batch P) (shape : b.Shape) :
    ∀ p ∈ endpoints b.moves, p ∈ b.keys := by
  intro p pm
  obtain ⟨m, mm, eq⟩ := List.mem_flatMap.mp pm
  have ends := move_endpoints_in_keys b shape m mm
  rcases (show p = m.dst ∨ p = m.src from by simpa using eq) with rfl | rfl
  · exact ends.2
  · exact ends.1

theorem endpoint_distinct {moves : List (Move P)} (nd : (endpoints moves).Nodup)
    {m : Move P} (hm : m ∈ moves) : m.src ≠ m.dst := by
  induction moves with
  | nil => simp at hm
  | cons n rest ih =>
    simp only [endpoints, List.flatMap_cons, List.cons_append, List.nil_append, List.nodup_cons] at nd
    rcases List.mem_cons.mp hm with rfl | hm
    · exact fun eq => nd.1 (by simp [eq])
    · exact ih nd.2.2 hm

theorem endpoint_unique {moves : List (Move P)} (nd : (endpoints moves).Nodup)
    {m n : Move P} (hm : m ∈ moves) (hn : n ∈ moves) {p : Slot P}
    (mp : p = m.dst ∨ p = m.src) (np : p = n.dst ∨ p = n.src) : m = n := by
  induction moves with
  | nil => simp at hm
  | cons k rest ih =>
    simp only [endpoints, List.flatMap_cons, List.cons_append, List.nil_append, List.nodup_cons] at nd
    have notMem (eq : p = k.dst ∨ p = k.src) : p ∉ endpoints rest := by
      rcases eq with rfl | rfl
      · exact fun mem => nd.1 (List.mem_cons_of_mem _ mem)
      · exact nd.2.1
    have inRest (l : Move P) (lm : l ∈ rest) (lp : p = l.dst ∨ p = l.src) :
        p ∈ endpoints rest := by
      rcases lp with rfl | rfl
      · exact (endpoint_mem lm).1
      · exact (endpoint_mem lm).2
    rcases List.mem_cons.mp hm with rfl | mt <;> rcases List.mem_cons.mp hn with rfl | nt
    · rfl
    · exact False.elim (notMem mp (inRest _ nt np))
    · exact False.elim (notMem np (inRest _ mt mp))
    · exact ih nd.2.2 mt nt

def Gate.Shape (gt : Gate P) : Prop :=
  (endpoints gt.moves).Nodup ∧
  (∀ p ∈ endpoints gt.moves, (gt.vers p).isSome = true) ∧
  (∀ p ∈ gt.pending, (gt.vers p).isSome = true) ∧
  (∀ p v, gt.vers p = some v → 0 ≤ v)

def Receiver (gt : Gate P) (p : Slot P) : Prop := ∃ m ∈ gt.moves, p = m.dst
def Source (gt : Gate P) (p : Slot P) : Prop := ∃ m ∈ gt.moves, p = m.src

theorem source_not_receiver {gt : Gate P} (shape : gt.Shape) {p : Slot P}
    (src : Source gt p) : ¬Receiver gt p := by
  rintro ⟨d, dm, hd⟩
  obtain ⟨s, sm, hs⟩ := src
  have same := endpoint_unique shape.1 sm dm (Or.inr hs) (Or.inl hd)
  subst d
  exact endpoint_distinct shape.1 sm (hs.symm.trans hd)

def GateOwnership (gates : List (Gate P)) : Prop :=
  (∀ gt ∈ gates, gt.Shape) ∧
  (∀ gt ∈ gates, ∀ ht ∈ gates, ∀ p,
    (gt.vers p).isSome = true → (ht.vers p).isSome = true → gt = ht)

theorem ownership_empty : GateOwnership ([] : List (Gate P)) := by
  simp [GateOwnership]

theorem retained_eq {gt ht : Gate P} (h : gt.retained = some ht) : gt = ht := by
  unfold Gate.retained at h
  split at h <;> simp_all

theorem retained_some {gt : Gate P} (moves : gt.moves ≠ []) (pending : gt.pending ≠ []) :
    gt.retained = some gt := by
  simp [Gate.retained, List.isEmpty_eq_false_iff.mpr moves, List.isEmpty_eq_false_iff.mpr pending]

def Gate.Descends (next prev : Gate P) : Prop :=
  next.vers = prev.vers ∧ next.moves.Sublist prev.moves ∧ next.pending.Sublist prev.pending

private theorem endpoint_sublist {ms ns : List (Move P)} (sub : ms.Sublist ns) :
    (endpoints ms).Sublist (endpoints ns) := by
  induction sub with
  | slnil => exact .slnil
  | cons m _ ih => exact .cons _ (.cons _ ih)
  | cons_cons m _ ih => exact .cons_cons _ (.cons_cons _ ih)

private theorem shape_descends {next prev : Gate P} (shape : prev.Shape) (d : next.Descends prev) :
    next.Shape := by
  obtain ⟨ver, moves, pending⟩ := d
  exact ⟨(endpoint_sublist moves).nodup shape.1,
    fun p hm => ver ▸ shape.2.1 p ((endpoint_sublist moves).subset hm),
    fun p hm => ver ▸ shape.2.2.1 p (pending.subset hm),
    fun p v h => shape.2.2.2 p v (ver ▸ h)⟩

private theorem ownership_filterMap {gates : List (Gate P)} {update : Gate P → Option (Gate P)}
    (own : GateOwnership gates) (down : ∀ g h, update g = some h → h.Descends g) :
    GateOwnership (gates.filterMap update) := by
  constructor
  · intro ht hm
    obtain ⟨gt, gm, same⟩ := List.mem_filterMap.mp hm
    exact shape_descends (own.1 gt gm) (down gt ht same)
  · intro gt gm ht hm p gp hp
    obtain ⟨g, gmem, ge⟩ := List.mem_filterMap.mp gm
    obtain ⟨h, hmem, he⟩ := List.mem_filterMap.mp hm
    have eq := own.2 g gmem h hmem p ((down g gt ge).1 ▸ gp) ((down h ht he).1 ▸ hp)
    exact Option.some.inj (ge.symm.trans (eq ▸ he))

theorem ownership_published {gates : List (Gate P)} (own : GateOwnership gates) (p : Slot P) (v : Int) :
    GateOwnership (published gates p v) := by
  apply ownership_filterMap own
  intro g h mem
  split at mem
  · obtain rfl := retained_eq mem
    exact ⟨rfl, List.Sublist.refl _, List.filter_sublist⟩
  · obtain rfl := retained_eq mem
    exact ⟨rfl, List.Sublist.refl _, List.Sublist.refl _⟩

theorem ownership_discarded {gates : List (Gate P)} (own : GateOwnership gates) (p : Slot P) :
    GateOwnership (discarded gates p) := by
  apply ownership_filterMap own
  intro g h mem
  split at mem
  · obtain rfl := retained_eq mem
    exact ⟨rfl, List.filter_sublist, List.filter_sublist⟩
  · obtain rfl := retained_eq mem
    exact ⟨rfl, List.Sublist.refl _, List.Sublist.refl _⟩

theorem untouched {gates : List (Gate P)} {keys : List (Slot P)} (free : touches gates keys = false)
    {gt : Gate P} (gm : gt ∈ gates) {p : Slot P} (pm : p ∈ keys) : gt.vers p = none := by
  have noneOwned : (keys.any fun p => (gt.vers p).isSome) = false :=
    Bool.eq_false_iff.mpr (List.any_eq_false.mp free gt gm)
  have no := List.any_eq_false.mp noneOwned p pm
  simpa using no

def newGate (x : State P) (moves : List (Move P)) (keys : List (Slot P)) : Gate P :=
  ⟨moves, fun p => if keys.contains p then some (version x p) else none, keys⟩

theorem new_gate_version {x : State P} {moves : List (Move P)} {keys : List (Slot P)}
    {p : Slot P} (mem : p ∈ keys) : (newGate x moves keys).vers p = some (version x p) := by
  simp [newGate, mem]

theorem ownership_install (x : State P) (moves : List (Move P)) (keys : List (Slot P))
    (old : GateOwnership x.gates) (distinct : (endpoints moves).Nodup)
    (ends : ∀ p ∈ endpoints moves, p ∈ keys) (free : touches x.gates keys = false)
    (nonneg : 0 ≤ x.core.g.vc) : GateOwnership (install x moves keys) := by
  unfold install
  split
  · exact old
  let fresh := newGate x moves keys
  have owns (p : Slot P) : (fresh.vers p).isSome = true ↔ p ∈ keys := by
    by_cases mem : p ∈ keys <;> simp [fresh, newGate, mem]
  have shape : fresh.Shape := by
    refine ⟨distinct, fun p hm => (owns p).mpr (ends p hm), fun p hm => (owns p).mpr hm, ?_⟩
    intro p v h
    simp only [fresh, newGate] at h
    split at h
    · have eq := Option.some.inj h
      unfold version at eq
      omega
    · cases h
  change GateOwnership (x.gates ++ [fresh])
  simp only [GateOwnership, List.mem_append, List.mem_singleton]
  constructor
  · rintro gt (mem | rfl)
    · exact old.1 gt mem
    · exact shape
  · rintro gt (gm | rfl) ht (hm | rfl) p gp hp
    · exact old.2 gt gm ht hm p gp hp
    · simp [untouched free gm ((owns p).mp hp)] at gp
    · simp [untouched free hm ((owns p).mp gp)] at hp
    · rfl

theorem rename_admission_facts {c : Config P} {x : State P} {src dst : Slot P}
    {refs : List (Slot P)} {rt : Slot P → Int} (en : enabledPrior c x (.opRename src dst refs rt) = true)
    (full : Storage3.opCur x.core.s src ≠ absent) :
    src ≠ dst ∧ Storage3.opFree x.core.s (union [src, dst] refs) = true ∧
    (∀ p ∈ refs, p ≠ dst ∧ p ≠ src) ∧ touches x.gates (union [src, dst] refs) = false := by
  have h := (Bool.and_eq_true_iff.mp (Bool.and_eq_true_iff.mp en).1).1
  change Storage3.enabled c x.core (.opRename src dst refs rt) = true at h
  have full' : (Storage3.opCur x.core.s src != absent) = true := bne_iff_ne.mpr full
  simp only [Storage3.enabled, full', if_true, Bool.and_eq_true, bne_iff_ne] at h
  have no := (Bool.and_eq_true_iff.mp en).2
  change (!touches x.gates (union refs [src, dst])) = true at no
  have no' : touches x.gates (union refs [src, dst]) = false := by simpa using no
  refine ⟨by grind, by grind, ?_, ?_⟩
  · have checked : refs.all (fun r => r != src && r != dst && c.texts.contains (rt r)) = true := by grind
    intro p pm
    have h' := List.all_eq_true.mp checked p pm
    simp only [Bool.and_eq_true, bne_iff_ne] at h'
    exact ⟨h'.1.2, h'.1.1⟩
  · apply List.any_eq_false.mpr
    intro gt gm
    apply Bool.eq_false_iff.mp
    apply List.any_eq_false.mpr
    intro p pm
    have reversed : p ∈ union refs [src, dst] := by
      rw [mem_union_general] at pm ⊢
      exact pm.symm
    simp [untouched no' gm reversed]

theorem ownership_execute (c : Config P) (x : State P) (a : Action P)
    (own : GateOwnership x.gates) (nonneg : 0 ≤ x.core.g.vc) (en : enabled c x a = true) :
    GateOwnership (execute c x a).gates := by
  cases a with
  | malformedApply | extClaim => exact own
  | opApply b =>
    have shape := enabled_shape c x b en
    have free := (Bool.and_eq_true_iff.mp (Bool.and_eq_true_iff.mp en).1).2
    exact ownership_install x b.moves b.keys own shape.2.1 (batch_endpoints_in_keys b shape)
      (by simpa using free) nonneg
  | prior a =>
    change GateOwnership (priorGates x a)
    cases a with
    | load | flushDel | opDelete => exact own
    | opRename src dst refs rt =>
      simp only [priorGates]
      split
      · have facts := rename_admission_facts en (by simpa using ‹(Storage3.opCur x.core.s src != absent) = true›)
        refine ownership_install x [⟨dst, src⟩] (union [src, dst] refs) own ?_ ?_ facts.2.2.2 nonneg
        · simpa [endpoints] using facts.1.symm
        · intro p pm
          apply (mem_union_general _ _ _).mpr
          exact Or.inl (by simpa [endpoints, or_comm] using pm)
      · exact own
    | old a =>
      cases a <;> try exact own
      case switchFin => exact ownership_empty
      case crash => exact ownership_empty
      case power => exact ownership_empty
      case dirSync ok =>
        cases ok
        · exact own
        · exact ownership_published own _ _
      case deliverUp ok =>
        simp only [priorGates]
        split
        · split
          · exact ownership_discarded own _
          · exact own
        · exact own

theorem gate_ownership_reachable (c : Config P) (disk : Slot P → Int) (x : State P)
    (normal : c.mutant = "none") (reached : Reachable c disk x) : GateOwnership x.gates := by
  induction reached with
  | initial => exact ownership_empty
  | @advance x y reached a next ih =>
    obtain ⟨en, rfl⟩ := step_eq_some_iff.mp next
    exact ownership_execute c x a ih (core_safety_reachable c disk x normal reached).1.bounds.nonnegative en

#print axioms gate_ownership_reachable

end Stage6
