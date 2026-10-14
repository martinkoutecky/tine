import Lineage4
import CoreSeen21
namespace Storage3
open Storage21
set_option maxRecDepth 8192
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false

theorem fold_put_property_mem4 (pages : Fin P → α) (refs : List (Fin P)) (nr : Fin P → α)
    (pr : α → Prop) (h : ∀ p, pr (pages p)) (hn : ∀ p ∈ refs, pr (nr p)) :
    ∀ p, pr (refs.foldl (fun acc r => put acc r (nr r)) pages p) := by
  intro p; rw [fold_put_if]; split
  · exact hn p (by assumption)
  · exact h p

theorem bytes_delete4 (c : Config P) (x : State P) (p : Fin P)
    (mutant : c.mutant = "none") (h : ByteDomain c x.s.toSys) : ByteDomain c (opDelete c x p).s.toSys := by
  simp only [opDelete,commit,project,mutant]
  apply bytes_commit
  refine ⟨h.disk,h.stable,?_,?_,h.job,h.w,h.mb,h.up⟩
  · exact property_put _ _ _ (fun pg : Page => RealByte c pg.buf) h.pages (by simpa [opEdit] using real_absent c)
  · simp only [show ("none" == "MDD") = false from rfl,Bool.false_eq_true,ite_false]
    exact property_put _ _ _ (fun dr : Draft => dr.bytes = noneBytes ∨ RealByte c dr.bytes) h.drafts
      (draft_real c _ (by simpa [opEdit] using real_absent c))

theorem bytes_opRename4 (c : Config P) (x : State P) (src dst : Fin P)
    (refs : List (Fin P)) (rt : Fin P → Int) (mutant : c.mutant = "none")
    (h : ByteDomain c x.s.toSys) (hen : enabled c x (.opRename src dst refs rt) = true) :
    ByteDomain c (opRename c x src dst refs rt).s.toSys := by
  have text : RealByte c (opCur x.s src) := by unfold opCur; split; exact h.pages src; exact h.disk src
  have rewrites : ∀ p ∈ refs, RealByte c (rt p) := by
    have all : refs.all (fun r => r != src && r != dst && c.texts.contains (rt r)) = true := by
      simp only [enabled,Bool.and_eq_true] at hen; grind
    intro p hp; have hb := List.all_eq_true.mp all p hp
    simp only [Bool.and_eq_true,List.contains_eq_mem] at hb
    exact Or.inr (by simpa using hb.2)
  simp only [opRename,commit,project,mutant]
  apply bytes_commit
  refine ⟨h.disk,h.stable,?_,?_,h.job,h.w,h.mb,h.up⟩
  · apply fold_put_property_mem4 _ _ _ (fun pg : Page => RealByte c pg.buf)
    · split
      · exact property_put _ _ _ (fun pg : Page => RealByte c pg.buf)
          (property_put _ _ _ (fun pg : Page => RealByte c pg.buf) h.pages (by simpa [opEdit] using text))
          (by simpa [opEdit] using real_absent c)
      · exact h.pages
    · intro p hp; simpa [opEdit] using rewrites p hp
  · simp only [show ("none" == "MRN4") = false from rfl,show ("none" == "MRN1") = false from rfl,
      show ("none" == "MRN5") = false from rfl,Bool.false_eq_true,ite_false]
    apply fold_put_property_mem4 _ _ _ (fun dr : Draft => dr.bytes = noneBytes ∨ RealByte c dr.bytes)
    · split
      · exact h.drafts
      · exact property_put _ _ _ (fun dr : Draft => dr.bytes = noneBytes ∨ RealByte c dr.bytes)
          (property_put _ _ _ (fun dr : Draft => dr.bytes = noneBytes ∨ RealByte c dr.bytes) h.drafts
            (draft_real c _ (by simpa [opEdit] using text)))
          (draft_real c _ (by simpa [opEdit] using real_absent c))
    · intro p hp; exact draft_real c _ (by simpa [opEdit] using rewrites p hp)

theorem bytes_execute4 (c : Config P) (x : State P) (a : Action P)
    (mutant : c.mutant = "none") (h : ByteDomain c x.s.toSys)
    (hen : enabled c x a = true) : ByteDomain c (execute c x a).s.toSys := by
  cases a with
  | old a =>
    rw [show (execute c x (.old a)).s.toSys = (Storage21.execute c (project x) a).s from old_sys4 c x a mutant]
    exact bytes_execute c (project x) a h hen
  | opDelete p => exact bytes_delete4 c x p mutant h
  | opRename src dst refs rt => exact bytes_opRename4 c x src dst refs rt mutant h hen
  | flushDel p =>
    simp only [execute,flushDel,lift,project,Storage21.flush]
    apply bytes_commit
    exact ⟨h.disk,h.stable,h.pages,h.drafts,fun _ => h.pages p,h.w,h.mb,h.up⟩

theorem bytes_reachable4 (c : Config P) (disk : Fin P → Int) (x : State P)
    (mutant : c.mutant = "none") (realDisk : ∀ p, RealByte c (disk p))
    (hr : Reachable c disk x) : ByteDomain c x.s.toSys := by
  induction hr with
  | initial =>
    exact ⟨realDisk,realDisk,by simp [initWith,embed,Storage21.init],by simp [initWith,embed,Storage21.init],
      by simp [initWith,embed,Storage21.init,nojob],by simp [initWith,embed,Storage21.init],
      by simp [initWith,embed,Storage21.init],by simp [initWith,embed,Storage21.init]⟩
  | @advance x y hr a hs ih =>
    unfold step at hs; split at hs
    · cases hs; exact bytes_execute4 c x a mutant ih (by assumption)
    · contradiction

theorem seen_mono4 (c : Config P) (x : State P) (a : Action P) (mutant : c.mutant = "none") :
    SeenMono x.g.toGhost (execute c x a).g.toGhost := by
  cases a with
  | old a =>
    have core := seenMono_execute c (project x) a
    cases a <;> simp only [execute,executeOld]
    case rename => exact core
    case deliverUp ok => simpa [mutant,lift,project] using core
    all_goals exact core
  | opDelete p => exact seen_commit c (project x) _ _ "opDelete"
  | opRename src dst refs rt => exact seen_commit c (project x) _ _ "opRename"
  | flushDel p => exact seenMono_execute c (project x) (.flush p)

theorem windowSeen_execute4 (c : Config P) (x : State P) (a : Action P)
    (mutant : c.mutant = "none") (h : WindowSeen (project x)) (hb : ByteDomain c x.s.toSys)
    (hen : enabled c x a = true) : WindowSeen (project (execute c x a)) := by
  cases a with
  | old a =>
    have core := windowSeen_execute c (project x) a hb hen h
    cases a <;> simp only [execute,executeOld]
    case rename => exact core
    case deliverUp ok => simpa [mutant,lift,project] using core
    all_goals exact core
  | opDelete p => exact windowSeen_commit c (project x) _ _ _ hb.w
  | opRename src dst refs rt => exact windowSeen_commit c (project x) _ _ _ hb.w
  | flushDel p => exact windowSeen_commit c (project x) _ _ _ hb.w

theorem windowSeen_reachable4 (c : Config P) (disk : Fin P → Int) (x : State P)
    (mutant : c.mutant = "none") (realDisk : ∀ p, RealByte c (disk p))
    (hr : Reachable c disk x) : WindowSeen (project x) := by
  induction hr with
  | initial => simp [WindowSeen,initWith,embed,project,Storage21.init]
  | @advance x y hr a hs ih =>
    unfold step at hs; split at hs
    · cases hs; exact windowSeen_execute4 c x a mutant ih (bytes_reachable4 c disk x mutant realDisk hr) (by assumption)
    · contradiction

theorem carrierSeen_execute4 (c : Config P) (x : State P) (a : Action P)
    (mutant : c.mutant = "none") (h : CarrierSeen (project x)) (hw : WindowSeen (project x))
    (hb : ByteDomain c x.s.toSys) (hen : enabled c x a = true) :
    CarrierSeen (project (execute c x a)) := by
  cases a with
  | old a =>
    have core := carrierSeen_execute c (project x) a h hw hen hb
    cases a <;> simp only [execute,executeOld]
    case rename => exact core
    case deliverUp ok => simpa [mutant,lift,project] using core
    all_goals exact core
  | opDelete p =>
    exact carrierSeen_frame (project x) _ h rfl (seen_mono4 c x (.opDelete p) mutant)
  | opRename src dst refs rt =>
    exact carrierSeen_frame (project x) _ h rfl (seen_mono4 c x (.opRename src dst refs rt) mutant)
  | flushDel p => exact carrierSeen_frame (project x) _ h rfl (seen_mono4 c x (.flushDel p) mutant)

theorem carrierSeen_reachable4 (c : Config P) (disk : Fin P → Int) (x : State P)
    (mutant : c.mutant = "none") (realDisk : ∀ p, RealByte c (disk p))
    (hr : Reachable c disk x) : CarrierSeen (project x) := by
  induction hr with
  | initial => simp [CarrierSeen,initWith,embed,project,Storage21.init]
  | @advance x y hr a hs ih =>
    unfold step at hs; split at hs
    · cases hs; exact carrierSeen_execute4 c x a mutant ih (windowSeen_reachable4 c disk x mutant realDisk hr)
        (bytes_reachable4 c disk x mutant realDisk hr) (by assumption)
    · contradiction
end Storage3
