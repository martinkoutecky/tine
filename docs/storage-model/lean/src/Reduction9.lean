import Slot9
import IdleClosure9
import Conservativity9
import Partial8
namespace Storage34
open Storage21
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false

-- Claims strengthen admission and otherwise erase to stutters. This is a
-- forward simulation of actual initialized histories, including extClaim.
theorem projects_reachable33_9 (c : Config P) (disk : Fin P → Int) (x : State P)
    (mutant : c.mutant = "none") (hr : Reachable c disk x) :
    Storage33.Reachable c disk (project x) := by
  induction hr with
  | initial => exact .initial
  | @advance x y hr a hs ih =>
    unfold step at hs
    split at hs
    · cases hs
      have en : enabled c x a = true := by assumption
      cases a with
      | prior a =>
        exact .advance ih (.prior a) (by simp only [Storage33.step,Storage33.enabled,show Storage32.enabled c (project x) a = true from en]; rfl)
      | opApply eff plan dst src =>
        change Storage33.Reachable c disk (project (opApply c x eff plan dst src))
        have free := apply_slot_free9 c x eff plan dst src mutant en
        rw [apply_refines33_9 c x eff plan dst src free]
        have baseEn : Storage33.enabledApply c (project x) eff plan dst src = true := by
          simp only [enabled,enabledApply,Bool.and_eq_true] at en; exact en.1
        exact .advance ih (.opApply eff plan dst src) (by simp [Storage33.step,Storage33.enabled,baseEn,Storage33.execute])
      | extClaim p present =>
        change Storage33.Reachable c disk (project (extClaim c x p present))
        rw [extClaim_projects_idle9 c x p present (idle_closed_reachable34_9 c disk x mutant hr)]
        exact ih
    · contradiction

theorem guarantee_universal_iff_s33_9 (c : Config P) (disk : Fin P → Int) (mutant : c.mutant = "none") :
    (∀ x,Reachable c disk x → guarantee x = true) ↔
      (∀ x,Storage33.Reachable c disk x → Storage33.guarantee x = true) := by
  constructor
  · intro h x hr; exact h (embed x) (noClaim_reachable_exact9 c disk x hr)
  · intro h x hr; exact h (project x) (projects_reachable33_9 c disk x mutant hr)
theorem order_universal_iff_s33_9 (c : Config P) (disk : Fin P → Int) (mutant : c.mutant = "none") :
    (∀ x,Reachable c disk x → orderHolds x = true) ↔
      (∀ x,Storage33.Reachable c disk x → Storage33.orderHolds x = true) := by
  constructor
  · intro h x hr; exact h (embed x) (noClaim_reachable_exact9 c disk x hr)
  · intro h x hr; exact h (project x) (projects_reachable33_9 c disk x mutant hr)
theorem resolution_universal_iff_s33_9 (c : Config P) (disk : Fin P → Int) (mutant : c.mutant = "none") :
    (∀ x,Reachable c disk x → renameResolves x = true) ↔
      (∀ x,Storage33.Reachable c disk x → Storage33.renameResolves x = true) := by
  constructor
  · intro h x hr; exact h (embed x) (noClaim_reachable_exact9 c disk x hr)
  · intro h x hr; exact h (project x) (projects_reachable33_9 c disk x mutant hr)

-- Restriction for the nontrivial s3.2 guarantee transfer: claims are allowed,
-- full opRename is allowed; opApply and the extra source restoration are not.
def NoApplyRestore (x : State P) : Action P → Prop
  | .prior a => Storage32.NoRestore (project x) a
  | .opApply _ _ _ _ => False
  | .extClaim _ _ => True
inductive ReachableNoApplyRestore (c : Config P) (disk : Fin P → Int) : State P → Prop where
  | initial : ReachableNoApplyRestore c disk (initWith c disk)
  | advance {x y} : ReachableNoApplyRestore c disk x → (a : Action P) →
      NoApplyRestore x a → step c x a = some y → ReachableNoApplyRestore c disk y

theorem noApplyRestore_actual_reachable9 (c : Config P) (disk : Fin P → Int) (x : State P)
    (hr : ReachableNoApplyRestore c disk x) : Reachable c disk x := by
  induction hr with
  | initial => exact .initial
  | advance hr a _ hs ih => exact .advance ih a hs

theorem noApplyRestore_projects32_9 (c : Config P) (disk : Fin P → Int) (x : State P)
    (mutant : c.mutant = "none") (hr : ReachableNoApplyRestore c disk x) :
    Storage32.ReachableNoRestore c disk (project x) := by
  induction hr with
  | initial => exact .initial
  | @advance x y hr a nr hs ih =>
    unfold step at hs; split at hs
    · cases hs
      have en : enabled c x a = true := by assumption
      cases a with
      | prior a => exact .advance ih a nr (by simp only [Storage32.step,show Storage32.enabled c (project x) a = true from en]; rfl)
      | opApply eff plan dst src => exact False.elim nr
      | extClaim p present =>
        change Storage32.ReachableNoRestore c disk (project (extClaim c x p present))
        rw [extClaim_projects_idle9 c x p present (idle_closed_reachable34_9 c disk x mutant (noApplyRestore_actual_reachable9 c disk x hr))]
        exact ih
    · contradiction

theorem noApplyRestore_guarantee34_9 (c : Config P) (T : Nat)
    (texts : c.texts = Storage3.positiveTexts4 T) (two : 2 ≤ T) (mutant : c.mutant = "none")
    (x : State P) (hr : ReachableNoApplyRestore c (Storage31.initDisk c) x) : guarantee x = true :=
  Storage32.noRestore_guarantee9 c T texts two mutant (project x) (noApplyRestore_projects32_9 c _ x mutant hr)

def NoMove (x : State P) : Action P → Prop
  | .prior a => Storage32.NoFullRename (Storage32.project (project x)) a
  | .opApply _ _ dst _ => dst = none
  | .extClaim _ _ => True
inductive ReachableNoMove (c : Config P) (disk : Fin P → Int) : State P → Prop where
  | initial : ReachableNoMove c disk (initWith c disk)
  | advance {x y} : ReachableNoMove c disk x → (a : Action P) →
      NoMove x a → step c x a = some y → ReachableNoMove c disk y

theorem noMove_actual_reachable9 (c : Config P) (disk : Fin P → Int) (x : State P)
    (hr : ReachableNoMove c disk x) : Reachable c disk x := by
  induction hr with
  | initial => exact .initial
  | advance hr a _ hs ih => exact .advance ih a hs

theorem noMove_projects33_9 (c : Config P) (disk : Fin P → Int) (x : State P)
    (mutant : c.mutant = "none") (hr : ReachableNoMove c disk x) :
    Storage33.ReachableNoMove c disk (project x) := by
  induction hr with
  | initial => exact .initial
  | @advance x y hr a nm hs ih =>
    unfold step at hs; split at hs
    · cases hs
      have en : enabled c x a = true := by assumption
      cases a with
      | prior a => exact .advance ih (.prior a) nm (by simp only [Storage33.step,Storage33.enabled,show Storage32.enabled c (project x) a = true from en]; rfl)
      | opApply eff plan dst src =>
        change Storage33.ReachableNoMove c disk (project (opApply c x eff plan dst src))
        rw [apply_refines33_9 c x eff plan dst src (apply_slot_free9 c x eff plan dst src mutant en)]
        have baseEn : Storage33.enabledApply c (project x) eff plan dst src = true := by
          simp only [enabled,enabledApply,Bool.and_eq_true] at en; exact en.1
        exact .advance ih (.opApply eff plan dst src) nm (by simp [Storage33.step,Storage33.enabled,baseEn,Storage33.execute])
      | extClaim p present =>
        change Storage33.ReachableNoMove c disk (project (extClaim c x p present))
        rw [extClaim_projects_idle9 c x p present (idle_closed_reachable34_9 c disk x mutant (noMove_actual_reachable9 c disk x hr))]
        exact ih
    · contradiction

theorem noMove_order_resolves34_9 (c : Config P) (disk : Fin P → Int) (x : State P)
    (mutant : c.mutant = "none") (hr : ReachableNoMove c disk x) :
    orderHolds x = true ∧ renameResolves x = true :=
  (Storage33.noMove_order_resolves8 c disk (project x) mutant (noMove_projects33_9 c disk x mutant hr)).2
#print axioms projects_reachable33_9
#print axioms guarantee_universal_iff_s33_9
#print axioms order_universal_iff_s33_9
#print axioms resolution_universal_iff_s33_9
#print axioms noApplyRestore_guarantee34_9
#print axioms noMove_order_resolves34_9
end Storage34
