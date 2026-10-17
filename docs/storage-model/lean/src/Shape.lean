import BClean
import Seen
namespace Storage
set_option maxRecDepth 8192
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false

abbrev PageShape (pg : Page) : Prop := pg.typed = false → pg.risk = false → pg.buf = pg.base
def Shapes (s : Sys P) : Prop := ∀ p, PageShape (s.pages p)
def JobSnapshot (s : Sys P) : Prop := s.job.on = true →
  (s.pages s.job.p).held = true ∧ (s.pages s.job.p).buf = s.job.bytes ∧
  (s.pages s.job.p).base = s.job.base ∧ (s.pages s.job.p).ver = s.job.ver ∧
  dirty (s.pages s.job.p) = true

@[simp] theorem pages_commit (c : Config P) (x : State P) (s : Sys P) (g : Ghost P) (n : String) :
    (commit c x s g n).s.pages = s.pages := rfl
@[simp] theorem pages_ackMail (s : Sys P) (p : Fin P) (ok : Bool) (v : Int) (t : Bool) :
    (ackMail s p ok v t).pages = s.pages := by cases ok <;> rfl
@[simp] theorem shapes_commit (c : Config P) (x : State P) (s : Sys P) (g : Ghost P) (n : String) :
    Shapes (commit c x s g n).s ↔ Shapes s := Iff.rfl
@[simp] theorem shapes_ackMail (s : Sys P) (p : Fin P) (ok : Bool) (v : Int) (t : Bool) :
    Shapes (ackMail s p ok v t) ↔ Shapes s := by cases ok <;> rfl

theorem shape_table (c : Config P) (mutant : c.mutant = "none") (pg : Page) (d v : Int)
    (h : PageShape pg) : PageShape (table c pg d v) := by
  simp only [table,mutant]
  simp only [show ("none" == "MQ") = false from rfl, show ("none" == "MLO") = false from rfl,
    Bool.false_and,Bool.or_false,Bool.false_eq_true,ite_false]
  split <;> (try split) <;> (try split)
  all_goals intro ht hr
  all_goals first | exact h ht hr | rfl | (simp_all [PageShape,clean])

theorem shape_submit (c : Config P) (mutant : c.mutant = "none") (pg : Page) (t bv v : Int) :
    PageShape (submitTo c pg t bv v) := by
  simp only [submitTo,mutant]
  simp only [show ("none" == "MIS") = false from rfl,Bool.false_eq_true,ite_false]
  split <;> simp [PageShape]

theorem shape_upOpen (c : Config P) (x : State P) (s : Sys P) (m : Up P) (ok : Bool)
    (h : Shapes s) : Shapes (upOpen c x s m ok).s := by
  apply (shapes_commit c x _ _ _).mpr; apply (shapes_ackMail _ _ _ _ _).mpr
  apply property_put _ _ _ PageShape h
  split
  · exact h m.p
  · intro _ _; rfl

theorem shape_upClose (c : Config P) (x : State P) (s : Sys P) (m : Up P)
    (h : Shapes s) : Shapes (upClose c x s m).s := by
  apply (shapes_commit c x _ _ _).mpr; split
  · exact property_put _ _ _ PageShape h (by intro _ _; rfl)
  · exact h

theorem shape_upDiscard (c : Config P) (x : State P) (s : Sys P) (m : Up P) (ok : Bool)
    (h : Shapes s) : Shapes (upDiscard c x s m ok).s := by
  simp only [upDiscard]; split <;> (try split)
  all_goals apply (shapes_commit c x _ _ _).mpr
  · exact h
  · exact (shapes_ackMail _ _ _ _ _).mpr h
  · apply (shapes_ackMail _ _ _ _ _).mpr
    exact property_put _ _ _ PageShape h (by intro _ _; rfl)

theorem shape_upSubmit (c : Config P) (mutant : c.mutant = "none")
    (x : State P) (s : Sys P) (m : Up P) (h : Shapes s) : Shapes (upSubmit c x s m).s := by
  simp only [upSubmit,mutant]
  simp only [show ("none" == "MDO") = false from rfl,show ("none" != "MIS") = true from rfl,
    Bool.and_true,Bool.false_eq_true,ite_false]
  split
  · exact h
  · apply (shapes_commit c x _ _ _).mpr; apply (shapes_ackMail _ _ _ _ _).mpr
    apply property_put _ _ _ PageShape h
    split
    · simp [PageShape]
    · exact shape_submit c mutant _ _ _ _

theorem shape_upOp (c : Config P) (mutant : c.mutant = "none")
    (x : State P) (s : Sys P) (m : Up P) (ok : Bool) (h : Shapes s) : Shapes (upOp c x s m ok).s := by
  simp only [upOp]; split <;> (try split)
  all_goals apply (shapes_commit c x _ _ _).mpr
  · exact h
  · apply (shapes_ackMail _ _ _ _ _).mpr; apply (shapes_ackMail _ _ _ _ _).mpr; exact h
  · apply (shapes_ackMail _ _ _ _ _).mpr; apply (shapes_ackMail _ _ _ _ _).mpr
    apply property_put _ _ _ PageShape
    · apply property_put _ _ _ PageShape h
      simp [PageShape]
    · exact shape_submit c mutant _ _ _ _

theorem snapshot_deliver_locked (c : Config P) (mutant : c.mutant = "none")
    (x : State P) (m : Up P) (tail : List (Up P)) (ok : Bool)
    (hen : (!locked x.s m.p && (m.kind != "op" || !locked x.s m.q)) = true)
    (hon : x.s.job.on = true) :
    (deliverUp c x m tail ok).s.pages x.s.job.p = x.s.pages x.s.job.p := by
  have hp : x.s.job.p ≠ m.p := by simp [locked,hon] at hen; grind
  have hq : m.kind = "op" → x.s.job.p ≠ m.q := by simp [locked,hon] at hen; grind
  simp only [deliverUp]; split <;> (try split) <;> (try split) <;> (try split)
  all_goals dsimp only [upOpen,upClose,upDiscard,upSubmit,upOp]
  all_goals try simp only [mutant]
  all_goals try simp only [show ("none" == "MDO") = false from rfl,Bool.false_eq_true,ite_false]
  all_goals (try split) <;> (try split) <;> (try split)
  all_goals simp only [pages_commit,pages_ackMail]
  all_goals first | rfl | (simp [put,hp]) | (have he : m.kind = "op" := by simpa using (by assumption : (m.kind == "op") = true); simp [put,hp,hq he])
  all_goals grind

theorem snapshot_execute (c : Config P) (mutant : c.mutant = "none")
    (x : State P) (a : Action P) (h : JobSnapshot x.s) (hi : InactivePages x)
    (hen : enabled c x a = true) : JobSnapshot (execute c x a).s := by
  cases a <;> simp only [execute]
  case deliverUp ok =>
    cases hq : x.s.up with
    | nil => exact h
    | cons m tail =>
      intro hon
      rw [job_deliverUp] at hon ⊢
      have he : x.s.alive = true ∧ (!locked x.s m.p && (m.kind != "op" || !locked x.s m.q)) = true := by
        simpa only [enabled,hq,Bool.and_eq_true] using hen
      rw [snapshot_deliver_locked c mutant x m tail ok he.2 hon]
      exact h hon
  case observe p =>
    intro hon
    have hj : x.s.job.on = true := hon
    have hp : x.s.job.p ≠ p := by
      simp [enabled,locked,hj] at hen; grind
    simpa [JobSnapshot,observe,put,hp] using h hj
  case flush p =>
    intro _
    have hh : (x.s.pages p).held = true := by simp [enabled] at hen; grind
    have hd : dirty (x.s.pages p) = true := by simp [enabled] at hen; grind
    exact ⟨hh,rfl,rfl,rfl,hd⟩
  case check =>
    simp only [check]; split
    · simp [JobSnapshot,job_commit,nojob]
    · exact h
  case rename => exact h
  case dirSync ok => simp [JobSnapshot,job_commit,nojob,dirSync]
  case saveFail => simp [JobSnapshot,job_commit,nojob,saveFail]
  case switchReq =>
    intro hon
    have old := h hon
    have go : ((x.s.pages x.s.job.p).held && dirty (x.s.pages x.s.job.p)) = true := by simp [old.1,old.2.2.2.2]
    simp only [switchReq,commit,go,ite_true]
    exact ⟨old.1,old.2.1,old.2.2.1,old.2.2.2.1,by simp [dirty,clean]⟩
  case launch =>
    intro hon
    have dead : x.s.alive = false := by simpa [enabled] using hen
    have hh := (h hon).1
    have hf := hi dead x.s.job.p
    simp_all
  case switchFin => simp [JobSnapshot,switchFin,down,job_commit,nojob]
  case crash => simp [JobSnapshot,crash,down,job_commit,nojob]
  case power keep => simp [JobSnapshot,power,down,job_commit,nojob]
  case draftSync p => simp only [draftSync]; split <;> (try split) <;> (try split) <;> exact h
  all_goals exact h

theorem snapshot_reachable (c : Config P) (mutant : c.mutant = "none")
    (x : State P) (hx : Reachable c x) : JobSnapshot x.s := by
  induction hx with
  | initial => simp [JobSnapshot,init,nojob]
  | @advance x y hr a hs ih =>
    have hi := inactive_reachable c x hr
    unfold step at hs; split at hs
    · cases hs; exact snapshot_execute c mutant x a ih hi (by assumption)
    · contradiction

theorem shapes_execute (c : Config P) (mutant : c.mutant = "none")
    (x : State P) (a : Action P) (h : Shapes x.s) (hj : JobSnapshot x.s)
    (hen : enabled c x a = true) : Shapes (execute c x a).s := by
  cases a <;> simp only [execute]
  case deliverUp ok =>
    cases hq : x.s.up with
    | nil => exact h
    | cons m tail =>
      simp only [deliverUp]; split <;> (try split) <;> (try split) <;> (try split)
      all_goals first
        | exact shape_upOpen c x _ m ok h
        | exact shape_upClose c x _ m h
        | exact shape_upDiscard c x _ m ok h
        | exact shape_upSubmit c mutant x _ m h
        | exact shape_upOp c mutant x _ m ok h
  case observe p =>
    exact property_put _ _ _ PageShape h (shape_table c mutant _ _ _ (h p))
  case check =>
    simp only [check]; split
    · exact property_put _ _ _ PageShape h (shape_table c mutant _ _ _ (h _))
    · exact h
  case dirSync ok =>
    have hon : x.s.job.on = true := by simp [enabled] at hen; grind
    have hb := (hj hon).2.1
    apply property_put _ _ _ PageShape h
    simp only [dirSync,mutant]
    simp only [show ("none" == "MRE") = false from rfl,Bool.and_false,Bool.false_eq_true,ite_false]
    cases ok
    · simp [onReply,mutant,PageShape]
    · simp [onReply,PageShape,hb]
  case saveFail =>
    apply property_put _ _ _ PageShape h
    simp [onReply,mutant,PageShape]
  case switchReq =>
    intro p; simp only [switchReq,pages_commit]; split
    · simp [PageShape]
    · exact h p
  case launch =>
    intro p; simp only [launch,pages_commit,restored]; split <;> simp [PageShape]
  case switchFin => simp [Shapes,switchFin,pages_commit,down,PageShape]
  case crash => simp [Shapes,crash,pages_commit,down,PageShape]
  case power keep => simp [Shapes,power,pages_commit,down,PageShape]
  case draftSync p => simp only [draftSync]; split <;> (try split) <;> (try split) <;> exact h
  all_goals exact h

theorem shapes_reachable (c : Config P) (mutant : c.mutant = "none")
    (x : State P) (hx : Reachable c x) : Shapes x.s := by
  induction hx with
  | initial => simp [Shapes,PageShape,init]
  | @advance x y hr a hs ih =>
    have hj := snapshot_reachable c mutant x hr
    unfold step at hs; split at hs
    · cases hs; exact shapes_execute c mutant x a ih hj (by assumption)
    · contradiction

theorem dirty_flags (pg : Page) (h : PageShape pg) (hd : dirty pg = true) :
    pg.typed = true ∨ pg.risk = true := by
  cases ht : pg.typed <;> cases hr : pg.risk <;> simp_all [dirty,clean]
#print axioms snapshot_reachable
#print axioms shapes_reachable
end Storage
