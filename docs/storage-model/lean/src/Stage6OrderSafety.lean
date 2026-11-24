import Stage6OrderEdges

namespace Stage6
open Storage21

structure OrderSafety (x : State P) : Prop where
  edges : OrderEdges x.gates x.obl
  free : x.core.s.job.on = true → ∀ o ∈ x.obl, o.1 ≠ x.core.s.job.p
  holds : x.orderOk = true

private theorem empty_edges (gates : List (Gate P))
    (closed : ∀ gt ∈ gates, ∀ m ∈ gt.moves, m.src ∈ gt.pending → m.dst ∉ gt.pending) :
    OrderEdges gates [] := by
  constructor
  · simp
  · intro gt gm m mm sm dm
    exact False.elim (closed gt gm m mm sm dm)

private theorem empty_order {x : State P} (gates : x.gates = []) (obl : x.obl = [])
    (ok : x.orderOk = true) : OrderSafety x := by
  refine ⟨?_, ?_, ok⟩
  · rw [gates, obl]
    exact empty_edges [] (by simp)
  · simp [obl]

private theorem order_frame {x y : State P} (h : OrderSafety x)
    (gates : y.gates = x.gates) (obl : y.obl = x.obl) (ok : y.orderOk = x.orderOk)
    (job : JobFollows x.core.s.toSys y.core.s.toSys) : OrderSafety y := by
  refine ⟨by rw [gates, obl]; exact h.edges, ?_, ok.trans h.holds⟩
  intro on o om
  have ⟨old, p⟩ := job on
  rw [obl] at om
  rw [p]
  exact h.free old o om

private theorem installed_orders {x y : State P} (h : OrderSafety x)
    (moves : List (Move P)) (keys : List (Slot P)) (fresh : List (Storage32.Obl P))
    (distinct : (endpoints moves).Nodup) (ends : ∀ p ∈ endpoints moves, p ∈ keys)
    (requirements : ∀ o, o ∈ fresh ↔ o ∈ obligations x moves keys)
    (gates : y.gates = install x moves keys) (obl : y.obl = union x.obl fresh)
    (ok : y.orderOk = x.orderOk) (job : y.core.s.job = x.core.s.job)
    (free : Storage3.opFree x.core.s keys = true) : OrderSafety y := by
  cases moves with
  | nil =>
    have no : fresh = [] := by
      apply List.eq_nil_iff_forall_not_mem.mpr
      intro o hm
      have hp := (requirements o).mp hm
      simp [obligations, mem_union_general] at hp
    have same : y.obl = x.obl := by simpa [no, union] using obl
    exact order_frame h gates same ok (fun on => ⟨job ▸ on, congrArg Job.p job⟩)
  | cons m moves =>
    let gt := newGate x (m :: moves) keys
    change y.gates = x.gates ++ [gt] at gates
    refine ⟨⟨?_, ?_⟩, ?_, ok.trans h.holds⟩
    · intro o om
      rw [obl, mem_union_general] at om
      rcases om with old | new
      · obtain ⟨os, gm, backed⟩ := h.edges.1 o old
        exact ⟨os, by simp [gates, gm], backed⟩
      · exact ⟨gt, by simp [gates], obligations_backed x _ _ distinct ends o ((requirements o).mp new)⟩
    · intro ht hm n nm sm dm v ver
      rw [gates, List.mem_append, List.mem_singleton] at hm
      rw [obl, mem_union_general]
      rcases hm with old | rfl
      · exact Or.inl (h.edges.2 ht old n nm sm dm v ver)
      · have same := Option.some.inj (ver.symm.trans (new_gate_version dm))
        subst v
        exact Or.inr ((requirements _).mpr (obligations_complete x _ _ nm))
    · intro on o om
      rw [job] at on ⊢
      rw [obl, mem_union_general] at om
      rcases om with old | new
      · exact h.free on o old
      · have backed := obligations_backed x _ _ distinct ends o ((requirements o).mp new)
        exact Storage32.opFree_excludes9 _ _ free on o.1 backed.1

private theorem singleton_requirements (x : State P) (src dst : Slot P) (refs : List (Slot P))
    (refsDistinct : ∀ p ∈ refs, p ≠ dst ∧ p ≠ src) (o : Storage32.Obl P) :
    o ∈ Storage32.makeObl (Storage35.project x.legacyView) dst src refs ↔
      o ∈ obligations x [⟨dst, src⟩] (union [src, dst] refs) := by
  have others (p : Slot P) : p ∈ exclude (union [src, dst] refs) (endpoints [⟨dst, src⟩]) ↔ p ∈ refs := by
    simp only [mem_exclude, mem_union_general, endpoints, List.flatMap_cons, List.flatMap_nil,
      List.append_nil, List.mem_cons, List.not_mem_nil, or_false]
    constructor
    · rintro ⟨(hs | hd) | hr, n⟩
      · exact False.elim (n (Or.inr hs))
      · exact False.elim (n (Or.inl hd))
      · exact hr
    · intro hr
      have h := refsDistinct p hr
      exact ⟨Or.inr hr, fun hd => hd.elim h.1 h.2⟩
  simp [Storage32.makeObl, obligations, mem_union_general, version, others,
    State.legacyView, Storage34.project, eq_comm]

private theorem prior_job_follows (c : Config P) (x : State P) (a : Storage32.Action P)
    (normal : c.mutant = "none") (noStart : Storage32.NotSaveStart a) :
    JobFollows x.core.s.toSys (execute c x (.prior a)).core.s.toSys := by
  let z := Storage35.project x.legacyView
  apply Storage35Base.preserve10 c z a (fun y => JobFollows x.core.s.toSys y.s.toBase.toSys)
  · exact Storage32.job_follows32_9 c z a normal noStart
  · intro ok m tail _ _
    have same : (Storage35Base.upDiscard c z {z.s with up := tail} m ok).s.job = x.core.s.job := by
      simp only [Storage35Base.upDiscard]
      split <;> (try split)
      all_goals simp only [Storage32.job_commit32_9, Storage21.job_ackMail9, Storage32.job_discarded9]
      all_goals rfl
    intro on
    exact ⟨congrArg Job.on same ▸ on, congrArg Job.p same⟩

private theorem order_start (x y : State P) (p : Slot P)
    (h : OrderSafety x) (own : GateOwnership x.gates) (ungated : (x.gates.any fun gt => gt.gated p) = false)
    (gates : y.gates = x.gates) (obl : y.obl = x.obl)
    (job : y.core.s.job.on = true → y.core.s.job.p = p)
    (ok : y.orderOk = (x.orderOk && !(x.obl.any fun o => o.1 == p))) : OrderSafety y := by
  have noEdge (o : Storage32.Obl P) (om : o ∈ x.obl) : o.1 ≠ p := by
    intro eq
    obtain ⟨gt, gm, back⟩ := h.edges.1 o om
    have gated := backed_gated (own.1 gt gm) back
    exact (List.any_eq_false.mp ungated gt gm) (eq ▸ gated)
  refine ⟨by rw [gates, obl]; exact h.edges, ?_, ?_⟩
  · intro on o om
    rw [job on]
    exact noEdge o (obl ▸ om)
  · rw [ok, h.holds]
    simp only [Bool.true_and, Bool.not_eq_true']
    exact List.any_eq_false.mpr (fun o om hit => noEdge o om (beq_iff_eq.mp hit))

private theorem order_delivery (c : Config P) (x : State P) (ok : Bool)
    (normal : c.mutant = "none") (own : GateOwnership x.gates) (h : OrderSafety x) :
    OrderSafety (execute c x (.prior (.old (.deliverUp ok)))) := by
  let y := execute c x (.prior (.old (.deliverUp ok)))
  have follow : JobFollows x.core.s.toSys y.core.s.toSys :=
    prior_job_follows c x _ normal ⟨by simp, by simp⟩
  cases hq : x.core.s.up with
  | nil =>
    apply order_frame h (job := follow)
    · simp [y, execute, priorGates, State.fromCore, hq]
    · simp [y, execute, Storage35.execute, Storage35Base.execute, Storage35Base.deliverUp,
        State.fromCore, State.legacyView, Storage34.project, Storage34.lift, hq]
    · simp [y, execute, Storage35.execute, Storage35Base.execute, Storage35Base.deliverUp,
        State.fromCore, State.legacyView, Storage34.project, Storage34.lift, hq]
  | cons m tail =>
    have ghost : y.obl = (if m.kind == "discard" && ok && (x.core.s.pages m.p).held then
        Storage32.oblDiscarded x.obl m.p else x.obl) ∧ y.orderOk = x.orderOk := by
      simp only [y, execute, Storage35.execute, Storage35Base.execute, Storage35Base.deliverUp,
        State.fromCore, State.legacyView, Storage34.project, Storage34.lift, hq]
      by_cases kind : m.kind = "discard"
      · by_cases held : (x.core.s.pages m.p).held = true <;> cases ok <;>
          simp [kind, held, Storage35Base.upDiscard, Storage32.commit, Storage3.commit,
            Storage21.commit, Storage32.discarded]
      · simp [kind, Storage32.lift]
    have gates : y.gates = (if m.kind == "discard" && ok && (x.core.s.pages m.p).held then
        discarded x.gates m.p else x.gates) := by simp [y, execute, priorGates, State.fromCore, hq]
    by_cases cancel : (m.kind == "discard" && ok && (x.core.s.pages m.p).held) = true
    · rw [if_pos cancel] at gates
      have obl : y.obl = Storage32.oblDiscarded x.obl m.p := ghost.1.trans (if_pos cancel)
      refine ⟨by rw [gates, obl]; exact edges_discarded own h.edges m.p, ?_, ghost.2.trans h.holds⟩
      intro on o om
      obtain ⟨old, p⟩ := follow on
      rw [obl] at om
      exact p ▸ h.free old o (List.mem_filter.mp om).1
    · exact order_frame h (gates.trans (if_neg cancel)) (ghost.1.trans (if_neg cancel)) ghost.2 follow

theorem order_execute (c : Config P) (x : State P) (a : Action P) (normal : c.mutant = "none")
    (own : GateOwnership x.gates) (h : OrderSafety x) (en : enabled c x a = true) :
    OrderSafety (execute c x a) := by
  cases a with
  | malformedApply => exact h
  | extClaim => exact order_frame h rfl rfl rfl (fun on => ⟨on, rfl⟩)
  | opApply b =>
    have shape := enabled_shape c x b en
    exact installed_orders h b.moves b.keys (obligations x b.moves b.keys) shape.2.1 (batch_endpoints_in_keys b shape)
      (fun _ => Iff.rfl) rfl rfl rfl rfl (apply_free en)
  | prior a =>
    cases a with
    | flushDel p =>
      exact order_start x _ p h own (by simpa using (Bool.and_eq_true_iff.mp en).2)
        rfl rfl (fun _ => rfl) rfl
    | opRename src dst refs rt =>
      by_cases cur : Storage3.opCur x.core.s src = absent
      · apply order_frame h
        · simp [execute, State.fromCore, priorGates, cur]
        · simp [execute, Storage35.execute, Storage35Base.execute, Storage32.execute,
            Storage32.opRename, Storage32.lift, Storage34.lift, Storage34.project, State.fromCore,
            State.legacyView, cur]
        · rfl
        · exact fun on => ⟨on, rfl⟩
      · obtain ⟨distinct, free, refsDistinct, _⟩ := rename_admission_facts en cur
        refine installed_orders h [⟨dst, src⟩] (union [src, dst] refs) _
          (by simpa [endpoints] using distinct.symm) ?_ (singleton_requirements x src dst refs refsDistinct)
          ?_ ?_ rfl rfl free
        · intro p pm
          apply (mem_union_general _ _ _).mpr
          exact Or.inl (by simpa [endpoints, or_comm] using pm)
        · simp [execute, State.fromCore, priorGates, cur]
        · simp [execute, Storage35.execute, Storage35Base.execute, Storage32.execute, Storage32.opRename,
            Storage32.lift, Storage34.lift, Storage34.project, State.fromCore, State.legacyView, cur]
    | old a =>
      cases a with
      | flush p =>
        exact order_start x _ p h own (by simpa using (Bool.and_eq_true_iff.mp en).2)
          rfl rfl (fun _ => rfl) rfl
      | deliverUp ok => exact order_delivery c x ok normal own h
      | dirSync ok =>
        cases ok
        · exact order_frame h rfl rfl rfl (prior_job_follows c x _ normal ⟨by simp, by simp⟩)
        · have on : x.core.s.job.on = true := by
            have base := (Bool.and_eq_true_iff.mp en).1
            change Storage21.enabled c (Storage3.project x.core) (.dirSync true) = true at base
            exact (Bool.and_eq_true_iff.mp (Bool.and_eq_true_iff.mp base).1).2
          have off : (execute c x (.prior (.old (.dirSync true)))).core.s.job.on = false := by
            change (Storage32.project (Storage32.execute c (Storage35.project x.legacyView)
              (.old (.dirSync true)))).s.job.on = false
            rw [Storage32.execute_projects_noRestore9 c _ (.old (.dirSync true)) normal trivial]
            rfl
          exact ⟨edges_published h.edges _ _ (h.free on), (by rw [off]; simp), h.holds⟩
      | crash | power | switchFin => exact empty_order rfl rfl h.holds
      | _ =>
        all_goals exact order_frame h rfl rfl rfl (prior_job_follows c x _ normal ⟨by simp, by simp⟩)
    | _ =>
      all_goals exact order_frame h rfl rfl rfl (prior_job_follows c x _ normal ⟨by simp, by simp⟩)

theorem order_safety_reachable (c : Config P) (disk : Slot P → Int) (x : State P)
    (normal : c.mutant = "none") (reached : Reachable c disk x) : OrderSafety x := by
  induction reached with
  | initial => exact empty_order rfl rfl rfl
  | @advance x y reached a next ih =>
    obtain ⟨en, rfl⟩ := step_eq_some_iff.mp next
    exact order_execute c x a normal (gate_ownership_reachable c disk x normal reached) ih en

theorem orderHolds_reachable (c : Config P) (disk : Slot P → Int) (x : State P)
    (normal : c.mutant = "none") (reached : Reachable c disk x) : orderHolds x = true :=
  (order_safety_reachable c disk x normal reached).holds

#print axioms order_safety_reachable
#print axioms orderHolds_reachable

end Stage6
