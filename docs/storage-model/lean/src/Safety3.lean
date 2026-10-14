import Storage3
import CoreC21
namespace Storage3
open Storage21
set_option maxRecDepth 8192
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false

@[simp] theorem C_commit3 (c : Config P) (x : State P) (s : Sys P) (g : Ghost P) (n : String) :
    Storage21.CInv c (project (commit c x s g n)) ↔ Storage21.CInv c ⟨s.toSys,g.toGhost⟩ :=
  Storage21.C_commit c (project x) s.toSys g.toGhost n

theorem C_execute3 (c : Config P) (x : State P) (a : Action P)
    (mutant : c.mutant = "none") (hx : Storage21.CInv c (project x))
    (hen : enabled c x a = true) : Storage21.CInv c (project (execute c x a)) := by
  cases a with
  | old a =>
    have core := Storage21.C_execute c (project x) a mutant hx hen
    cases a <;> simp only [execute,executeOld]
    case rename =>
      refine Storage21.C_frame c (Storage21.rename c (project x)) (project (rename c x)) rfl rfl rfl ?_ core
      simp [Storage3.rename,Storage3.commit,project,Storage21.rename,Storage21.CTag]
    case deliverUp ok => simpa [mutant,lift,project] using core
    all_goals exact core
  | opRename src dst refs rt =>
    simp only [execute,opRename,commit,project,Storage21.C_commit]
    simpa [Storage21.CInv,Storage21.Guarded,Storage21.CTag,project] using hx
  | opDelete p =>
    simp only [execute,opDelete,commit,project,Storage21.C_commit]
    simpa [Storage21.CInv,Storage21.Guarded,Storage21.CTag,project] using hx
  | flushDel p =>
    simpa [execute,flushDel,lift,project,Storage21.flush,Storage21.C_commit,
      Storage21.CInv,Storage21.Guarded,Storage21.CTag] using hx.1

theorem C_reachable3 (c : Config P) (disk : Fin P → Int) (x : State P)
    (mutant : c.mutant = "none") (hr : Reachable c disk x) : Storage21.CInv c (project x) := by
  induction hr with
  | initial => simp [initWith,embed,project,Storage21.CInv,Storage21.Guarded,Storage21.init,Storage21.nojob]
  | @advance x y hr a hs ih =>
    unfold step at hs; split at hs
    · cases hs; exact C_execute3 c x a mutant ih (by assumption)
    · contradiction

theorem C_absent_reachable (c : Config P) (disk : Fin P → Int) (x : State P)
    (mutant : c.mutant = "none") (hr : Reachable c disk x) : "C-overwrote-external" ∉ x.g.bad :=
  (C_reachable3 c disk x mutant hr).1

def NewTagsClean (x : Storage21.State P) : Prop :=
  "D4-unclean-page-changed" ∉ x.g.bad ∧ "rename-clobbered-target" ∉ x.g.bad

@[simp] theorem newTags_commit (c : Config P) (x : Storage21.State P) (s : Storage21.Sys P)
    (g : Storage21.Ghost P) (n : String) :
    NewTagsClean (Storage21.commit c x s g n) ↔ NewTagsClean ⟨s,g⟩ := by
  simp [NewTagsClean,Storage21.commit]

theorem newTags_core_execute (c : Config P) (x : Storage21.State P) (a : Storage21.Action P)
    (h : NewTagsClean x) : NewTagsClean (Storage21.execute c x a) := by
  cases a <;> simp only [Storage21.execute]
  case deliverUp ok =>
    cases hq : x.s.up with
    | nil => exact h
    | cons m tail =>
      simp only [Storage21.deliverUp]
      split <;> (try split) <;> (try split) <;> (try split)
      all_goals simp only [Storage21.upOpen,Storage21.upClose,Storage21.upDiscard,Storage21.upOp,Storage21.upSubmit]
      all_goals (try split) <;> (try split)
      all_goals simp_all [newTags_commit,NewTagsClean,Storage21.commit]
  case check => simp only [Storage21.check]; split <;> simpa [Storage21.commit,NewTagsClean] using h
  case draftSync p => simp only [Storage21.draftSync]; split <;> (try split) <;> simpa [Storage21.commit,NewTagsClean] using h
  all_goals simpa [Storage21.wOpen,Storage21.wEdit,Storage21.wSend,Storage21.wResolve,Storage21.wDiscard,
    Storage21.wOp,Storage21.wClose,Storage21.wRecv,Storage21.observe,Storage21.flush,Storage21.rename,
    Storage21.dirSync,Storage21.saveFail,Storage21.switchReq,Storage21.switchFin,Storage21.extWriteD,
    Storage21.crash,Storage21.windowCrash,Storage21.power,Storage21.launch,NewTagsClean,Storage21.commit,Storage21.gDown] using h

theorem newTags_old (c : Config P) (x : State P) (a : Storage21.Action P)
    (mutant : c.mutant = "none") (h : NewTagsClean (project x)) :
    NewTagsClean (project (execute c x (.old a))) := by
  have core := newTags_core_execute c (project x) a h
  cases a <;> simp only [execute,executeOld]
  case rename => simpa [rename,Storage21.execute,Storage21.rename,commit,project,NewTagsClean,Storage21.commit] using core
  case deliverUp ok => simpa [mutant,lift,project] using core
  all_goals exact core

theorem mem_union_fin (a b : List (Fin P)) (t : Fin P) :
    t ∈ union a b ↔ t ∈ a ∨ t ∈ b := by
  induction b generalizing a with
  | nil => simp [union]
  | cons v vs ih =>
    simp only [union,List.foldl_cons]
    change t ∈ union (if a.contains v then a else a ++ [v]) vs ↔ _
    rw [ih]
    split <;> simp_all <;> grind

theorem rename_clean_guard (c : Config P) (x : State P) (src dst : Fin P)
    (refs : List (Fin P)) (rt : Fin P → Int) (mutant : c.mutant = "none")
    (hen : enabled c x (.opRename src dst refs rt) = true) :
    (union (if opCur x.s src != absent then [src,dst] else []) refs).all (opClean x.s) = true ∧
    (opCur x.s src != absent && opCur x.s dst != absent) = false := by
  have rows : refs.all (opClean x.s) = true := by
    simp only [enabled,mutant] at hen
    simp at hen
    grind
  by_cases full : opCur x.s src = absent
  · constructor
    · apply List.all_eq_true.mpr
      intro p hp
      have mem : p ∈ refs := by
        have m := (mem_union_fin _ _ _).mp hp
        simpa [full] using m
      exact List.all_eq_true.mp rows p mem
    · simp [full]
  · have cl : opClean x.s src = true ∧ opClean x.s dst = true ∧ opCur x.s dst = absent := by
      simp [enabled,mutant,full] at hen
      grind
    constructor
    · apply List.all_eq_true.mpr
      intro p hp
      rcases (mem_union_fin _ _ _).mp hp with named | ref
      · have cases : p = src ∨ p = dst := by simpa [full] using named
        rcases cases with rfl | rfl
        · exact cl.1
        · exact cl.2.1
      · exact List.all_eq_true.mp rows p ref
    · simp [cl.2.2]

theorem newTags_execute3 (c : Config P) (x : State P) (a : Action P)
    (mutant : c.mutant = "none") (h : NewTagsClean (project x))
    (hen : enabled c x a = true) : NewTagsClean (project (execute c x a)) := by
  cases a with
  | old a => exact newTags_old c x a mutant h
  | opDelete p => simpa [execute,opDelete,commit,project,Storage21.commit,NewTagsClean] using h
  | flushDel p => simpa [execute,flushDel,lift,project,Storage21.flush,newTags_commit,NewTagsClean,Storage21.commit] using h
  | opRename src dst refs rt =>
    obtain ⟨cl,free⟩ := rename_clean_guard c x src dst refs rt mutant hen
    simp only [execute,opRename,commit,project,cl,free,Bool.not_true,tags,Bool.false_eq_true,ite_false]
    simpa [Storage21.commit,NewTagsClean,project] using h

theorem newTags_reachable (c : Config P) (disk : Fin P → Int) (x : State P)
    (mutant : c.mutant = "none") (hr : Reachable c disk x) : NewTagsClean (project x) := by
  induction hr with
  | initial => simp [NewTagsClean,initWith,embed,project,Storage21.init]
  | @advance x y hr a hs ih =>
    unfold step at hs; split at hs
    · cases hs; exact newTags_execute3 c x a mutant ih (by assumption)
    · contradiction

end Storage3
