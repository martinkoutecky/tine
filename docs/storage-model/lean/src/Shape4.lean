import BLaunch3
import CoreLineage21
namespace Storage3
open Storage21
set_option maxRecDepth 8192
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false

theorem fold_put_if (pages : Fin P → α) (refs : List (Fin P)) (nr : Fin P → α)
    (p : Fin P) : refs.foldl (fun acc r => put acc r (nr r)) pages p =
    if p ∈ refs then nr p else pages p := by
  induction refs generalizing pages with
  | nil => simp
  | cons r tail ih =>
    rw [List.foldl_cons,ih]
    by_cases ht : p ∈ tail <;> by_cases he : p = r <;> simp [ht,he,put]

theorem fold_put_property (pages : Fin P → α) (refs : List (Fin P)) (nr : Fin P → α)
    (pr : α → Prop) (h : ∀ p, pr (pages p)) (hn : ∀ p, pr (nr p)) :
    ∀ p, pr (refs.foldl (fun acc r => put acc r (nr r)) pages p) := by
  intro p; rw [fold_put_if]; split
  · exact hn p
  · exact h p

theorem old_sys4 (c : Config P) (x : State P) (a : Storage21.Action P)
    (mutant : c.mutant = "none") :
    (project (execute c x (.old a))).s = (Storage21.execute c (project x) a).s := by
  cases a <;> simp [execute,executeOld,mutant,project,lift,rename,dirSync,power,commit,
    Storage21.execute,Storage21.rename,Storage21.commit]

theorem old_promise4 (c : Config P) (x : State P) (a : Storage21.Action P)
    (mutant : c.mutant = "none") :
    (execute c x (.old a)).g.promise = (Storage21.execute c (project x) a).g.promise := by
  cases a <;> simp [execute,executeOld,mutant,project,lift,rename,dirSync,power,commit,
    Storage21.execute,Storage21.rename,Storage21.commit]

theorem old_vc4 (c : Config P) (x : State P) (a : Storage21.Action P)
    (mutant : c.mutant = "none") :
    (execute c x (.old a)).g.vc = (Storage21.execute c (project x) a).g.vc := by
  cases a <;> simp [execute,executeOld,mutant,project,lift,rename,dirSync,power,commit,
    Storage21.execute,Storage21.rename,Storage21.commit]

theorem opDelete_unlocked4 (c : Config P) (x : State P) (p : Fin P)
    (hen : enabled c x (.opDelete p) = true) (hon : x.s.job.on = true) : x.s.job.p ≠ p := by
  simp [enabled,opFree,hon] at hen
  grind

theorem opRename_unlocked4 (c : Config P) (x : State P) (src dst : Fin P)
    (refs : List (Fin P)) (rt : Fin P → Int)
    (hen : enabled c x (.opRename src dst refs rt) = true) (hon : x.s.job.on = true) :
    x.s.job.p ∉ refs ∧ (opCur x.s src ≠ absent → x.s.job.p ≠ src ∧ x.s.job.p ≠ dst) := by
  have free : opFree x.s (union (if opCur x.s src != absent then [src,dst] else []) refs) = true := by
    simp only [enabled,Bool.and_eq_true] at hen; grind
  simp only [opFree,hon,Bool.true_and,Bool.not_eq_true,List.contains_eq_mem] at free
  have no : x.s.job.p ∉ union (if opCur x.s src != absent then [src,dst] else []) refs := by simpa using free
  have pair : (opCur x.s src ≠ absent → x.s.job.p ≠ src ∧ x.s.job.p ≠ dst) ∧ x.s.job.p ∉ refs := by
    simpa [mem_union_fin] using no
  exact ⟨pair.2,pair.1⟩

theorem opDelete_snapshot4 (c : Config P) (x : State P) (p : Fin P)
    (h : JobSnapshot x.s.toSys) (hen : enabled c x (.opDelete p) = true) :
    JobSnapshot (opDelete c x p).s.toSys := by
  intro hon
  change x.s.job.on = true at hon
  have ne := opDelete_unlocked4 c x p hen hon
  simpa [opDelete,commit,Storage21.commit,project,put,ne] using h hon

theorem opRename_snapshot4 (c : Config P) (x : State P) (src dst : Fin P)
    (refs : List (Fin P)) (rt : Fin P → Int)
    (h : JobSnapshot x.s.toSys) (hen : enabled c x (.opRename src dst refs rt) = true) :
    JobSnapshot (opRename c x src dst refs rt).s.toSys := by
  intro hon
  change x.s.job.on = true at hon
  have ne := opRename_unlocked4 c x src dst refs rt hen hon
  simp only [opRename,commit,Storage21.commit,project]
  rw [fold_put_not_mem _ refs _ _ ne.1]
  by_cases full : opCur x.s src = absent
  · simpa [full] using h hon
  · simpa [full,put,(ne.2 full).1,(ne.2 full).2] using h hon

theorem snapshot_execute4 (c : Config P) (x : State P) (a : Action P)
    (mutant : c.mutant = "none") (h : JobSnapshot x.s.toSys)
    (hi : InactivePages (project x)) (hen : enabled c x a = true) :
    JobSnapshot (execute c x a).s.toSys := by
  cases a with
  | old a => rw [show (execute c x (.old a)).s.toSys = (Storage21.execute c (project x) a).s from old_sys4 c x a mutant]
             exact snapshot_execute c mutant (project x) a h hi hen
  | opDelete p => exact opDelete_snapshot4 c x p h hen
  | opRename src dst refs rt => exact opRename_snapshot4 c x src dst refs rt h hen
  | flushDel p =>
    intro _
    have hh : (x.s.pages p).held = true := by simp [enabled] at hen; grind
    have hd : dirty (x.s.pages p) = true := by simp [enabled] at hen; grind
    exact ⟨hh,rfl,rfl,rfl,hd⟩

theorem snapshot_reachable4 (c : Config P) (disk : Fin P → Int) (x : State P)
    (mutant : c.mutant = "none") (hr : Reachable c disk x) : JobSnapshot x.s.toSys := by
  induction hr with
  | initial => simp [JobSnapshot,initWith,embed,project,Storage21.init,nojob]
  | @advance x y hr a hs ih =>
    unfold step at hs; split at hs
    · cases hs; exact snapshot_execute4 c x a mutant ih (inactive_reachable3 c disk x mutant hr) (by assumption)
    · contradiction

theorem shapes_execute4 (c : Config P) (x : State P) (a : Action P)
    (mutant : c.mutant = "none") (h : Shapes x.s.toSys)
    (hj : JobSnapshot x.s.toSys) (hen : enabled c x a = true) :
    Shapes (execute c x a).s.toSys := by
  cases a with
  | old a => rw [show (execute c x (.old a)).s.toSys = (Storage21.execute c (project x) a).s from old_sys4 c x a mutant]
             exact shapes_execute c mutant (project x) a h hj hen
  | opDelete p =>
    simp only [execute,opDelete,commit,project,shapes_commit]
    exact property_put _ _ _ PageShape h (by simp [PageShape,opEdit])
  | opRename src dst refs rt =>
    simp only [execute,opRename,commit,project,shapes_commit]
    apply fold_put_property _ _ _ PageShape
    · split
      · exact property_put _ _ _ PageShape
          (property_put _ _ _ PageShape h (by simp [PageShape,opEdit])) (by simp [PageShape,opEdit])
      · exact h
    · intro p; simp [PageShape,opEdit]
  | flushDel p => exact h

theorem shapes_reachable4 (c : Config P) (disk : Fin P → Int) (x : State P)
    (mutant : c.mutant = "none") (hr : Reachable c disk x) : Shapes x.s.toSys := by
  induction hr with
  | initial => simp [Shapes,PageShape,initWith,embed,project,Storage21.init]
  | @advance x y hr a hs ih =>
    unfold step at hs; split at hs
    · cases hs; exact shapes_execute4 c x a mutant ih (snapshot_reachable4 c disk x mutant hr) (by assumption)
    · contradiction
end Storage3
