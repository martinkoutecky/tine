import GateOwnership11
namespace Storage21
set_option maxHeartbeats 4000000
set_option maxRecDepth 8192
set_option linter.unusedSimpArgs false
def RequestText11 (m : Up P) : Prop :=
  (m.kind = "submit" → m.t ≠ absent) ∧
  (m.kind = "op" → m.t ≠ absent ∧ m.t2 ≠ absent)
def InputTexts11 (w : Fin P → W) (up : List (Up P)) : Prop :=
  (∀ p, (w p).pend = true → (w p).text ≠ absent) ∧ ∀ m ∈ up, RequestText11 m
theorem inputs_nil11 : InputTexts11 (P := P) (fun _ => {}) [] := by simp [InputTexts11]
theorem inputs_append11 (w : Fin P → W) (up : List (Up P)) (m : Up P)
    (hw : ∀ p, (w p).pend = true → (w p).text ≠ absent)
    (hu : ∀ u ∈ up, RequestText11 u) (hm : RequestText11 m) : InputTexts11 w (up ++ [m]) := by
  refine ⟨hw,?_⟩
  intro u mem; rcases List.mem_append.mp mem with old | fresh
  · exact hu u old
  · have eq : u = m := by simpa using fresh
    subst u; exact hm
theorem inputs_delivery11 (c : Config P) (x : State P) (m : Up P) (tail : List (Up P))
    (ok : Bool) (h : InputTexts11 x.s.w (m::tail)) :
    InputTexts11 (deliverUp c x m tail ok).s.w (deliverUp c x m tail ok).s.up := by
  have ht : InputTexts11 x.s.w tail := ⟨h.1,by intro u hm; exact h.2 u (List.mem_cons_of_mem _ hm)⟩
  simp only [deliverUp]; split <;> (try split) <;> (try split) <;> (try split)
  all_goals simp only [upOpen,upClose,upDiscard,upSubmit,upOp]
  all_goals repeat (first | exact ht | split | (cases m.cur <;> exact ht))

theorem inputs_execute21_11 (c : Config P) (x : State P) (a : Action P)
    (mutant : c.mutant = "none") (texts : absent ∉ c.texts)
    (h : InputTexts11 x.s.w x.s.up) (en : enabled c x a = true) :
    InputTexts11 (execute c x a).s.w (execute c x a).s.up := by
  have textne (t : Int) (ht : t ∈ c.texts) : t ≠ absent := by intro eq; subst t; exact texts ht
  cases a <;> simp only [execute]
  case wEdit p t =>
    have mem : t ∈ c.texts := by simp [enabled] at en; grind
    refine ⟨?_,h.2⟩
    intro q; by_cases eq : q = p
    · subst q; simpa [wEdit,commit,put] using textne t mem
    · simpa [wEdit,commit,put,eq] using h.1 q
  case wSend p =>
    have pend : (x.s.w p).pend = true := by simp [enabled] at en; grind
    apply inputs_append11
    · intro q; by_cases eq : q = p
      · subst q; simp [wSend,commit,put]
      · simpa [wSend,commit,put,eq] using h.1 q
    · exact h.2
    · simpa [RequestText11,up1] using h.1 p pend
  case wResolve p t =>
    have mem : t ∈ c.texts := by simp [enabled] at en; grind
    apply inputs_append11
    · intro q; by_cases eq : q = p
      · subst q; simp [wResolve,commit,put]
      · simpa [wResolve,commit,put,eq] using h.1 q
    · exact h.2
    · simpa [RequestText11,up1] using textne t mem
  case wOp p q t u =>
    have tm : t ∈ c.texts := by simp [enabled] at en; grind
    have um : u ∈ c.texts := by simp [enabled] at en; grind
    have ne : p ≠ q := by simp [enabled] at en; grind
    apply inputs_append11
    · intro k
      by_cases kp : k = p
      · subst k; simpa [wOp,commit,put,ne] using (show (x.s.w p).pend = true → t ≠ absent from fun _ => textne t tm)
      · by_cases kq : k = q
        · subst k; simpa [wOp,commit,put,kp] using (show (x.s.w q).pend = true → u ≠ absent from fun _ => textne u um)
        · simpa [wOp,commit,put,kp,kq] using h.1 k
    · exact h.2
    · exact ⟨by simp,by intro _; exact ⟨textne t tm,textne u um⟩⟩
  case wOpen p =>
    apply inputs_append11
    · intro q; by_cases eq : q = p
      · subst q; simp [wOpen,commit,put]
      · simpa [wOpen,commit,put,eq] using h.1 q
    · exact h.2
    · simp [RequestText11,up1]
  case wClose p =>
    apply inputs_append11
    · intro q; by_cases eq : q = p
      · subst q; simp [wClose,commit,put]
      · simpa [wClose,commit,put,eq] using h.1 q
    · exact h.2
    · simp [RequestText11,up1]
  case wDiscard p =>
    apply inputs_append11
    · intro q; by_cases eq : q = p
      · subst q; simp [wDiscard,commit,put]
      · simpa [wDiscard,commit,put,eq] using h.1 q
    · exact h.2
    · simp [RequestText11,up1]
  case wRecv p =>
    refine ⟨?_,h.2⟩
    intro q
    by_cases eq : q = p
    · subst q
      simp only [wRecv,commit,put,ite_true]
      split <;> (try split) <;> (try split) <;> (try split) <;> (try split)
      all_goals first | exact h.1 p | (simp_all [mutant])
    · simpa [wRecv,commit,put,eq] using h.1 q
  case deliverUp ok =>
    cases hq : x.s.up with
    | nil => exact h
    | cons m tail => exact inputs_delivery11 c x m tail ok (by simpa [hq] using h)
  case check => simp only [check]; split <;> exact h
  case switchFin => exact inputs_nil11
  case crash => exact inputs_nil11
  case power keep => exact inputs_nil11
  case windowCrash =>
    constructor
    · simp [windowCrash,commit]
    · intro m mem
      have hm : m ∈ x.s.up.map (fun u => {u with cur := false}) := by
        simpa [windowCrash,commit,mutant] using mem
      obtain ⟨u,hu,rfl⟩ := List.mem_map.mp hm
      exact h.2 u hu
  all_goals exact h
end Storage21

namespace Storage35Base
open Storage21 Storage32
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false
theorem inputs_discard11 (c : Config P) (x : State P) (m : Up P) (tail : List (Up P))
    (ok : Bool) (h : InputTexts11 x.s.w (m::tail)) :
    InputTexts11 (upDiscard c x {x.s with up := tail} m ok).s.w
      (upDiscard c x {x.s with up := tail} m ok).s.up := by
  have ht : InputTexts11 x.s.w tail := ⟨h.1,by intro u hm; exact h.2 u (List.mem_cons_of_mem _ hm)⟩
  simp only [upDiscard]; split <;> (try split)
  all_goals cases m.cur <;> exact ht

theorem inputs_execute11 (c : Config P) (x : State P) (a : Action P)
    (mutant : c.mutant = "none") (texts : absent ∉ c.texts)
    (h : InputTexts11 x.s.w x.s.up) (en : Storage32.enabled c x a = true) :
    InputTexts11 (execute c x a).s.w (execute c x a).s.up := by
  cases a with
  | load p => exact h
  | flushDel p => exact h
  | opDelete p => exact h
  | opRename src dst refs rt => exact h
  | old a =>
    have olden : Storage21.enabled c (Storage3.project (Storage32.project x)) a = true :=
      Storage32.enabled_projects9 c x (.old a) en
    have core := inputs_execute21_11 c (Storage3.project (Storage32.project x)) a mutant texts h olden
    cases a <;> simp only [execute,Storage32.execute]
    case deliverUp ok =>
      cases hq : x.s.up with
      | nil => simpa [deliverUp,hq] using h
      | cons m tail =>
        simp only [deliverUp,hq]
        split
        · exact inputs_discard11 c x m tail ok (by simpa [hq] using h)
        · simpa [Storage32.lift,Storage3.executeOld,mutant,Storage3.lift,Storage21.execute,
            Storage32.project,Storage3.project,hq] using core
    case check =>
      simpa [Storage32.check,Storage32.lift,Storage3.executeOld,Storage3.lift,
        Storage21.execute,Storage21.check,mutant] using core
    case rename => simpa [Storage32.rename,Storage32.lift,mutant,Storage3.rename,Storage3.commit,
        Storage21.commit,Storage32.project] using h
    case dirSync ok =>
      cases ok <;> simpa [Storage32.dirSync,Storage32.lift,Storage32.discharge,mutant,
        Storage3.dirSync,Storage3.lift,Storage21.dirSync,Storage21.commit,Storage32.project,Storage3.project] using h
    case saveFail => simpa [Storage32.saveFail,Storage32.lift,mutant,Storage3.executeOld,
        Storage3.lift,Storage21.execute,Storage21.saveFail,Storage21.commit,Storage32.project,Storage3.project] using h
    all_goals exact core
end Storage35Base

namespace Storage35
open Storage21
set_option maxHeartbeats 4000000
theorem inputTexts_reachable35_11 (c : Config P) (disk : Fin P → Int) (x : State P)
    (mutant : c.mutant = "none") (texts : absent ∉ c.texts) (hr : Reachable c disk x) :
    InputTexts11 x.s.w x.s.up := by
  induction hr with
  | initial => exact inputs_nil11
  | @advance x y hr a hs ih =>
    unfold step at hs; split at hs
    · cases hs
      have en : enabled c x a = true := by assumption
      cases a with
      | prior a => exact Storage35Base.inputs_execute11 c (project x) a mutant texts ih en
      | opApply eff plan dst src => exact ih
      | extClaim p present => exact ih
    · contradiction
end Storage35
