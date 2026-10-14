import CoreActive6
import CoreFence6
import CorePromiseBytes6
import ExactResidual5
namespace Storage3
open Storage21
set_option maxRecDepth 8192
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false

theorem old_sys6 (c : Config P) (x : State P) (a : Storage21.Action P)
    (mutant : c.mutant = "none") :
    (execute c x (.old a)).s.toSys = (Storage21.execute c (project x) a).s := old_sys4 c x a mutant

theorem old_pages6 (c : Config P) (x : State P) (a : Storage21.Action P)
    (mutant : c.mutant = "none") :
    (execute c x (.old a)).s.pages = (Storage21.execute c (project x) a).s.pages :=
  congrArg Storage21.Sys.pages (old_sys6 c x a mutant)

theorem old_ext6 (c : Config P) (x : State P) (a : Storage21.Action P)
    (mutant : c.mutant = "none") :
    (execute c x (.old a)).g.ext = (Storage21.execute c (project x) a).g.ext := by
  cases a <;> simp [execute,executeOld,mutant,project,lift,rename,dirSync,power,commit,
    Storage21.execute,Storage21.rename,Storage21.commit]

theorem old_wrote6 (c : Config P) (x : State P) (a : Storage21.Action P)
    (mutant : c.mutant = "none") :
    (execute c x (.old a)).g.wrote = (Storage21.execute c (project x) a).g.wrote := by
  cases a <;> simp [execute,executeOld,mutant,project,lift,rename,dirSync,power,commit,
    Storage21.execute,Storage21.rename,Storage21.commit]

theorem fold_ack_epochs6 (es : List (Fin P × Int)) (pr : Fin P → Promise)
    (v : Fin P → Int) (ext : Fin P → Int) (h : ∀ p, (pr p).ep ≤ ext p)
    (nonneg : ∀ p, 0 ≤ ext p) :
    ∀ p, (es.foldl (fun acc e => put acc e.1 (ack (acc e.1) e.2 (v e.1) false 0)) pr p).ep ≤ ext p := by
  induction es generalizing pr with
  | nil => exact h
  | cons e tail ih =>
    apply ih
    intro p; by_cases eq : p = e.1
    · subst p; simp only [put,ite_true]; exact ack_epoch_le _ _ _ _ _ _ (h _) (nonneg _)
    · simpa [put,eq] using h p

theorem fold_ack_unchanged6 (es : List (Fin P × Int)) (pr : Fin P → Promise)
    (v : Fin P → Int) (p : Fin P) (mem : p ∉ es.map Prod.fst) :
    es.foldl (fun acc e => put acc e.1 (ack (acc e.1) e.2 (v e.1) false 0)) pr p = pr p := by
  induction es generalizing pr with
  | nil => rfl
  | cons e tail ih =>
    have ne : p ≠ e.1 := by intro eq; apply mem; simp [eq]
    have nt : p ∉ tail.map Prod.fst := by intro ht; apply mem; simp [ht]
    rw [List.foldl_cons,ih _ nt]
    simp [put,ne]

theorem epochs_execute6 (c : Config P) (x : State P) (a : Action P)
    (mutant : c.mutant = "none") (h : EpochBounds x.s.toSys x.g.toGhost) :
    EpochBounds (execute c x a).s.toSys (execute c x a).g.toGhost := by
  cases a with
  | old a =>
    have core := epochs_execute c (project x) a h
    refine ⟨?_,?_,?_⟩
    · simpa only [old_ext6 c x a mutant] using core.ext
    · simpa only [old_promise4 c x a mutant,old_ext6 c x a mutant] using core.promise
    · change (execute c x (.old a)).s.toSys.job.ep ≤ (execute c x (.old a)).g.ext (execute c x (.old a)).s.toSys.job.p
      rw [old_sys6 c x a mutant,old_ext6 c x a mutant]
      exact core.job
  | opDelete p =>
    simp only [execute,opDelete,commit,project]
    apply epochs_commit
    refine ⟨h.ext,?_,h.job⟩
    intro q; split
    · exact h.promise q
    · by_cases eq : q = p
      · subst q; simp only [put,ite_true]; exact ack_epoch_le _ _ _ _ _ _ (h.promise _) (h.ext _)
      · simpa [put,eq] using h.promise q
  | opRename src dst refs rt =>
    simp only [execute,opRename,commit,project]
    apply epochs_commit
    refine ⟨h.ext,?_,h.job⟩
    exact fold_ack_epochs6 _ x.g.promise (fun p => x.g.vc+1+p.val) x.g.ext h.promise h.ext
  | flushDel p => exact epochs_execute c (project x) (.flush p) h

theorem epochs_reachable6 (c : Config P) (disk : Fin P → Int) (x : State P)
    (mutant : c.mutant = "none") (hr : Reachable c disk x) :
    EpochBounds x.s.toSys x.g.toGhost := by
  induction hr with
  | initial => constructor <;> simp [initWith,embed,Storage21.init,nojob]
  | @advance x y hr a hs ih =>
    unfold step at hs; split at hs
    · cases hs; exact epochs_execute6 c x a mutant ih
    · contradiction

theorem distinct_execute6 (c : Config P) (x : State P) (a : Action P)
    (mutant : c.mutant = "none") (h : QueueDistinct x.s.toSys)
    (hen : enabled c x a = true) : QueueDistinct (execute c x a).s.toSys := by
  cases a with
  | old a => rw [old_sys6 c x a mutant]; exact distinct_execute c (project x) a h hen
  | opDelete p => exact h
  | opRename src dst refs rt => exact h
  | flushDel p => exact h

theorem distinct_reachable6 (c : Config P) (disk : Fin P → Int) (x : State P)
    (mutant : c.mutant = "none") (hr : Reachable c disk x) : QueueDistinct x.s.toSys := by
  induction hr with
  | initial => simp [QueueDistinct,initWith,embed,Storage21.init]
  | @advance x y hr a hs ih =>
    unfold step at hs; split at hs
    · cases hs; exact distinct_execute6 c x a mutant ih (by assumption)
    · contradiction

theorem rename_promise_match6 (c : Config P) (x : State P) (src dst : Fin P)
    (refs : List (Fin P)) (rt : Fin P → Int) (lineage : PromiseLineage (project x))
    (hen : enabled c x (.opRename src dst refs rt) = true) (p : Fin P)
    (mem : p ∈ (renameGot4 x src dst refs rt).map Prod.fst) :
    renamePromise4 x src dst refs rt p =
      ⟨true,(renamePages4 x src dst refs rt p).buf,x.g.vc+1+p.val,false,0⟩ := by
  have fg := fold_ack_point4 (renameGot4 x src dst refs rt) x.g.promise
    (fun p => x.g.vc+1+p.val) p (renamePages4 x src dst refs rt p).buf
    (by intro e he eq; have hp := (rename_got_shape4 c x src dst refs rt hen e he).1; simpa [eq] using hp.symm)
  have old := lineage.1 p
  change (x.g.promise p).ver ≤ x.g.vc at old
  simpa [renamePromise4,mem,ack_newer _ _ _ false 0 (by omega : (x.g.promise p).ver < x.g.vc+1+p.val)] using fg

theorem active_delete6 (c : Config P) (x : State P) (p : Fin P)
    (mutant : c.mutant = "none") (h : ActiveAgreement x.s.toSys x.g.toGhost)
    (lineage : PromiseLineage (project x)) :
    ActiveAgreement (opDelete c x p).s.toSys (opDelete c x p).g.toGhost := by
  have old := lineage.1 p
  have ack := ack_newer (x.g.promise p) absent (x.g.vc+1) false 0 (by change (x.g.promise p).ver ≤ x.g.vc at old; omega)
  simp only [opDelete,commit,project,mutant]
  apply active_commit
  simp only [show ("none" == "MDD") = false from rfl,Bool.false_eq_true,ite_false,ack]
  intro q held on eq
  by_cases same : q = p
  · subst q; simp [put,opEdit]
  · simpa only [put,same,ite_false] using h q (by simpa [put,same] using held)
      (by simpa [put,same] using on) (by simpa [put,same] using eq)

theorem active_rename_pre6 (c : Config P) (x : State P) (src dst : Fin P)
    (refs : List (Fin P)) (rt : Fin P → Int) (h : ActiveAgreement x.s.toSys x.g.toGhost)
    (lineage : PromiseLineage (project x)) (hen : enabled c x (.opRename src dst refs rt) = true) :
    ActiveAgreement {x.s.toSys with pages := renamePages4 x src dst refs rt}
      {x.g.toGhost with promise := renamePromise4 x src dst refs rt} := by
  intro p held on eq
  by_cases mem : p ∈ (renameGot4 x src dst refs rt).map Prod.fst
  · simp [rename_promise_match6 c x src dst refs rt lineage hen p mem]
  · have pg := rename_not_got4 x src dst refs rt p mem
    have pr : renamePromise4 x src dst refs rt p = x.g.promise p := by
      exact fold_ack_unchanged6 (renameGot4 x src dst refs rt) x.g.promise (fun p => x.g.vc+1+p.val) p mem
    simpa only [pg,pr] using h p (by simpa [pg] using held)
      (by simpa [pr] using on) (by simpa [pg,pr] using eq)

theorem active_execute6 (c : Config P) (x : State P) (a : Action P)
    (mutant : c.mutant = "none") (h : ActiveAgreement x.s.toSys x.g.toGhost)
    (lineage : PromiseLineage (project x)) (snap : JobSnapshot x.s.toSys)
    (distinct : QueueDistinct x.s.toSys) (hen : enabled c x a = true) :
    ActiveAgreement (execute c x a).s.toSys (execute c x a).g.toGhost := by
  cases a with
  | old a =>
    have core := active_execute c mutant (project x) a h lineage.1 snap distinct hen
    simpa only [ActiveAgreement,old_pages6 c x a mutant,old_promise4 c x a mutant] using core
  | opDelete p => exact active_delete6 c x p mutant h lineage
  | opRename src dst refs rt =>
    simp only [execute,opRename,commit,project,mutant]
    apply active_commit
    simpa only [ActiveAgreement,renamePages4,renamePromise4,renameGot4,
      show ("none" != "MRN5") = true from rfl,Bool.and_true] using
      active_rename_pre6 c x src dst refs rt h lineage hen
  | flushDel p => exact active_commit c (project x) _ _ _ h

theorem active_reachable6 (c : Config P) (disk : Fin P → Int) (x : State P)
    (mutant : c.mutant = "none") (hr : Reachable c disk x) :
    ActiveAgreement x.s.toSys x.g.toGhost := by
  induction hr with
  | initial => simp [ActiveAgreement,initWith,embed,Storage21.init]
  | @advance x y hr a hs ih =>
    unfold step at hs; split at hs
    · cases hs; exact active_execute6 c x a mutant ih (lineage_reachable4 c disk x mutant hr)
        (snapshot_reachable4 c disk x mutant hr) (distinct_reachable6 c disk x mutant hr) (by assumption)
    · contradiction
end Storage3
