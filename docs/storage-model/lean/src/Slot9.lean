import Storage34
import Apply8
import PartialGuarantee9
namespace Storage32
open Storage21 Storage3
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false

theorem vocabulary31_execute9 (c : Config P) (x : Storage31.State P) (a : Action P)
    (mutant : c.mutant = "none") (h : BadVocabulary x.g.toGhost) :
    BadVocabulary (Storage31.execute c x a).g.toGhost := by
  cases a with
  | load p => exact vocabulary_commit c (Storage3.project x) _ _ _ h
  | old a => exact vocabulary_execute3 c x (.old a) mutant h
  | opRename src dst refs rt => exact vocabulary_execute3 c x (.opRename src dst refs rt) mutant h
  | opDelete p => exact vocabulary_execute3 c x (.opDelete p) mutant h
  | flushDel p => exact vocabulary_execute3 c x (.flushDel p) mutant h

theorem vocabulary_discard9 (c : Config P) (x : State P) (sys : Sys P)
    (m : Up P) (rok : Bool) (h : BadVocabulary x.g.toBase.toGhost) :
    BadVocabulary (upDiscard c x sys m rok).g.toBase.toGhost := by
  simp only [upDiscard]
  split <;> (try split)
  all_goals apply vocabulary_commit
  · apply vocabulary_add _ _ h
    intro t ht; simp only [List.mem_singleton] at ht; subst t; simp [BadKinds]
  · exact h
  · exact h

theorem vocabulary_execute_noRestore9 (c : Config P) (x : State P) (a : Action P)
    (mutant : c.mutant = "none") (h : BadVocabulary x.g.toBase.toGhost) (nr : NoRestore x a) :
    BadVocabulary (execute c x a).g.toBase.toGhost := by
  change BadVocabulary (project (execute c x a)).g.toGhost
  rw [execute_projects_noRestore9 c x a mutant nr]
  exact vocabulary31_execute9 c (project x) a mutant h

theorem vocabulary_execute32_9 (c : Config P) (x : State P) (a : Action P)
    (mutant : c.mutant = "none") (h : BadVocabulary x.g.toBase.toGhost) :
    BadVocabulary (execute c x a).g.toBase.toGhost := by
  cases a with
  | old a =>
    by_cases delivery : ∃ ok, a = .deliverUp ok
    · obtain ⟨ok,rfl⟩ := delivery
      simp only [execute,deliverUp]
      cases hq : x.s.up with
      | nil => exact h
      | cons m tail =>
        simp only [hq]
        split
        · exact vocabulary_discard9 c x {x.s with up := tail} m ok h
        · exact vocabulary_execute3 c (project x) (.old (.deliverUp ok)) mutant h
    · apply vocabulary_execute_noRestore9 c x (.old a) mutant h
      cases a <;> simp [NoRestore]
      case deliverUp ok => exact False.elim (delivery ⟨ok,rfl⟩)
  | opRename src dst refs rt => exact vocabulary_execute_noRestore9 c x (.opRename src dst refs rt) mutant h trivial
  | opDelete p => exact vocabulary_execute_noRestore9 c x (.opDelete p) mutant h trivial
  | flushDel p => exact vocabulary_execute_noRestore9 c x (.flushDel p) mutant h trivial
  | load p => exact vocabulary_execute_noRestore9 c x (.load p) mutant h trivial
end Storage32
namespace Storage34
open Storage21
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false

theorem apply_slot_free9 (c : Config P) (x : State P) (eff plan : Map P) (dst src : Option (Fin P))
    (mutant : c.mutant = "none") (en : enabledApply c x eff plan dst src = true) :
    claimedCreates x eff plan = false := by
  simp only [enabledApply,Bool.and_eq_true] at en
  simpa [mutant] using en.2

theorem apply_refines33_9 (c : Config P) (x : State P) (eff plan : Map P) (dst src : Option (Fin P))
    (free : claimedCreates x eff plan = false) :
    project (opApply c x eff plan dst src) = Storage33.opApply c (project x) eff plan dst src := by
  simp only [opApply,project,free,tags,Bool.false_eq_true,ite_false,union,List.foldl_nil]

theorem apply_no_claimed_create9 (c : Config P) (x : State P) (eff plan : Map P) (dst src : Option (Fin P))
    (mutant : c.mutant = "none") (en : enabledApply c x eff plan dst src = true) :
    ∀ p ∈ Storage33.keys eff, Storage33.get plan p = absent →
      Storage33.get eff p ≠ absent → p ∉ x.s.claimed := by
  have free := apply_slot_free9 c x eff plan dst src mutant en
  intro p hp expected text claimed
  have mem : x.s.claimed.contains p = true := by simpa using claimed
  have any : claimedCreates x eff plan = true := by
    apply List.any_eq_true.mpr
    exact ⟨p,hp,by simp [expected,text,mem,claimed]⟩
  simp [free] at any

theorem apply_guarantee9 (c : Config P) (x : State P) (eff plan : Map P) (dst src : Option (Fin P))
    (mutant : c.mutant = "none") (en : enabledApply c x eff plan dst src = true)
    (before : guarantee x = true) : guarantee (opApply c x eff plan dst src) = true := by
  unfold guarantee
  rw [apply_refines33_9 c x eff plan dst src (apply_slot_free9 c x eff plan dst src mutant en)]
  have baseEn : Storage33.enabledApply c (project x) eff plan dst src = true := by
    simp only [enabledApply,Bool.and_eq_true] at en; exact en.1
  exact Storage33.apply_guarantee8 c (project x) eff plan dst src mutant baseEn before

theorem vocabulary_step9 (c : Config P) (x : State P) (a : Action P)
    (mutant : c.mutant = "none") (h : BadVocabulary x.g.toBase.toGhost)
    (en : enabled c x a = true) : BadVocabulary (execute c x a).g.toBase.toGhost := by
  cases a with
  | prior a => exact Storage32.vocabulary_execute32_9 c (project x) a mutant h
  | opApply eff plan dst src =>
    have slot := apply_slot_free9 c x eff plan dst src mutant en
    have baseEn : Storage33.enabledApply c (project x) eff plan dst src = true := by
      simp only [enabled,enabledApply,Bool.and_eq_true] at en; exact en.1
    have eq : (execute c x (.opApply eff plan dst src)).g.bad = x.g.bad := by
      change (project (opApply c x eff plan dst src)).g.bad = x.g.bad
      rw [apply_refines33_9 c x eff plan dst src slot]
      exact Storage33.apply_bad8 c (project x) eff plan dst src mutant baseEn
    intro t ht
    exact h t (eq ▸ ht)
  | extClaim p present => exact vocabulary_commit c (Storage3.project (Storage32.project (project x))) _ _ _ h

theorem vocabulary_reachable9 (c : Config P) (disk : Fin P → Int) (x : State P)
    (mutant : c.mutant = "none") (hr : Reachable c disk x) : BadVocabulary x.g.toBase.toGhost := by
  induction hr with
  | initial => simp [BadVocabulary,initWith,embed,Storage33.initWith,Storage32.initWith,Storage32.embed,Storage31.initWith,Storage3.initWith,Storage3.embed,Storage21.init]
  | @advance x y hr a hs ih =>
    unfold step at hs
    split at hs
    · cases hs; exact vocabulary_step9 c x a mutant ih (by assumption)
    · contradiction

theorem name_slot_history9 (c : Config P) (disk : Fin P → Int) (x : State P)
    (mutant : c.mutant = "none") (hr : Reachable c disk x) :
    "name-slot-create-over-claim" ∉ x.g.bad := by
  intro mem
  have kind := vocabulary_reachable9 c disk x mutant hr _ mem
  simp [BadKinds] at kind
#print axioms name_slot_history9
#print axioms apply_guarantee9
end Storage34
