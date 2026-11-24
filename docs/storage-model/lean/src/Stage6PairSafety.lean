import Stage6PairSteps
import Stage6OrderSafety

namespace Stage6
open Storage21

private theorem apply_gates_subset (c : Config P) (x : State P) (b : Batch P)
    (g : Storage32.Gate P) (mem : g ∈ x.pairView.s.gates) :
    g ∈ (opApply c x b).pairView.s.gates := by
  obtain ⟨gt, gm, m, mm, eq⟩ := pairGate_mem.mp mem
  apply pairGate_mem.mpr
  refine ⟨gt, ?_, m, mm, eq⟩
  change gt ∈ install x b.moves b.keys
  unfold install
  split
  · exact gm
  · exact List.mem_append_left _ gm

private theorem apply_pairs (c : Config P) (x : State P) (b : Batch P)
    (r : Slot P × Slot P) (mem : r ∈ (opApply c x b).renamed) :
    (r ∈ x.renamed ∧ r.1 ∉ b.keys ∧ r.2 ∉ b.keys) ∨
      ∃ m ∈ b.moves, x.core.s.disk m.src ≠ absent ∧ r = (m.src, m.dst) := by
  change r ∈ union (Storage32.unrename x.renamed b.keys)
    ((b.moves.filter fun m => x.core.s.disk m.src != absent).map fun m => (m.src, m.dst)) at mem
  rw [mem_union_general] at mem
  rcases mem with old | fresh
  · obtain ⟨old, frame⟩ := List.mem_filter.mp old
    exact Or.inl ⟨old, by simpa using frame⟩
  · obtain ⟨m, mm, eq⟩ := List.mem_map.mp fresh
    obtain ⟨mm, disk⟩ := List.mem_filter.mp mm
    exact Or.inr ⟨m, mm, bne_iff_ne.mp disk, eq.symm⟩

private theorem apply_pairSafety (c : Config P) (x : State P) (b : Batch P)
    (normal : c.mutant = "none") (safe : Storage35.PairSafety10 x.pairView)
    (en : enabledApply c x b = true) : Storage35.PairSafety10 (opApply c x b).pairView := by
  let y := (opApply c x b).pairView
  let old := {y with g := {y.g with renamed := Storage32.unrename x.renamed b.keys}}
  have oldSafe : Storage35.PairSafety10 old := by
    apply Storage35.pairSafety_endpoints11 x.pairView old safe rfl
    · intro r rm
      exact (List.mem_filter.mp rm).1
    · intro r rm
      have outside : r.1 ∉ b.keys ∧ r.2 ∉ b.keys := by
        simpa using (List.mem_filter.mp rm).2
      have a := (admission_frame c x b).2.2 r.1 outside.1
      have d := (admission_frame c x b).2.2 r.2 outside.2
      exact ⟨(show Storage35.PageSafe11 (x.core.s.disk r.1) (x.core.s.pages r.1)
          ((opApply c x b).core.s.pages r.1) from a.1 ▸ Storage35.pageSafe_refl11 _ _),
        (show Storage35.PageSafe11 (x.core.s.disk r.2) (x.core.s.pages r.2)
          ((opApply c x b).core.s.pages r.2) from d.1 ▸ Storage35.pageSafe_refl11 _ _)⟩
    · exact apply_gates_subset c x b
    · rfl
  intro r rm
  rcases apply_pairs c x b r rm with ⟨prev, src, dst⟩ | ⟨m, mm, disk, rfl⟩
  · have mem : r ∈ old.g.renamed := by
      change r ∈ Storage32.unrename x.renamed b.keys
      simp [Storage32.unrename, prev, src, dst]
    exact oldSafe r mem
  · have shape := enabled_shape c x b en
    have ⟨sm, dm⟩ := move_endpoints_in_keys b shape m mm
    have dest := (shape.2.2 m mm).2.1
    have draft := admission_versions c x b normal m.dst dm
    have free := apply_free en
    have pending : PendingMove (opApply c x b).gates m.src m.dst := by
      obtain ⟨gt, gates, moves, keys, _⟩ := admission_gate c x b (by intro no; simp [no] at mm)
      refine ⟨gt, by simp [gates], ?_, keys ▸ sm, keys ▸ dm⟩
      cases m
      exact moves ▸ mm
    right; right
    refine ⟨disk, draft.1, ?_, draft.2.2.2.1, ?_, ?_, (pendingMove_iff _ _ _).mp pending⟩
    · change ((opApply c x b).core.s.pages m.dst).buf ≠ absent
      rw [draft.2.1]
      exact dest
    · intro on
      exact Ne.symm (Storage32.opFree_excludes9 _ _ free on m.src sm)
    · intro on eq
      exact False.elim (Storage32.opFree_excludes9 _ _ free on m.dst dm eq.symm)

structure PairFacts (x : State P) : Prop where
  published : Storage32.PairPublished11 (Storage35.project x.pairView)
  live : Storage35.LivePairs11 x.pairView
  inputs : InputTexts11 x.core.s.w x.core.s.up

private theorem apply_pairFacts (c : Config P) (x : State P) (b : Batch P) (h : PairFacts x)
    (en : enabledApply c x b = true) : PairFacts (opApply c x b) := by
  refine ⟨?_, ?_, h.inputs⟩
  · apply Storage32.pairPublished_job_same11 (Storage35.project x.pairView)
      (Storage35.project (opApply c x b).pairView) h.published rfl rfl
    intro r rm
    rcases apply_pairs c x b r rm with ⟨prev, _, _⟩ | ⟨m, mm, _, rfl⟩
    · exact Or.inl prev
    · right
      intro on
      have ends := move_endpoints_in_keys b (enabled_shape c x b en) m mm
      have free := Storage32.opFree_excludes9 _ _ (apply_free en) on
      exact ⟨Ne.symm (free _ ends.1), Ne.symm (free _ ends.2)⟩
  · have base := apply_enabled_legacy en
    simp only [Storage34.enabledApply, Storage33.enabledApply, Bool.and_eq_true] at base
    have on : x.core.s.alive = true := base.1.1.1.1.1.1.1.1.1
    intro off
    change x.core.s.alive = false at off
    simp [on] at off

private theorem prior_pairFacts (c : Config P) (x : State P) (a : Storage32.Action P)
    (normal : c.mutant = "none") (texts : absent ∉ c.texts) (own : GateOwnership x.gates)
    (h : PairFacts x) (en : enabledPrior c x a = true) : PairFacts (execute c x (.prior a)) := by
  let z := execute c x (.prior a)
  let y := Storage35.execute c x.pairView (.prior a)
  have enabled := pairView_enabled c x a normal own en
  have published := Storage35Base.pairPublished_execute11 c (Storage35.project x.pairView) a normal h.published enabled
  have live := Storage35.livePairs_execute11 c x.pairView (.prior a) normal h.live enabled
  have inputs := Storage35Base.inputs_execute11 c (Storage35.project x.pairView) a normal texts h.inputs enabled
  obtain ⟨core, renamed⟩ := coreAndPairs_eq c x a normal
  have disk : y.s.disk = z.core.s.disk := congrArg (fun t : Storage31.State P => t.s.disk) core
  have job : y.s.job = z.core.s.job := congrArg (fun t : Storage31.State P => t.s.job) core
  refine ⟨?_, ?_, ?_⟩
  · apply Storage32.pairPublished_frame11 (Storage35.project y) _ published disk.symm
    · intro r rm
      exact renamed ▸ rm
    · intro on phase
      exact ⟨job ▸ on, job ▸ phase, congrArg Job.p job.symm, congrArg Job.bytes job.symm⟩
  · intro off
    have alive : y.s.alive = z.core.s.alive := congrArg (fun t : Storage31.State P => t.s.alive) core
    change z.renamed = []
    exact renamed ▸ live (alive ▸ off)
  · have w : y.s.w = z.core.s.w := congrArg (fun t : Storage31.State P => t.s.w) core
    have up : y.s.up = z.core.s.up := congrArg (fun t : Storage31.State P => t.s.up) core
    exact w ▸ up ▸ inputs

theorem pair_safety_reachable (c : Config P) (disk : Slot P → Int) (x : State P)
    (normal : c.mutant = "none") (reached : Reachable c disk x) :
    Storage35.PairSafety10 x.pairView ∧ PairFacts x := by
  have texts := (reachable_valid reached).2.1
  induction reached with
  | initial =>
    exact ⟨(by intro _ rm; cases rm), ⟨(by intro _ rm; cases rm), fun _ => rfl, inputs_nil11⟩⟩
  | @advance x y reached a next ih =>
    obtain ⟨en, rfl⟩ := step_eq_some_iff.mp next
    obtain ⟨safe, facts⟩ := ih
    cases a with
    | malformedApply => exact ⟨safe, facts⟩
    | extClaim => exact ⟨safe, ⟨facts.published, facts.live, facts.inputs⟩⟩
    | opApply b => exact ⟨apply_pairSafety c x b normal safe en, apply_pairFacts c x b facts en⟩
    | prior a =>
      have own := gate_ownership_reachable c disk x normal reached
      have kinds := (core_safety_reachable c disk x normal reached).1.protocol.kinds
      exact ⟨pairSafety_prior c x a normal texts own safe facts.published facts.live kinds facts.inputs en,
        prior_pairFacts c x a normal texts own facts en⟩

theorem renameResolves_reachable (c : Config P) (disk : Slot P → Int) (x : State P)
    (normal : c.mutant = "none") (reached : Reachable c disk x) : renameResolves x = true :=
  Storage35.pairSafety_resolves10 x.pairView (pair_safety_reachable c disk x normal reached).1

theorem all_targets_arbitrary_initial_disk (c : Config P) (disk : Slot P → Int) (x : State P)
    (normal : c.mutant = "none") (reached : Reachable c disk x) :
    guarantee x = true ∧ orderHolds x = true ∧ renameResolves x = true :=
  ⟨guarantee_reachable c disk x normal reached, orderHolds_reachable c disk x normal reached,
    renameResolves_reachable c disk x normal reached⟩

#print axioms pair_safety_reachable
#print axioms renameResolves_reachable
#print axioms all_targets_arbitrary_initial_disk

end Stage6
