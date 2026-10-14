import AInvariants6
import CorePublication6
import CoreCustodyClosed6
namespace Storage3
open Storage21
set_option maxRecDepth 8192
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false

theorem fence_execute6 (c : Config P) (x : State P) (a : Action P)
    (mutant : c.mutant = "none") (h : PublicationFence x.s.toSys x.g.toGhost)
    (epochs : EpochBounds x.s.toSys x.g.toGhost) (hen : enabled c x a = true) :
    PublicationFence (execute c x a).s.toSys (execute c x a).g.toGhost := by
  cases a with
  | old a =>
    have core := fence_execute c (project x) a h epochs hen
    rw [old_sys6 c x a mutant]
    simpa only [PublicationFence,old_promise4 c x a mutant,old_ext6 c x a mutant] using core
  | opDelete p =>
    simp only [execute,opDelete,commit,project]
    apply fence_commit
    apply fence_frame x.s.toSys _ x.g.toGhost _ h
    all_goals try rfl
    intro q u hn hp
    split
    · exact hp
    · by_cases eq : q = p
      · subst q; simp only [put,ite_true]; exact ack_epoch_le _ _ _ _ _ _ hp hn
      · simpa [put,eq] using hp
  | opRename src dst refs rt =>
    simp only [execute,opRename,commit,project]
    apply fence_commit
    intro on phase
    have raw := h on phase
    refine ⟨raw.1,?_,raw.2.2⟩
    -- Only the job page's bound is required; unrelated promises can be larger.
    have point : ∀ es : List (Fin P × Int), ∀ pr : Fin P → Promise,
        (pr x.s.job.p).ep ≤ x.s.job.ep →
        (es.foldl (fun acc e => put acc e.1 (ack (acc e.1) e.2 (x.g.vc+1+e.1.val) false 0)) pr x.s.job.p).ep ≤ x.s.job.ep := by
      intro es; induction es with
      | nil => intro pr hp; exact hp
      | cons e tail ih =>
        intro pr hp; apply ih
        by_cases eq : x.s.job.p = e.1
        · simp only [put,eq,ite_true]; exact ack_epoch_le _ _ _ _ _ _ (by simpa [eq] using hp) raw.1
        · simpa [put,eq] using hp
    exact point _ _ raw.2.1
  | flushDel p =>
    simp [execute,flushDel,lift,project,Storage21.flush,Storage21.commit,PublicationFence]

theorem fence_reachable6 (c : Config P) (disk : Fin P → Int) (x : State P)
    (mutant : c.mutant = "none") (hr : Reachable c disk x) :
    PublicationFence x.s.toSys x.g.toGhost := by
  induction hr with
  | initial => simp [PublicationFence,initWith,embed,Storage21.init,nojob]
  | @advance x y hr a hs ih =>
    unfold step at hs; split at hs
    · cases hs; exact fence_execute6 c x a mutant ih (epochs_reachable6 c disk x mutant hr) (by assumption)
    · contradiction

theorem promiseBytes_execute6 (c : Config P) (x : State P) (a : Action P)
    (mutant : c.mutant = "none") (h : PromiseBytes c x.g.toGhost)
    (bytes : ByteDomain c x.s.toSys) (hen : enabled c x a = true) :
    PromiseBytes c (execute c x a).g.toGhost := by
  cases a with
  | old a =>
    have core := promiseBytes_execute c mutant (project x) a h bytes hen
    simpa only [PromiseBytes,project,old_promise4 c x a mutant] using core
  | opDelete p =>
    have new := bytes_delete4 c x p mutant bytes
    simp only [execute,opDelete,commit,project,mutant]
    apply promiseBytes_commit
    · simp only [show ("none" == "MDD") = false from rfl,Bool.false_eq_true,ite_false]
      intro q on
      by_cases eq : q = p
      · subst q; simp only [put,ite_true] at on ⊢; exact promiseBytes_ack c _ _ _ _ _ (h _) (real_absent c) on
      · simpa [put,eq] using h q (by simpa [put,eq] using on)
    · simpa only [opDelete,commit,project,Storage21.commit,mutant] using new.pages
  | opRename src dst refs rt =>
    have new := bytes_opRename4 c x src dst refs rt mutant bytes hen
    simp only [execute,opRename,commit,project,mutant]
    apply promiseBytes_commit
    · simp only [show ("none" != "MRN5") = true from rfl,Bool.and_true]
      intro p on
      change (renamePromise4 x src dst refs rt p).on = true at on
      change RealByte c (renamePromise4 x src dst refs rt p).bytes
      by_cases mem : p ∈ (renameGot4 x src dst refs rt).map Prod.fst
      · obtain ⟨e,he,ep⟩ := List.mem_map.mp mem
        have fg := fold_ack_point4 (renameGot4 x src dst refs rt) x.g.promise
          (fun p => x.g.vc+1+p.val) p (renamePages4 x src dst refs rt p).buf
          (by intro e he eq; have hp := (rename_got_shape4 c x src dst refs rt hen e he).1; simpa [eq] using hp.symm)
        have eq : renamePromise4 x src dst refs rt p = ack (x.g.promise p)
            (renamePages4 x src dst refs rt p).buf (x.g.vc+1+p.val) false 0 := by simpa [mem] using fg
        rw [eq] at on ⊢
        apply promiseBytes_ack c _ _ _ _ _ (h _) _ on
        exact new.pages p
      · have eq := fold_ack_unchanged6 (renameGot4 x src dst refs rt) x.g.promise
          (fun p => x.g.vc+1+p.val) p mem
        change renamePromise4 x src dst refs rt p = x.g.promise p at eq
        rw [eq] at on ⊢; exact h p on
    · exact new.pages
  | flushDel p => exact promiseBytes_commit c (project x) _ _ _ h bytes.pages

theorem promiseBytes_reachable6 (c : Config P) (disk : Fin P → Int) (x : State P)
    (mutant : c.mutant = "none") (realDisk : ∀ p, RealByte c (disk p))
    (hr : Reachable c disk x) : PromiseBytes c x.g.toGhost := by
  induction hr with
  | initial => simp [PromiseBytes,initWith,embed,Storage21.init]
  | @advance x y hr a hs ih =>
    unfold step at hs; split at hs
    · cases hs; exact promiseBytes_execute6 c x a mutant ih (bytes_reachable4 c disk x mutant realDisk hr) (by assumption)
    · contradiction

end Storage3
