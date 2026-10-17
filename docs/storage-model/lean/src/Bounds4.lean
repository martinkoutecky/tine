import Shape4
namespace Storage3
open Storage21
set_option maxRecDepth 8192
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false

theorem bounds_delete4 (c : Config P) (x : State P) (p : Fin P)
    (h : VersionBounds x.s.toSys x.g.vc) :
    VersionBounds (opDelete c x p).s.toSys (opDelete c x p).g.vc := by
  simp only [opDelete,commit,project]
  apply bounds_commit
  have hg := bounds_mono x.s.toSys x.g.vc (x.g.vc+1) h (by omega)
  refine ⟨hg.nonnegative,?_,hg.w,hg.mb,hg.up⟩
  exact property_put _ _ _ (fun pg : Page => pg.ver ≤ x.g.vc+1) hg.pages (by simp [opEdit])

theorem bounds_opRename4 (c : Config P) (x : State P) (src dst : Fin P)
    (refs : List (Fin P)) (rt : Fin P → Int)
    (h : VersionBounds x.s.toSys x.g.vc) :
    VersionBounds (opRename c x src dst refs rt).s.toSys (opRename c x src dst refs rt).g.vc := by
  simp only [opRename,commit,project]
  apply bounds_commit
  have hg := bounds_mono x.s.toSys x.g.vc (x.g.vc+P) h (by omega)
  have fresh : ∀ p : Fin P, x.g.vc+1+p.val ≤ x.g.vc+P := by intro p; have hp := p.isLt; omega
  refine ⟨hg.nonnegative,?_,hg.w,hg.mb,hg.up⟩
  apply fold_put_property _ _ _ (fun pg : Page => pg.ver ≤ x.g.vc+P)
  · split
    · exact property_put _ _ _ (fun pg : Page => pg.ver ≤ x.g.vc+P)
        (property_put _ _ _ (fun pg : Page => pg.ver ≤ x.g.vc+P) hg.pages
          (by simpa [opEdit] using fresh dst)) (by simpa [opEdit] using fresh src)
    · exact hg.pages
  · intro p; simpa [opEdit] using fresh p

theorem bounds_execute4 (c : Config P) (x : State P) (a : Action P)
    (mutant : c.mutant = "none") (h : VersionBounds x.s.toSys x.g.vc) :
    VersionBounds (execute c x a).s.toSys (execute c x a).g.vc := by
  cases a with
  | old a =>
    rw [show (execute c x (.old a)).s.toSys = (Storage21.execute c (project x) a).s from old_sys4 c x a mutant,
      old_vc4 c x a mutant]
    exact bounds_execute c (project x) a h
  | opDelete p => exact bounds_delete4 c x p h
  | opRename src dst refs rt => exact bounds_opRename4 c x src dst refs rt h
  | flushDel p => exact bounds_execute c (project x) (.flush p) h

theorem bounds_reachable4 (c : Config P) (disk : Fin P → Int) (x : State P)
    (mutant : c.mutant = "none") (hr : Reachable c disk x) : VersionBounds x.s.toSys x.g.vc := by
  induction hr with
  | initial => constructor <;> simp [initWith,embed,project,Storage21.init]
  | @advance x y hr a hs ih =>
    unfold step at hs; split at hs
    · cases hs; exact bounds_execute4 c x a mutant ih
    · contradiction

theorem clear_execute4 (c : Config P) (x : State P) (a : Action P)
    (mutant : c.mutant = "none") (h : JobClear x.s.toSys)
    (hj : JobSnapshot x.s.toSys) (hi : InactivePages (project x))
    (hen : enabled c x a = true) : JobClear (execute c x a).s.toSys := by
  cases a with
  | old a =>
    rw [show (execute c x (.old a)).s.toSys = (Storage21.execute c (project x) a).s from old_sys4 c x a mutant]
    exact jobClear_execute c mutant (project x) a h hj hi hen
  | opDelete p =>
    intro hon
    change x.s.job.on = true at hon
    have ne := opDelete_unlocked4 c x p hen hon
    simpa [execute,opDelete,commit,Storage21.commit,project,put,ne] using h hon
  | opRename src dst refs rt =>
    intro hon
    change x.s.job.on = true at hon
    have ne := opRename_unlocked4 c x src dst refs rt hen hon
    simp only [execute,opRename,commit,Storage21.commit,project]
    rw [fold_put_not_mem _ refs _ _ ne.1]
    by_cases full : opCur x.s src = absent
    · simpa [full] using h hon
    · simpa [full,put,(ne.2 full).1,(ne.2 full).2] using h hon
  | flushDel p => intro _; change (x.s.pages p).conflict = false; simp [enabled] at hen; grind

theorem clear_reachable4 (c : Config P) (disk : Fin P → Int) (x : State P)
    (mutant : c.mutant = "none") (hr : Reachable c disk x) : JobClear x.s.toSys := by
  induction hr with
  | initial => simp [JobClear,initWith,embed,project,Storage21.init,nojob]
  | @advance x y hr a hs ih =>
    unfold step at hs; split at hs
    · cases hs; exact clear_execute4 c x a mutant ih (snapshot_reachable4 c disk x mutant hr)
        (inactive_reachable3 c disk x mutant hr) (by assumption)
    · contradiction
end Storage3
