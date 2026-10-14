import Load31
import Conservativity31
namespace Storage31
open Storage21 Storage3
set_option maxRecDepth 8192
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false

theorem inv_execute (c : Config P) (x : State P) (a : Storage31.Action P)
    (mutant : c.mutant = "none") (noNone : NoNone6 c) (noUnknown : NoUnknown6 c)
    (h : Inv c x) (hen : Storage31.enabled c x a = true) :
    Inv c (Storage31.execute c x a) := by
  cases a with
  | load p => exact inv_load c x p h hen
  | old a => exact inv_baseline c x (.old a) mutant noNone noUnknown h hen
  | opRename src dst refs rt =>
    exact inv_baseline c x (.opRename src dst refs rt) mutant noNone noUnknown h
      (baseline_enabled c x (.opRename src dst refs rt) (by simp) hen)
  | opDelete p =>
    exact inv_baseline c x (.opDelete p) mutant noNone noUnknown h
      (baseline_enabled c x (.opDelete p) (by simp) hen)
  | flushDel p => exact inv_baseline c x (.flushDel p) mutant noNone noUnknown h hen

theorem inv_reachable7 (c : Config P) (disk : Fin P → Int) (x : State P)
    (mutant : c.mutant = "none") (realDisk : ∀ p, RealByte c (disk p))
    (noNone : NoNone6 c) (noUnknown : NoUnknown6 c) (hr : Storage31.Reachable c disk x) : Inv c x := by
  induction hr with
  | initial => exact inv_initial c disk mutant realDisk noUnknown
  | @advance x y hr a hs ih =>
    unfold Storage31.step at hs; split at hs
    · cases hs; exact inv_execute c x a mutant noNone noUnknown ih (by assumption)
    · contradiction

theorem guarantee_of_inv7 (c : Config P) (x : State P) (h : Inv c x) :
    Storage31.guarantee x = true := by
  have bad : x.g.bad = [] := by
    apply List.eq_nil_iff_forall_not_mem.mpr
    intro tag mem
    have kind := h.vocabulary tag mem
    simp [BadKinds] at kind
    rcases kind with rfl | rfl | rfl | rfl | rfl | rfl | rfl | rfl
    all_goals first | exact h.B mem | exact h.W mem | exact h.taken mem | exact h.C.1 mem |
      exact h.G mem | exact h.unheld mem | exact h.newTags.1 mem | exact h.newTags.2 mem
  have A := noLoss_of_custody c (project x) h.custody
  have accepted := accepted_of_carriage (project x) h.protocol.carriage
  have trash : Storage3.trashed x = true := by
    apply List.all_eq_true.mpr
    intro p _
    apply List.all_eq_true.mpr
    intro b hb
    simp only [Bool.or_eq_true,List.contains_iff_mem]
    exact Or.inl (h.trash.1 p b hb)
  change Storage3.guarantee x = true
  unfold Storage3.guarantee
  simp only [Storage3.noLoss,Storage3.accepted,A,accepted,trash,Bool.true_and,Bool.and_true]
  simp [Storage3.transitions,Storage21.transitions,project,bad]

theorem guarantee_positiveTexts7 (c : Config P) (T : Nat)
    (texts : c.texts = positiveTexts4 T) (two : 2 ≤ T) (mutant : c.mutant = "none")
    (x : State P) (hr : Storage31.Reachable c (initDisk c) x) : Storage31.guarantee x = true :=
  guarantee_of_inv7 c x (inv_reachable7 c (initDisk c) x mutant
    (positiveTexts_initial_real4 c T texts two) (positiveTexts_noNone6 c T texts)
    (positiveTexts_noUnknown6 c T texts) hr)

#print guarantee_positiveTexts7
#print axioms guarantee_positiveTexts7
#print axioms initialized_old_runs_exact7
#print axioms infinite_old_runs_exact7
end Storage31
