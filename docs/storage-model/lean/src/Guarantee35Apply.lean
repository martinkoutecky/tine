import BatchApply35
namespace Storage33
open Storage21 Storage3
set_option maxHeartbeats 8000000
set_option maxRecDepth 8192
set_option linter.unusedSimpArgs false
theorem congr2_10 (f : α → β → γ) (x y : α) (u v : β)
    (h : x = y) (k : u = v) : f x u = f y v := by cases h; cases k; rfl
theorem ghost_full_eq10 (x y : Storage3.Ghost P) (core : x.toGhost = y.toGhost)
    (reads : x.opRead = y.opRead) (removed : x.removed = y.removed)
    (durable : x.delDurable = y.delDurable) : x = y := by cases x; cases y; simp_all

theorem fold_wrote_page10 (ps : List (Fin P)) (wr : Fin P → List (Int × Int))
    (bytes vers : Fin P → Int) (p : Fin P) :
    ps.foldl (fun acc q => put acc q (union (acc q) [(bytes q,vers q)])) wr p =
      if p ∈ ps then union (wr p) [(bytes p,vers p)] else wr p := by
  induction ps generalizing wr with
  | nil => simp
  | cons q tail ih =>
    rw [List.foldl_cons,ih]
    by_cases same : p = q
    · subst q; simp [put,union_repeat10]
    · by_cases mem : p ∈ tail <;> simp [put,same,mem]

theorem apply_eq_batch10 (c : Config P) (x : State P) (eff plan : Map P)
    (dst src : Option (Fin P)) (mutant : c.mutant = "none")
    (h : Storage31.Inv c (Storage32.project x)) (en : enabledApply c x eff plan dst src = true) :
    Storage32.project (opApply c x eff plan dst src) =
      Storage31.batch10 c (Storage32.project x) (keys eff) (get eff) := by
  have admission := apply_admission8 c x eff plan dst src mutant en
  have cleanAll : (keys eff).all (Storage3.opClean x.s.toBase) = true := by
    simp only [enabledApply,Bool.and_eq_true] at en
    simpa [mutant] using en.1.1.1.2
  have staleFalse : stale x eff plan = false := by
    simp only [enabledApply,Bool.and_eq_true] at en
    simpa [mutant] using en.1.1.2
  have named : (keys eff).filter (Storage3.opClean x.s.toBase) = keys eff := by
    apply List.filter_eq_self.mpr
    exact List.all_eq_true.mp cleanAll
  have acknew : ∀ p, ack (x.g.promise p) (get eff p) (x.g.vc+1+p.val) false 0 =
      (⟨true,get eff p,x.g.vc+1+p.val,false,0⟩ : Promise) := by
    intro p
    have bound := h.lineage.1 p
    change (x.g.promise p).ver ≤ x.g.vc at bound
    exact ack_newer _ _ _ _ _ (by omega)
  have base : ∀ p ∈ keys eff,get plan p = (x.s.pages p).base := by
    intro p mem
    obtain ⟨held,cleaned,planEq⟩ := admission p mem
    have bufBase : (x.s.pages p).buf = (x.s.pages p).base := by simp [clean] at cleaned; exact cleaned.1.2
    simpa [get,planEq,Storage3.opCur,held] using bufBase
  simp only [opApply,Storage32.commit,Storage32.project,Storage31.batch10]
  apply congr2_10 (fun s g => Storage3.commit c (Storage32.project x) s g "opApply")
  · apply congrArg (fun s : Storage21.Sys P => (⟨s,x.s.trash,x.s.trashStable⟩ : Storage3.Sys P))
    apply sys_eq10
    all_goals first | rfl | skip
    all_goals funext p
    all_goals by_cases mem : p ∈ keys eff
    all_goals first
      | (simp [fold_put_if,mem,mutant,Storage31.batchSys10,Storage31.batchPage10,
          Storage31.batchVer10,Storage32.project,Storage3.opEdit,(admission p mem).1,draftEntry])
      | (simp [fold_put_if,mem,mutant,Storage31.batchSys10,Storage32.project])
  · apply ghost_full_eq10
    · apply ghost_eq10
      all_goals first | rfl | skip
      all_goals try simp only [cleanAll,staleFalse,tags,Bool.not_true,Bool.false_eq_true,ite_false,named,
        Storage31.batchGhost10]
      all_goals try rfl
      all_goals try
        funext p
        simp [fold_ack_page8,fold_wrote_page10,acknew,Storage31.batchGhost10,
          Storage31.batchVer10,Storage32.project]
    · funext p
      by_cases mem : p ∈ keys eff
      · simp [Storage31.batchGhost10,Storage32.project,mem,base p mem]
      · simp [Storage31.batchGhost10,Storage32.project,mem]
    · rfl
    · rfl
end Storage33
namespace Storage35
open Storage21 Storage3
set_option maxHeartbeats 8000000
set_option linter.unusedSimpArgs false
theorem apply_inv35_10 (c : Config P) (x : State P) (eff plan : Storage34.Map P)
    (dst src : Option (Fin P)) (mutant : c.mutant = "none")
    (h : Storage31.Inv c (Storage32.project (project x)))
    (en : Storage34.enabledApply c x eff plan dst src = true) :
    Storage31.Inv c (Storage32.project (project (opApply c x eff plan dst src))) := by
  have baseEn : Storage33.enabledApply c (project x) eff plan dst src = true := by
    simp only [Storage34.enabledApply,Bool.and_eq_true] at en; exact en.1
  have admission := Storage33.apply_admission8 c (project x) eff plan dst src mutant baseEn
  have alive : x.s.alive = true := by simp [Storage33.enabledApply,project,Storage34.project] at baseEn; grind
  have free : Storage3.opFree x.s.to32.toBase (Storage33.keys eff) = true := by
    simp [Storage33.enabledApply,project,Storage34.project] at baseEn; grind
  have real : ∀ p ∈ Storage33.keys eff,RealByte c (Storage33.get eff p) := by
    have valid : (Storage33.keys eff).all (fun p => Storage33.get eff p == absent || c.texts.contains (Storage33.get eff p)) = true := by
      simp only [Storage33.enabledApply,Bool.and_eq_true] at baseEn; grind
    intro p mem; simpa [RealByte] using List.all_eq_true.mp valid p mem
  change Storage31.Inv c (Storage32.project (Storage34.project (Storage34.opApply c x eff plan dst src)))
  rw [Storage34.apply_refines33_9 c x eff plan dst src
    (Storage34.apply_slot_free9 c x eff plan dst src mutant en)]
  rw [Storage33.apply_eq_batch10 c (project x) eff plan dst src mutant h baseEn]
  apply Storage31.batch_inv10 c _ (Storage33.keys eff) (Storage33.get eff) h alive
  · intro p mem; exact (admission p mem).1
  · intro p mem; exact (admission p mem).2.1
  · exact real
  · intro on
    change x.s.job.on = true at on
    change x.s.job.p ∉ Storage33.keys eff
    simpa [Storage3.opFree,on] using free

theorem inv_reachable35_10 (c : Config P) (disk : Fin P → Int) (x : State P)
    (mutant : c.mutant = "none") (realDisk : ∀ p,RealByte c (disk p))
    (noNone : NoNone6 c) (noUnknown : NoUnknown6 c) (hr : Reachable c disk x) :
    Storage31.Inv c (Storage32.project (project x)) := by
  induction hr with
  | initial => exact Storage31.inv_initial c disk mutant realDisk noUnknown
  | @advance x y hr a hs ih =>
    unfold step at hs; split at hs
    · cases hs
      have en : enabled c x a = true := by assumption
      cases a with
      | prior a => exact Storage35Base.inv_execute_noApply10 c (project x) a mutant noNone noUnknown ih en
      | opApply eff plan dst src => exact apply_inv35_10 c x eff plan dst src mutant ih en
      | extClaim p present =>
        change Storage31.Inv c (Storage32.project (Storage34.project (Storage34.extClaim c x p present)))
        rw [Storage34.extClaim_projects_idle9 c x p present (aux_reachable10 c disk x mutant hr).idle]
        exact ih
    · contradiction

theorem guarantee_reachable35_10 (c : Config P) (T : Nat)
    (texts : c.texts = positiveTexts4 T) (two : 2 ≤ T) (mutant : c.mutant = "none")
    (x : State P) (hr : Reachable c (Storage31.initDisk c) x) : guarantee x = true :=
  Storage31.guarantee_of_inv7 c _ (inv_reachable35_10 c _ x mutant
    (positiveTexts_initial_real4 c T texts two) (positiveTexts_noNone6 c T texts)
    (positiveTexts_noUnknown6 c T texts) hr)
end Storage35
