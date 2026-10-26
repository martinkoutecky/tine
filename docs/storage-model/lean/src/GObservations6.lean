import CoreObs6
import AClosure6
namespace Storage3
open Storage21
set_option maxRecDepth 8192
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false

theorem obs_opEdit6 (c : Config P) (s : Sys P) (p : Fin P) (t v : Int)
    (disk : ∀ p, ObservedByte c (s.disk p)) (pg : ∀ p, ObservedByte c (s.pages p).obs) :
    ObservedByte c (opEdit s p t v).obs := by
  simp only [opEdit]; split
  · exact pg p
  · exact disk p

theorem obs_execute6 (c : Config P) (x : State P) (a : Action P)
    (mutant : c.mutant = "none") (h : ObsOnly c x.s.toSys) (bytes : ByteDomain c x.s.toSys)
    (hen : enabled c x a = true) : ObsOnly c (execute c x a).s.toSys := by
  cases a with
  | old a => rw [old_sys6 c x a mutant]; exact obsOnly_execute c (project x) a h bytes hen
  | opDelete p =>
    simp only [execute,opDelete,commit,project]
    apply obsOnly_commit
    refine ⟨h.1,h.2.1,?_,h.2.2.2.1,h.2.2.2.2⟩
    exact property_put _ _ _ (fun pg : Page => ObservedByte c pg.obs) h.2.2.1 (obs_opEdit6 c x.s p absent (x.g.vc+1) h.1 h.2.2.1)
  | opRename src dst refs rt =>
    simp only [execute,opRename,commit,project]
    apply obsOnly_commit
    refine ⟨h.1,h.2.1,?_,h.2.2.2.1,h.2.2.2.2⟩
    apply fold_put_property _ _ _ (fun pg : Page => ObservedByte c pg.obs)
    · split
      · exact property_put _ _ _ (fun pg : Page => ObservedByte c pg.obs)
          (property_put _ _ _ (fun pg : Page => ObservedByte c pg.obs) h.2.2.1 (obs_opEdit6 c x.s dst (opCur x.s src) (x.g.vc+1+dst.val) h.1 h.2.2.1))
          (obs_opEdit6 c x.s src absent (x.g.vc+1+src.val) h.1 h.2.2.1)
      · exact h.2.2.1
    · intro p; exact obs_opEdit6 c x.s p (rt p) (x.g.vc+1+p.val) h.1 h.2.2.1
  | flushDel p => exact obsOnly_commit c (project x) _ x.g.toGhost "flushDel" h

theorem obs_reachable6 (c : Config P) (disk : Fin P → Int) (x : State P)
    (mutant : c.mutant = "none") (realDisk : ∀ p, RealByte c (disk p))
    (hr : Reachable c disk x) : ObsOnly c x.s.toSys := by
  induction hr with
  | initial =>
    refine ⟨fun p => real_observed (realDisk p),fun p => real_observed (realDisk p),?_,?_,?_⟩
    all_goals simp [initWith,embed,Storage21.init,ObservedByte]
  | @advance x y hr a hs ih =>
    unfold step at hs; split at hs
    · cases hs; exact obs_execute6 c x a mutant ih (bytes_reachable4 c disk x mutant realDisk hr) (by assumption)
    · contradiction
end Storage3
