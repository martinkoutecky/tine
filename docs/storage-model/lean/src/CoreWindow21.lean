import CoreC21
namespace Storage21
set_option maxRecDepth 8192
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false

def WInv (x : State P) : Prop := "B'-window-replaced" ∉ x.g.bad

theorem W_commit (c : Config P) (x : State P) (s : Sys P) (g : Ghost P) (n : String)
    (hb : "B'-window-replaced" ∉ g.bad)
    (hw : ∀ p, keptW x p s g.owed n = true) : WInv (commit c x s g n) := by
  have all : (ids P).all (fun p => keptW x p s g.owed n) = true := List.all_eq_true.mpr (fun p _ => hw p)
  change "B'-window-replaced" ∉ union (union g.bad (tags _ "B-unsaved-replaced")) (tags (!(ids P).all (fun p => keptW x p s g.owed n)) "B'-window-replaced")
  rw [all]; simpa using hb

theorem W_commit_same (c : Config P) (x : State P) (s : Sys P) (g : Ghost P) (n : String)
    (hb : "B'-window-replaced" ∉ g.bad) (hw : s.w = x.s.w) : WInv (commit c x s g n) := by
  apply W_commit c x s g n hb
  intro p; simp [keptW,hw]

theorem W_commit_exempt (c : Config P) (x : State P) (s : Sys P) (g : Ghost P) (n : String)
    (hb : "B'-window-replaced" ∉ g.bad)
    (hn : n ∈ ["wEdit","wResolve","wOp","windowCrash","crash","power"]) : WInv (commit c x s g n) := by
  apply W_commit c x s g n hb
  intro p; simp [keptW,hn]

@[simp] theorem w_ackMail (s : Sys P) (p : Fin P) (ok : Bool) (v : Int) (t : Bool) :
    (ackMail s p ok v t).w = s.w := by cases ok <;> rfl

theorem W_upOpen (c : Config P) (x : State P) (m : Up P) (tail : List (Up P)) (ok : Bool)
    (h : WInv x) : WInv (upOpen c x {x.s with up := tail} m ok) := by
  simp only [upOpen]; split <;> (try split)
  all_goals apply W_commit_same
  all_goals first | exact h | simp only [w_ackMail]

theorem W_upClose (c : Config P) (x : State P) (m : Up P) (tail : List (Up P))
    (h : WInv x) : WInv (upClose c x {x.s with up := tail} m) := by
  simp only [upClose]; split <;> exact W_commit_same c x _ _ _ h rfl

theorem W_upDiscard (c : Config P) (x : State P) (m : Up P) (tail : List (Up P)) (ok : Bool)
    (h : WInv x) : WInv (upDiscard c x {x.s with up := tail} m ok) := by
  simp only [upDiscard]; split <;> (try split)
  all_goals apply W_commit_same
  all_goals first | exact h | simpa [WInv] using h | simp only [w_ackMail]

theorem W_upSubmit (c : Config P) (x : State P) (m : Up P) (tail : List (Up P))
    (h : WInv x) : WInv (upSubmit c x {x.s with up := tail} m) := by
  simp only [upSubmit]; split
  all_goals apply W_commit_same
  all_goals first | exact h | simpa [WInv] using h | simp only [w_ackMail]

theorem W_upOp (c : Config P) (x : State P) (m : Up P) (tail : List (Up P)) (ok : Bool)
    (h : WInv x) : WInv (upOp c x {x.s with up := tail} m ok) := by
  simp only [upOp]; split <;> (try split)
  all_goals apply W_commit_same
  all_goals first | exact h | simpa [WInv] using h | simp only [w_ackMail]

theorem W_deliverUp (c : Config P) (x : State P) (m : Up P) (tail : List (Up P)) (ok : Bool)
    (h : WInv x) : WInv (deliverUp c x m tail ok) := by
  simp only [deliverUp]; split <;> (try split) <;> (try split) <;> (try split)
  all_goals first
    | exact W_upOpen c x m tail ok h
    | exact W_upClose c x m tail h
    | exact W_upDiscard c x m tail ok h
    | exact W_upOp c x m tail ok h
    | exact W_upSubmit c x m tail h

theorem W_execute_nonrecv (c : Config P) (x : State P) (a : Action P) (h : WInv x)
    (hen : enabled c x a = true) (excluded : ∀ p, a ≠ .wRecv p) : WInv (execute c x a) := by
  cases a <;> simp only [execute]
  case wRecv p => exact False.elim (excluded p rfl)
  case deliverUp ok =>
    cases hq : x.s.up with
    | nil => exact h
    | cons m tail => exact W_deliverUp c x m tail ok h
  case wOpen p =>
    have off : (x.s.w p).on = false := by simp [enabled] at hen; grind
    apply W_commit
    · exact h
    · intro q; by_cases he : q = p
      · subst q; simp [keptW,off]
      · simp [keptW,put,he]
  case wClose p =>
    have pend : (x.s.w p).pend = false := by simp [enabled] at hen; grind
    have sent : (x.s.w p).sent = false := by simp [enabled] at hen; grind
    apply W_commit
    · exact h
    · intro q; by_cases he : q = p
      · subst q; simp [keptW,pend,sent]
      · simp [keptW,put,he]
  case wSend p =>
    apply W_commit
    · exact h
    · intro q; by_cases he : q = p <;> simp_all [keptW,put]
  case wDiscard p =>
    apply W_commit
    · exact h
    · intro q; by_cases he : q = p <;> simp_all [keptW,put]
  case switchFin =>
    have rows : ∀ p, (x.s.w p).pend = false ∧ (x.s.w p).sent = false := by
      have all : (ids P).all (fun p => !(x.s.w p).pend && !(x.s.w p).sent &&
        (!(x.s.pages p).held || clean (x.s.pages p) || (x.s.pages p).risk) &&
        (x.s.drafts p == draftEntry (x.s.pages p) || (c.mutant == "MX" && (x.s.pages p).held && (x.s.pages p).risk))) = true := by
        simp only [enabled,canSwitch,Bool.and_eq_true] at hen; exact hen.2
      intro p
      have hp := List.all_eq_true.mp all p (by simp [ids])
      simp only [Bool.and_eq_true,Bool.not_eq_true] at hp
      simpa using hp.1.1
    apply W_commit
    · exact h
    · intro p; simp [keptW,down,(rows p).1,(rows p).2]
  case wEdit p v => exact W_commit_exempt c x _ _ _ h (by simp)
  case wResolve p v => exact W_commit_exempt c x _ _ _ h (by simp)
  case wOp p q t u => exact W_commit_exempt c x _ _ _ h (by simp)
  case windowCrash => exact W_commit_exempt c x _ _ _ h (by simp)
  case crash => exact W_commit_exempt c x _ _ _ h (by simp)
  case power keep => exact W_commit_exempt c x _ _ _ h (by simp)
  case check => simp only [check]; split <;> exact W_commit_same c x _ _ _ h rfl
  case draftSync p => simp only [draftSync]; split <;> (try split) <;> exact W_commit_same c x _ _ _ h rfl
  case rename => apply W_commit_same; simpa [WInv] using h; rfl
  all_goals exact W_commit_same c x _ _ _ h rfl

-- Exact conditions needed only in the replacing acknowledgement branches.
def MailFence (x : State P) : Prop := ∀ p,
  (x.s.mb p).ack ≥ 0 → (x.s.w p).sent = true →
    ((x.s.mb p).held = false → (x.s.w p).pend = false) ∧
    ((x.s.w p).pend = false → (p,(x.s.w p).text) ∉ x.g.owed)

theorem W_wRecv_of_fence (c : Config P) (mutant : c.mutant = "none")
    (x : State P) (p : Fin P) (h : WInv x) (hf : MailFence x) : WInv (wRecv c x p) := by
  simp only [wRecv,mutant]
  simp only [show ("none" == "MAK") = false from rfl,show ("none" == "MWA") = false from rfl,
    Bool.or_false]
  apply W_commit
  · exact h
  · intro q; by_cases he : q = p
    · subst q
      by_cases isAck : ((x.s.mb p).ack >= 0 && (x.s.w p).sent) = true
      · have acks : (x.s.mb p).ack ≥ 0 ∧ (x.s.w p).sent = true := by simpa using isAck
        have fence := hf p acks.1 acks.2
        by_cases held : (x.s.mb p).held = true
        · by_cases pend : (x.s.w p).pend = true
          · simp [keptW,put,isAck,held,pend]
          · have np : (x.s.w p).pend = false := by simpa using pend
            simp [keptW,put,isAck,held,pend,fence.2 np]
        · have nh : (x.s.mb p).held = false := by simpa using held
          have np := fence.1 nh
          simp [keptW,put,isAck,held,np,fence.2 np]
      · by_cases sent : (x.s.w p).sent = true
        · have ackNeg : ¬ (x.s.mb p).ack ≥ 0 := by simpa [sent] using isAck
          by_cases on : (x.s.w p).on = true <;> simp [keptW,put,isAck,sent,ackNeg,on]
        · by_cases pend : (x.s.w p).pend = true
          · by_cases on : (x.s.w p).on = true <;> simp [keptW,put,isAck,sent,pend,on]
          · simp [keptW,put,isAck,sent,pend]
    · simp [keptW,put,he]


end Storage21
