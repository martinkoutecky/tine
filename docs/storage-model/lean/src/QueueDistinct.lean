import CustodyDelivery
namespace Storage
set_option maxRecDepth 8192
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false

def QueueDistinct (s : Sys P) : Prop := ∀ m ∈ s.up, m.kind = "op" → m.p ≠ m.q

theorem distinct_commit (c : Config P) (x : State P) (s : Sys P) (g : Ghost P) (n : String)
    (h : QueueDistinct s) : QueueDistinct (commit c x s g n).s := h

theorem distinct_ackMail (s : Sys P) (p : Fin P) (ok : Bool) (v : Int) (took : Bool)
    (h : QueueDistinct s) : QueueDistinct (ackMail s p ok v took) := by cases ok <;> exact h

theorem distinct_delivery (c : Config P) (x : State P) (m : Up P) (tail : List (Up P))
    (ok : Bool) (h : QueueDistinct {x.s with up := tail}) :
    QueueDistinct (deliverUp c x m tail ok).s := by
  simp only [deliverUp]; split <;> (try split) <;> (try split) <;> (try split)
  all_goals simp only [upOpen,upClose,upDiscard,upOp,upSubmit]
  all_goals (try split) <;> (try split)
  all_goals apply distinct_commit
  all_goals first | exact h | skip
  all_goals (try apply distinct_ackMail) <;> (try apply distinct_ackMail)
  all_goals first | exact h | (split <;> exact h)

theorem distinct_execute (c : Config P) (x : State P) (a : Action P)
    (h : QueueDistinct x.s) (hen : enabled c x a = true) : QueueDistinct (execute c x a).s := by
  cases a <;> simp only [execute]
  case deliverUp ok =>
    cases hq : x.s.up with
    | nil => exact h
    | cons m tail =>
      exact distinct_delivery c x m tail ok (fun u hu => h u (by simp [hq,hu]))
  case wOp p q t u =>
    have ne : p ≠ q := by simp [enabled] at hen; grind
    apply distinct_commit
    intro m hm kind
    simp only [List.mem_append,List.mem_singleton] at hm
    rcases hm with hm | rfl
    · exact h m hm kind
    · exact ne
  case wOpen p =>
    apply distinct_commit
    intro m hm kind
    simp only [List.mem_append,List.mem_singleton] at hm
    rcases hm with hm | rfl
    · exact h m hm kind
    · simp [up1] at kind
  case wSend p =>
    apply distinct_commit
    intro m hm kind
    simp only [List.mem_append,List.mem_singleton] at hm
    rcases hm with hm | rfl
    · exact h m hm kind
    · simp [up1] at kind
  case wResolve p v =>
    apply distinct_commit
    intro m hm kind
    simp only [List.mem_append,List.mem_singleton] at hm
    rcases hm with hm | rfl
    · exact h m hm kind
    · simp [up1] at kind
  case wDiscard p =>
    apply distinct_commit
    intro m hm kind
    simp only [List.mem_append,List.mem_singleton] at hm
    rcases hm with hm | rfl
    · exact h m hm kind
    · simp [up1] at kind
  case wClose p =>
    apply distinct_commit
    intro m hm kind
    simp only [List.mem_append,List.mem_singleton] at hm
    rcases hm with hm | rfl
    · exact h m hm kind
    · simp [up1] at kind
  case windowCrash =>
    apply distinct_commit
    simp only [QueueDistinct]; split
    · simp
    · intro m hm kind
      obtain ⟨u,hu,rfl⟩ := List.mem_map.mp hm
      exact h u hu kind
  case check => simp only [check]; split <;> apply distinct_commit <;> exact h
  case draftSync p => simp only [draftSync]; split <;> apply distinct_commit <;> exact h
  case switchFin => simp [QueueDistinct,switchFin,commit,down]
  case crash => simp [QueueDistinct,crash,commit,down]
  case power keep => simp [QueueDistinct,power,commit,down]
  all_goals apply distinct_commit
  all_goals exact h

theorem distinct_reachable (c : Config P) (x : State P) (hx : Reachable c x) : QueueDistinct x.s := by
  induction hx with
  | initial => simp [QueueDistinct,init]
  | @advance x y hr a hs ih =>
    unfold step at hs; split at hs
    · cases hs; exact distinct_execute c x a ih (by assumption)
    · contradiction

theorem custody_delivery (c : Config P) (mutant : c.mutant = "none")
    (x : State P) (ok : Bool) (hx : Reachable c x) (h : Custody c x) :
    Custody c (execute c x (.deliverUp ok)) := by
  have bounds := (lineage_reachable c mutant x hx).1
  have distinct := distinct_reachable c x hx
  simp only [execute]
  cases hq : x.s.up with
  | nil => exact h
  | cons m tail =>
    simp only [deliverUp]
    split
    · exact custody_upOpen c x m tail ok h
    · split
      · exact custody_upClose c mutant x m tail h
      · split
        · exact custody_upDiscard c mutant x m tail ok h
        · split
          · exact custody_upOp c mutant x m tail ok h bounds
              (distinct m (by simp [hq]) (by simpa using (by assumption : (m.kind == "op") = true)))
          · exact custody_upSubmit c mutant x m tail h

#print axioms distinct_reachable
#print axioms custody_delivery
end Storage
