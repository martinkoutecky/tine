import CoreWindow21
namespace Storage21
set_option maxRecDepth 8192
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false

theorem mem_union_general [BEq α] [LawfulBEq α] (a b : List α) (t : α) :
    t ∈ union a b ↔ t ∈ a ∨ t ∈ b := by
  induction b generalizing a with
  | nil => simp [union]
  | cons v vs ih =>
    simp only [union,List.foldl_cons]
    change t ∈ union (if a.contains v then a else a ++ [v]) vs ↔ _
    rw [ih]
    split <;> simp_all <;> grind

abbrev QueueCarries (up : List (Up P)) (owed : List (Fin P × Int)) : Prop :=
  ∀ o ∈ owed, ∃ m ∈ up, o ∈ carries m
abbrev Carriage (x : State P) : Prop := QueueCarries x.s.up x.g.owed

@[simp] theorem carriage_commit (c : Config P) (x : State P) (s : Sys P) (g : Ghost P) (n : String) :
    Carriage (commit c x s g n) ↔ QueueCarries s.up g.owed := Iff.rfl
@[simp] theorem up_ackMail (s : Sys P) (p : Fin P) (ok : Bool) (v : Int) (t : Bool) :
    (ackMail s p ok v t).up = s.up := by cases ok <;> rfl

theorem carriage_append (up : List (Up P)) (owed added : List (Fin P × Int)) (m : Up P)
    (h : QueueCarries up owed) (hc : ∀ o ∈ added, o ∈ carries m) :
    QueueCarries (up ++ [m]) (union owed added) := by
  intro o ho
  rcases (mem_union_general _ _ _).mp ho with old | new
  · obtain ⟨u,hu,hc⟩ := h o old
    exact ⟨u,by simp [hu],hc⟩
  · exact ⟨m,by simp,hc o new⟩

theorem carriage_append_noowed (up : List (Up P)) (owed : List (Fin P × Int)) (m : Up P)
    (h : QueueCarries up owed) : QueueCarries (up ++ [m]) owed := by
  intro o ho; obtain ⟨u,hu,hc⟩ := h o ho; exact ⟨u,by simp [hu],hc⟩

theorem carriage_pop (up : List (Up P)) (owed removed : List (Fin P × Int))
    (m : Up P) (tail : List (Up P)) (hq : up = m::tail)
    (h : QueueCarries up owed) (hc : ∀ o ∈ carries m, o ∈ removed) :
    QueueCarries tail (exclude owed removed) := by
  intro o ho
  have old : o ∈ owed := (List.mem_filter.mp ho).1
  have nrem : o ∉ removed := by simpa [exclude] using (List.mem_filter.mp ho).2
  obtain ⟨u,hu,hcarry⟩ := h o old
  rw [hq,List.mem_cons] at hu
  rcases hu with rfl | hu
  · exact False.elim (nrem (hc o hcarry))
  · exact ⟨u,hu,hcarry⟩

theorem carriage_pop_empty (up : List (Up P)) (owed : List (Fin P × Int))
    (m : Up P) (tail : List (Up P)) (hq : up = m::tail) (h : QueueCarries up owed)
    (hc : carries m = []) : QueueCarries tail owed := by
  simpa [exclude] using carriage_pop up owed [] m tail hq h (by simp [hc])

def HeadHeld (x : State P) : Prop := ∀ m tail, x.s.up = m::tail →
  (m.kind = "submit" → (x.s.pages m.p).held = true) ∧
  (m.kind = "op" → (x.s.pages m.p).held = true ∧ (x.s.pages m.q).held = true)

theorem carriage_deliverUp (c : Config P) (x : State P) (m : Up P) (tail : List (Up P)) (ok : Bool)
    (h : Carriage x) (hq : x.s.up = m::tail) (hh : HeadHeld x) (kind : m.kind ∈ ["open","close","discard","submit","op"]) :
    Carriage (deliverUp c x m tail ok) := by
  have held := hh m tail hq
  have empty : m.kind ≠ "submit" → m.kind ≠ "op" → QueueCarries tail x.g.owed := by
    intro ns no
    apply carriage_pop_empty x.s.up x.g.owed m tail hq h
    simp [carries,ns,no]
  have submitTail : m.kind = "submit" → QueueCarries tail (exclude x.g.owed [(m.p,m.t)]) := by
    intro ks
    exact carriage_pop x.s.up x.g.owed [(m.p,m.t)] m tail hq h (by simp [carries,ks])
  have opTail : m.kind = "op" → QueueCarries tail (exclude x.g.owed [(m.p,m.t),(m.q,m.t2)]) := by
    intro ko
    exact carriage_pop x.s.up x.g.owed [(m.p,m.t),(m.q,m.t2)] m tail hq h (by simp [carries,ko])
  simp only [deliverUp]; split <;> (try split) <;> (try split) <;> (try split)
  · have km : m.kind = "open" := by simpa using (by assumption : (m.kind == "open") = true)
    simp only [upOpen,carriage_commit,up_ackMail]
    split <;> (try split) <;> exact empty (by simp [km]) (by simp [km])
  · have km : m.kind = "close" := by simpa using (by assumption : (m.kind == "close") = true)
    simp only [upClose,carriage_commit]; split <;> exact empty (by simp [km]) (by simp [km])
  · have km : m.kind = "discard" := by simpa using (by assumption : (m.kind == "discard") = true)
    simp only [upDiscard]; split <;> (try split)
    all_goals simp only [carriage_commit,up_ackMail]
    all_goals exact empty (by simp [km]) (by simp [km])
  · have km : m.kind = "op" := by simpa using (by assumption : (m.kind == "op") = true)
    have pg := held.2 km
    simp only [upOp,pg.1,pg.2,Bool.not_true,Bool.false_or,Bool.false_eq_true,ite_false]
    split <;> simp only [carriage_commit,up_ackMail] <;> exact opTail km
  · have km : m.kind = "submit" := by
      have kinds := kind
      simp_all
    have pg := held.1 km
    simp only [upSubmit,pg,Bool.not_true,Bool.false_eq_true,ite_false,carriage_commit,up_ackMail]
    exact submitTail km

theorem carriage_execute_non_delivery (c : Config P) (mutant : c.mutant = "none")
    (x : State P) (a : Action P) (h : Carriage x) (excluded : ∀ ok, a ≠ .deliverUp ok) :
    Carriage (execute c x a) := by
  cases a <;> simp only [execute]
  case deliverUp ok => exact False.elim (excluded ok rfl)
  case wOpen p => exact carriage_append_noowed _ _ _ h
  case wClose p => exact carriage_append_noowed _ _ _ h
  case wDiscard p => exact carriage_append_noowed _ _ _ h
  case wSend p =>
    exact carriage_append _ _ _ _ h (by intro o ho; simpa [carries,up1] using ho)
  case wResolve p v =>
    exact carriage_append _ _ _ _ h (by intro o ho; simpa [carries,up1] using ho)
  case wOp p q t u =>
    exact carriage_append _ _ _ _ h (by intro o ho; simpa [carries] using ho)
  case windowCrash =>
    simp only [windowCrash,mutant]
    simp only [show ("none" == "MDQ") = false from rfl,Bool.false_eq_true,ite_false]
    intro o ho; obtain ⟨m,hm,hc⟩ := h o ho
    exact ⟨{m with cur := false},List.mem_map.mpr ⟨m,hm,rfl⟩,hc⟩
  case crash => simp [Carriage,QueueCarries,crash,commit,gDown]
  case power keep => simp [Carriage,QueueCarries,power,commit,gDown]
  case switchFin => simp [Carriage,QueueCarries,switchFin,commit,gDown]
  case check => simp only [check]; split <;> exact h
  case draftSync p => simp only [draftSync]; split <;> (try split) <;> exact h
  all_goals exact h

theorem accepted_of_carriage (x : State P) (h : Carriage x) : accepted x = true := by
  simp only [accepted,Bool.or_eq_true]; right
  apply List.all_eq_true.mpr
  intro o ho
  obtain ⟨m,hm,hc⟩ := h o ho
  exact List.any_eq_true.mpr ⟨m,hm,by simpa using hc⟩


end Storage21
