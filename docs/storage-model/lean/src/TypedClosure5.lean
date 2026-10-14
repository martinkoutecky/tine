import TypedCore5
import BClosure4
import Scope4
namespace Storage3
open Storage21
set_option maxRecDepth 8192
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false

theorem protected_delete5 (c : Config P) (x : State P) (p : Fin P)
    (mutant : c.mutant = "none") (h : TypedProtected5 (project x))
    (lineage : PromiseLineage (project x)) : TypedProtected5 (project (opDelete c x p)) := by
  have bound := lineage.1 p
  change (x.g.promise p).ver ≤ x.g.vc at bound
  have ack := ack_newer (x.g.promise p) absent (x.g.vc+1) false 0 (by omega)
  simp only [opDelete,commit,project,mutant]
  apply protected_commit5
  simp only [show ("none" == "MDD") = false from rfl,Bool.false_eq_true,ite_false,ack]
  apply protected_update5 _ _ _ _ _ h
  intro _ _
  right
  simp [opEdit]

theorem protected_rename_pre5 (c : Config P) (x : State P) (src dst : Fin P)
    (refs : List (Fin P)) (rt : Fin P → Int) (h : TypedProtected5 (project x))
    (lineage : PromiseLineage (project x)) (hen : enabled c x (.opRename src dst refs rt) = true) :
    TypedProtected5 ⟨{x.s.toSys with pages := renamePages4 x src dst refs rt},
      {x.g.toGhost with promise := renamePromise4 x src dst refs rt}⟩ := by
  intro p hh ht
  have fg := fold_ack_point4 (renameGot4 x src dst refs rt) x.g.promise
    (fun p => x.g.vc+1+p.val) p (renamePages4 x src dst refs rt p).buf
    (by intro e he eq; have hp := (rename_got_shape4 c x src dst refs rt hen e he).1; simpa [eq] using hp.symm)
  change renamePromise4 x src dst refs rt p = _ at fg
  by_cases mem : p ∈ (renameGot4 x src dst refs rt).map Prod.fst
  · obtain ⟨e,he,ep⟩ := List.mem_map.mp mem
    have shape := rename_got_shape4 c x src dst refs rt hen e he
    have ver : (renamePages4 x src dst refs rt p).ver = x.g.vc+1+p.val := by simpa [ep] using shape.2.1
    have old := lineage.1 p
    change (x.g.promise p).ver ≤ x.g.vc at old
    have newer : (x.g.promise p).ver < x.g.vc+1+p.val := by omega
    have ackEq := ack_newer (x.g.promise p) (renamePages4 x src dst refs rt p).buf (x.g.vc+1+p.val) false 0 newer
    right; left
    change (renamePromise4 x src dst refs rt p).ver ≥ _ ∧ _
    simp [fg,mem,ackEq,ver]
  · have pg := rename_not_got4 x src dst refs rt p mem
    have pr : renamePromise4 x src dst refs rt p = x.g.promise p := by simpa [mem] using fg
    change (renamePages4 x src dst refs rt p).held = true at hh
    change (renamePages4 x src dst refs rt p).typed = true at ht
    rcases h p (by simpa [pg,project] using hh) (by simpa [pg,project] using ht) with seen | promise | blind
    · left; simpa [pg,project] using seen
    · right; left; simpa [pg,pr,project] using promise
    · right; right; simpa [Blind5,pg,project] using blind

theorem protected_opRename5 (c : Config P) (x : State P) (src dst : Fin P)
    (refs : List (Fin P)) (rt : Fin P → Int) (mutant : c.mutant = "none")
    (h : TypedProtected5 (project x)) (lineage : PromiseLineage (project x))
    (hen : enabled c x (.opRename src dst refs rt) = true) :
    TypedProtected5 (project (opRename c x src dst refs rt)) := by
  simp only [opRename,commit,project,mutant]
  apply protected_commit5
  simpa only [TypedProtected5,Blind5,renamePages4,renamePromise4,renameGot4,project,
    show ("none" != "MRN5") = true from rfl,Bool.and_true] using
    protected_rename_pre5 c x src dst refs rt h lineage hen

theorem protected_execute_s3_5 (c : Config P) (x : State P) (a : Action P)
    (mutant : c.mutant = "none") (h : TypedProtected5 (project x))
    (carrier : CarrierSeen (project x)) (bytes : ByteDomain c x.s.toSys)
    (inactive : InactiveClient (project x)) (lineage : PromiseLineage (project x))
    (hen : enabled c x a = true) : TypedProtected5 (project (execute c x a)) := by
  cases a with
  | old a =>
    have core := protected_execute5 c mutant (project x) a h carrier bytes inactive hen
    cases a <;> simp only [execute,executeOld]
    case rename => simpa [TypedProtected5,Blind5,rename,commit,project,Storage21.execute,Storage21.rename,Storage21.commit] using core
    case deliverUp ok => simpa [mutant,lift,project] using core
    all_goals exact core
  | opDelete p => exact protected_delete5 c x p mutant h lineage
  | opRename src dst refs rt => exact protected_opRename5 c x src dst refs rt mutant h lineage hen
  | flushDel p => exact protected_commit5 c (project x) _ _ _ h

theorem protected_reachable5 (c : Config P) (disk : Fin P → Int) (x : State P)
    (mutant : c.mutant = "none") (realDisk : ∀ p, RealByte c (disk p))
    (hr : Reachable c disk x) : TypedProtected5 (project x) := by
  induction hr with
  | initial => simp [TypedProtected5,initWith,embed,project,Storage21.init]
  | @advance x y hr a hs ih =>
    unfold step at hs; split at hs
    · cases hs
      exact protected_execute_s3_5 c x a mutant ih
        (carrierSeen_reachable4 c disk x mutant realDisk hr) (bytes_reachable4 c disk x mutant realDisk hr)
        (protocol_reachable3 c disk x mutant hr).inactive (lineage_reachable4 c disk x mutant hr) (by assumption)
    · contradiction

theorem staleHead_proved5 (c : Config P) (disk : Fin P → Int)
    (mutant : c.mutant = "none") (realDisk : ∀ p, RealByte c (disk p)) :
    StaleHeadObligation4 c disk := by
  intro x hr ok m tail hq hen kind hh ht _ _
  rcases protected_reachable5 c disk x mutant realDisk hr m.p hh ht with seen | promise | blind
  · exact Or.inl seen
  · exact Or.inr promise
  · have mem : m ∈ (project x).s.up := by simp [project,hq]
    exact False.elim (blind.2.1 m mem kind rfl)

theorem B_reachable5 (c : Config P) (disk : Fin P → Int) (x : State P)
    (mutant : c.mutant = "none") (realDisk : ∀ p, RealByte c (disk p))
    (hr : Reachable c disk x) : "B-unsaved-replaced" ∉ x.g.bad :=
  B_reachable4_of_staleHead c disk mutant (staleHead_proved5 c disk mutant realDisk) x hr

theorem B_positiveTexts5 (c : Config P) (T : Nat)
    (texts : c.texts = positiveTexts4 T) (two : 2 ≤ T) (mutant : c.mutant = "none")
    (x : State P) (hr : Reachable c (initDisk c) x) : "B-unsaved-replaced" ∉ x.g.bad :=
  B_reachable5 c (initDisk c) x mutant (positiveTexts_initial_real4 c T texts two) hr

theorem guarantee_remaining5 (c : Config P) (disk : Fin P → Int) (x : State P)
    (mutant : c.mutant = "none") (realDisk : ∀ p, RealByte c (disk p))
    (hr : Reachable c disk x) : guarantee x = true ↔ noLoss x = true ∧ "G-overwrote-unseen" ∉ x.g.bad :=
  guarantee_remaining4_of_staleHead c disk mutant (staleHead_proved5 c disk mutant realDisk) x hr

end Storage3
