import CoreAnchorClosure6
import AClosure6
namespace Storage3
open Storage21
set_option maxRecDepth 8192
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false

-- Auxiliary authority view. The real state, transitions and guarantee are unchanged.
-- The view's empty bad list is used only to carry structural admissibility; actual G is
-- proved separately from literal JobAuthority, never inferred from the view's tags.
def shadow6 (x : State P) : Storage21.State P :=
  ⟨x.s.toSys,{x.g.toGhost with bad := [], wrote := (fun p => union (x.g.wrote p) ((x.g.opRead p).map fun b => (b,0)))}⟩

def Auth3_6 (x : State P) (p : Fin P) (b : Int) : Prop :=
  b ∈ x.g.seen p ∨ (∃ v, (b,v) ∈ x.g.wrote p) ∨ b ∈ x.g.opRead p

theorem shadow_auth6 (x : State P) (p : Fin P) (b : Int) :
    Auth (shadow6 x).g p b ↔ Auth3_6 x p b := by
  simp [Auth,shadow6,Auth3_6,mem_union_general,List.mem_map]
  grind

theorem shadow_sys_execute6 (c : Config P) (x : State P) (a : Storage21.Action P) :
    (Storage21.execute c (shadow6 x) a).s = (Storage21.execute c (project x) a).s := by
  cases a <;> first | rfl | skip
  case check =>
    simp only [Storage21.execute,Storage21.check,shadow6,project]; split <;> rfl
  case deliverUp ok =>
    simp only [Storage21.execute,shadow6,project]
    cases hq : x.s.up with
    | nil => rfl
    | cons m tail =>
      simp only [Storage21.deliverUp]
      split <;> (try split) <;> (try split) <;> (try split)
      all_goals simp only [upOpen,upClose,upDiscard,upSubmit,upOp]
      all_goals (try split) <;> (try split) <;> (try split)
      all_goals rfl


theorem shadow_vc_execute6 (c : Config P) (x : State P) (a : Storage21.Action P) :
    (Storage21.execute c (shadow6 x) a).g.vc = (Storage21.execute c (project x) a).g.vc := by
  cases a <;> first | rfl | skip
  case check =>
    simp only [Storage21.execute,Storage21.check,shadow6,project]; split <;> rfl
  case deliverUp ok =>
    simp only [Storage21.execute,shadow6,project]
    cases hq : x.s.up with
    | nil => rfl
    | cons m tail =>
      simp only [Storage21.deliverUp]
      split <;> (try split) <;> (try split) <;> (try split)
      all_goals simp only [upOpen,upClose,upDiscard,upSubmit,upOp]
      all_goals (try split) <;> (try split) <;> (try split)
      all_goals rfl
  case draftSync p =>
    simp only [Storage21.execute,Storage21.draftSync,shadow6,project]; split <;> (try split) <;> rfl


theorem shadow_guard_execute6 (c : Config P) (x : State P) (a : Storage21.Action P) :
    (Storage21.execute c (shadow6 x) a).g.guard = (Storage21.execute c (project x) a).g.guard := by
  cases a <;> first | rfl | skip
  case check =>
    simp only [Storage21.execute,Storage21.check,shadow6,project]; split <;> rfl
  case deliverUp ok =>
    simp only [Storage21.execute,shadow6,project]
    cases hq : x.s.up with
    | nil => rfl
    | cons m tail =>
      simp only [Storage21.deliverUp]
      split <;> (try split) <;> (try split) <;> (try split)
      all_goals simp only [upOpen,upClose,upDiscard,upSubmit,upOp]
      all_goals (try split) <;> (try split) <;> (try split)
      all_goals rfl
  case draftSync p =>
    simp only [Storage21.execute,Storage21.draftSync,shadow6,project]; split <;> (try split) <;> rfl


theorem shadow_auth_insert6 (wr : List (Int × Int)) (rd : List Int) (fresh : List (Int × Int))
    (b : Int) : (∃ v, (b,v) ∈ union (union wr (rd.map fun t => (t,0))) fresh) ↔
      (∃ v, (b,v) ∈ union (union wr fresh) (rd.map fun t => (t,0))) := by
  simp [mem_union_general,List.mem_map]
  grind

theorem shadow_old_auth6 (c : Config P) (x : State P) (a : Storage21.Action P)
    (mutant : c.mutant = "none") :
    AuthMono (Storage21.execute c (shadow6 x) a).g (shadow6 (execute c x (.old a))).g := by
  cases a <;> simp only [Storage21.execute,execute,executeOld]
  case deliverUp ok =>
    simp only [mutant,show ("none" == "MDO") = false from rfl,Bool.false_eq_true,ite_false]
    cases hq : x.s.up with
    | nil =>
      simp only [shadow6,project,hq,lift]
      intro p b hb; exact hb
    | cons m tail =>
      simp only [shadow6,project,hq]
      simp only [Storage21.deliverUp]
      split <;> (try split) <;> (try split) <;> (try split)
      all_goals simp only [upOpen,upClose,upDiscard,upSubmit,upOp]
      all_goals (try split) <;> (try split) <;> (try split)
      all_goals intro p b hb
      all_goals by_cases eq : p = m.q
      all_goals simp [eq,Auth,shadow6,lift,project,Storage21.commit,mem_union_general,List.mem_map,put] at hb ⊢
      all_goals grind
  case draftSync p =>
    simp only [Storage21.draftSync,shadow6,project]
    split <;> (try split) <;> (try split)
    all_goals intro q b hb
    all_goals by_cases eq : q = p
    all_goals simp [eq,Auth,shadow6,lift,project,Storage21.commit,mem_union_general,List.mem_map,put] at hb ⊢
    all_goals grind
  case check =>
    simp only [Storage21.check,shadow6,project]
    split
    all_goals intro p b hb
    all_goals simp [Auth,shadow6,lift,project,Storage21.commit,mem_union_general,List.mem_map,put] at hb ⊢
    all_goals grind
  case rename =>
    intro p b hb
    by_cases eq : p = x.s.job.p
    all_goals simp [eq,Auth,shadow6,rename,commit,Storage21.rename,project,Storage21.commit,
      mem_union_general,List.mem_map,put] at hb ⊢
    all_goals grind
  all_goals intro p b hb
  all_goals simp [Auth,shadow6,lift,project,Storage21.commit,mem_union_general,List.mem_map,put,gDown,down,
    wOpen,wClose,wEdit,wSend,wRecv,wOp,wResolve,wDiscard,observe,flush,dirSync,Storage21.dirSync,
    saveFail,switchReq,switchFin,crash,windowCrash,power,Storage21.power,extWriteD,launch,commit] at hb ⊢
  all_goals grind

end Storage3
