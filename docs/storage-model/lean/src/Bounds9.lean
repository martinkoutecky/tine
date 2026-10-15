import PartialGuarantee9
import Storage33
namespace Storage32
open Storage21 Storage3
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false

theorem bounds_discard32_9 (c : Config P) (x : State P) (sys : Sys P)
    (m : Up P) (ok : Bool) (h : VersionBounds sys.toBase.toSys x.g.vc) :
    VersionBounds (upDiscard c x sys m ok).s.toBase.toSys (upDiscard c x sys m ok).g.vc := by
  simp only [upDiscard]
  split
  · apply bounds_commit
    exact h
  · split
    · apply bounds_commit
      exact bounds_ackMail _ _ _ _ _ _ h (h.pages m.p)
    · apply bounds_commit
      apply bounds_ackMail
      · split
        · have hg := bounds_mono sys.toBase.toSys x.g.vc (x.g.vc+2) h (by omega)
          refine ⟨hg.nonnegative,?_,hg.w,hg.mb,hg.up⟩
          apply property_put _ _ _ (fun pg : Page => pg.ver ≤ x.g.vc+2)
          · apply property_put _ _ _ (fun pg : Page => pg.ver ≤ x.g.vc+2) hg.pages
            dsimp only; omega
          · dsimp only; omega
        · have hg := bounds_mono sys.toBase.toSys x.g.vc (x.g.vc+1) h (by omega)
          refine ⟨hg.nonnegative,?_,hg.w,hg.mb,hg.up⟩
          apply property_put _ _ _ (fun pg : Page => pg.ver ≤ x.g.vc+1) hg.pages
          exact Int.le_refl _
      · split <;> dsimp only <;> omega
theorem bounds31_execute9 (c : Config P) (x : Storage31.State P) (a : Action P)
    (mutant : c.mutant = "none") (h : VersionBounds x.s.toSys x.g.vc) :
    VersionBounds (Storage31.execute c x a).s.toSys (Storage31.execute c x a).g.vc := by
  cases a with
  | load p =>
    apply bounds_commit
    have hg := bounds_mono x.s.toSys x.g.vc (x.g.vc+1) h (by omega)
    refine ⟨hg.nonnegative,?_,hg.w,hg.mb,hg.up⟩
    exact property_put _ _ _ (fun pg : Page => pg.ver ≤ x.g.vc+1) hg.pages (Int.le_refl _)
  | old a => exact bounds_execute4 c x (.old a) mutant h
  | opRename src dst refs rt => exact bounds_execute4 c x (.opRename src dst refs rt) mutant h
  | opDelete p => exact bounds_execute4 c x (.opDelete p) mutant h
  | flushDel p => exact bounds_execute4 c x (.flushDel p) mutant h

theorem bounds_execute_noRestore9 (c : Config P) (x : State P) (a : Action P)
    (mutant : c.mutant = "none") (h : VersionBounds x.s.toBase.toSys x.g.vc) (nr : NoRestore x a) :
    VersionBounds (execute c x a).s.toBase.toSys (execute c x a).g.vc := by
  change VersionBounds (project (execute c x a)).s.toSys (project (execute c x a)).g.vc
  rw [execute_projects_noRestore9 c x a mutant nr]
  exact bounds31_execute9 c (project x) a mutant h

theorem bounds_execute32_9 (c : Config P) (x : State P) (a : Action P)
    (mutant : c.mutant = "none") (h : VersionBounds x.s.toBase.toSys x.g.vc) :
    VersionBounds (execute c x a).s.toBase.toSys (execute c x a).g.vc := by
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
        · apply bounds_discard32_9 c x {x.s with up := tail} m ok
          exact ⟨h.nonnegative,h.pages,h.w,h.mb,by intro u hu; exact h.up u (by simp [hq,hu])⟩
        · exact bounds_execute4 c (project x) (.old (.deliverUp ok)) mutant h
    · apply bounds_execute_noRestore9 c x (.old a) mutant h
      cases a <;> simp [NoRestore]
      case deliverUp ok => exact False.elim (delivery ⟨ok,rfl⟩)
  | opRename src dst refs rt => exact bounds_execute_noRestore9 c x (.opRename src dst refs rt) mutant h trivial
  | opDelete p => exact bounds_execute_noRestore9 c x (.opDelete p) mutant h trivial
  | flushDel p => exact bounds_execute_noRestore9 c x (.flushDel p) mutant h trivial
  | load p => exact bounds_execute_noRestore9 c x (.load p) mutant h trivial

theorem bounds_reachable32_9 (c : Config P) (disk : Fin P → Int) (x : State P)
    (mutant : c.mutant = "none") (hr : Reachable c disk x) :
    VersionBounds x.s.toBase.toSys x.g.vc := by
  induction hr with
  | initial => constructor <;> simp [initWith,embed,Storage31.initWith,Storage3.initWith,Storage3.embed,Storage21.init]
  | @advance x y hr a hs ih =>
    unfold step at hs; split at hs
    · cases hs; exact bounds_execute32_9 c x a mutant ih
    · contradiction
#print axioms bounds_discard32_9
#print axioms bounds_reachable32_9
end Storage32

namespace Storage33
open Storage21 Storage3
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false

theorem bounds_apply33_9 (c : Config P) (x : State P) (eff plan : Map P) (dst src : Option (Fin P))
    (h : VersionBounds x.s.toBase.toSys x.g.vc) :
    VersionBounds (opApply c x eff plan dst src).s.toBase.toSys (opApply c x eff plan dst src).g.vc := by
  simp only [opApply,Storage32.commit,Storage3.commit,Storage3.project,Storage32.project]
  apply bounds_commit
  have hg := bounds_mono x.s.toBase.toSys x.g.vc (x.g.vc+P) h (by omega)
  have fresh : ∀ p : Fin P, x.g.vc+1+p.val ≤ x.g.vc+P := by intro p; have hp := p.isLt; omega
  refine ⟨hg.nonnegative,?_,hg.w,hg.mb,hg.up⟩
  apply fold_put_property _ _ _ (fun pg : Page => pg.ver ≤ x.g.vc+P) hg.pages
  intro p
  simp [opEdit]
  exact fresh p

theorem bounds_reachable33_9 (c : Config P) (disk : Fin P → Int) (x : State P)
    (mutant : c.mutant = "none") (hr : Reachable c disk x) :
    VersionBounds x.s.toBase.toSys x.g.vc := by
  induction hr with
  | initial => constructor <;> simp [initWith,Storage32.initWith,Storage32.embed,Storage31.initWith,Storage3.initWith,Storage3.embed,Storage21.init]
  | @advance x y hr a hs ih =>
    unfold step at hs; split at hs
    · cases hs
      cases a with
      | prior a => exact Storage32.bounds_execute32_9 c x a mutant ih
      | opApply eff plan dst src => exact bounds_apply33_9 c x eff plan dst src ih
    · contradiction
end Storage33
