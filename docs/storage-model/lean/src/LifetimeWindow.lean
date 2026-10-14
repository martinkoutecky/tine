import Lifetime
namespace Storage
set_option maxRecDepth 8192
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false

theorem lifetime_enqueue (s : Sys P) (h : Lifetime s) (m : Up P) (nw : Fin P → W)
    (cur : m.cur = true)
    (sent : ∀ p, Touch m p → (s.w p).sent = false)
    (needon : ∀ p, Needs m p → (s.w p).on = true)
    (newon : ∀ p, (nw p).on = true → (s.w p).on = true ∧ ¬ Barrier m p)
    (pend : ∀ p, (nw p).pend = true → (nw p).on = true)
    (newsent : ∀ p, m.kind ≠ "close" → Touch m p → (nw p).sent = true)
    (frame : ∀ p, ¬Touch m p → (nw p).sent = (s.w p).sent) :
    Lifetime {s with w := nw, up := s.up ++ [m]} := by
  constructor
  · apply List.pairwise_append.mpr
    refine ⟨h.ordered,by simp,?_⟩
    intro a ha b hb p ba nb
    simp only [List.mem_singleton] at hb; subst b
    have ta : Touch a p := Or.inl ba.2
    exact ((h.active p (needon p nb)).2 a ha ta).2 ba
  · apply List.pairwise_append.mpr
    refine ⟨h.last,by simp,?_⟩
    intro a ha b hb p ca tb
    simp only [List.mem_singleton] at hb; subst b
    have wait := (h.waiting a ha p ca).1
    have ns := sent p tb
    simp_all
  · intro a ha p na
    rcases List.mem_append.mp ha with ha | ha
    · exact h.held a ha p na
    · simp only [List.mem_singleton] at ha; subst a
      exact (h.active p (needon p na)).1
  · intro p hon
    obtain ⟨oldon,nb⟩ := newon p hon
    refine ⟨(h.active p oldon).1,?_⟩
    intro a ha ta
    rcases List.mem_append.mp ha with ha | ha
    · exact (h.active p oldon).2 a ha ta
    · simp only [List.mem_singleton] at ha; subst a; exact ⟨cur,nb⟩
  · exact pend
  · intro a ha p ca
    rcases List.mem_append.mp ha with ha | ha
    · have old := h.waiting a ha p ca
      have nt : ¬ Touch m p := by intro tb; have ns := sent p tb; simp_all
      exact ⟨by simpa [frame p nt] using old.1,old.2⟩
    · simp only [List.mem_singleton] at ha; subst a
      have neg : (s.mb p).ack < 0 := by
        by_cases pos : (s.mb p).ack ≥ 0
        · have done := (h.done p pos).1; have ns := sent p ca.2.2; simp_all
        · omega
      exact ⟨newsent p ca.2.1 ca.2.2,neg⟩
  · intro p pos
    have old := h.done p pos
    have nt : ¬ Touch m p := by intro tb; have ns := sent p tb; simp_all
    refine ⟨by simpa [frame p nt] using old.1,?_⟩
    intro a ha ta
    rcases List.mem_append.mp ha with ha | ha
    · exact old.2 a ha ta
    · simp only [List.mem_singleton] at ha; subst a; exact nt ta
  · exact h.mailOn
  · exact h.mailHeld

theorem lifetime_patch (s : Sys P) (h : Lifetime s) (p : Fin P) (nw : W) (nm : Mail)
    (active : nw.on = true → (s.pages p).held = true ∧
      ∀ m ∈ s.up, Touch m p → m.cur = true ∧ ¬Barrier m p)
    (pending : nw.pend = true → nw.on = true)
    (waiting : ∀ m ∈ s.up, Current m p → nw.sent = true ∧ nm.ack < 0)
    (done : nm.ack ≥ 0 → nw.sent = true ∧ ∀ m ∈ s.up, ¬Touch m p)
    (mail : nm.ack ≥ 0 → nm.on = true ∧ nm.held = (s.pages p).held) :
    Lifetime {s with w := put s.w p nw, mb := put s.mb p nm} := by
  refine ⟨h.ordered,h.last,h.held,?_,?_,?_,?_,?_,?_⟩
  · intro q hon; by_cases eq : q = p
    · subst q; exact active (by simpa [put] using hon)
    · exact h.active q (by simpa [put,eq] using hon)
  · intro q hp; by_cases eq : q = p
    · subst q; simpa [put] using pending (by simpa [put] using hp)
    · simpa [put,eq] using h.pending q (by simpa [put,eq] using hp)
  · intro m hm q hc; by_cases eq : q = p
    · subst q; simpa [put] using waiting m hm hc
    · simpa [put,eq] using h.waiting m hm q hc
  · intro q pos; by_cases eq : q = p
    · subst q; simpa [put] using done (by simpa [put] using pos)
    · simpa [put,eq] using h.done q (by simpa [put,eq] using pos)
  · intro q pos; by_cases eq : q = p
    · subst q; simpa [put] using (mail (by simpa [put] using pos)).1
    · simpa [put,eq] using h.mailOn q (by simpa [put,eq] using pos)
  · intro q pos; by_cases eq : q = p
    · subst q; simpa [put] using (mail (by simpa [put] using pos)).2
    · simpa [put,eq] using h.mailHeld q (by simpa [put,eq] using pos)

theorem lifetime_wEdit (c : Config P) (x : State P) (p : Fin P) (v : Int)
    (h : Lifetime x.s) (on : (x.s.w p).on = true) : Lifetime (wEdit c x p v).s := by
  apply lifetime_commit
  have raw := lifetime_patch x.s h p {x.s.w p with text := v, pend := true} (x.s.mb p)
    (fun _ => h.active p on) (fun _ => on)
    (fun m hm hc => h.waiting m hm p hc) (h.done p)
    (fun pos => ⟨h.mailOn p pos,h.mailHeld p pos⟩)
  simpa only [show put x.s.mb p (x.s.mb p) = x.s.mb from by funext q; by_cases eq : q = p <;> simp_all [put]] using raw

theorem lifetime_wOpen (c : Config P) (x : State P) (p : Fin P) 
    (h : Lifetime x.s) (hen : enabled c x (.wOpen p) = true) : Lifetime (wOpen c x p).s := by
  have ns : (x.s.w p).sent = false := by simp [enabled] at hen; grind
  have hpending := h.pending
  apply lifetime_commit
  apply lifetime_enqueue x.s h
  · rfl
  all_goals simp_all [Touch,Needs,Barrier,up1,put]
  all_goals grind

theorem lifetime_wSend (c : Config P) (x : State P) (p : Fin P) 
    (h : Lifetime x.s) (hen : enabled c x (.wSend p) = true) : Lifetime (wSend c x p).s := by
  have ns : (x.s.w p).sent = false := by simp [enabled] at hen; grind
  have on : (x.s.w p).on = true := by simp [enabled] at hen; grind
  have hpending := h.pending
  apply lifetime_commit
  apply lifetime_enqueue x.s h
  · rfl
  all_goals simp_all [Touch,Needs,Barrier,up1,put]
  all_goals grind

theorem lifetime_wResolve (c : Config P) (x : State P) (p : Fin P) (v : Int)
    (h : Lifetime x.s) (hen : enabled c x (.wResolve p v) = true) : Lifetime (wResolve c x p v).s := by
  have ns : (x.s.w p).sent = false := by simp [enabled] at hen; grind
  have on : (x.s.w p).on = true := by simp [enabled] at hen; grind
  have hpending := h.pending
  apply lifetime_commit
  apply lifetime_enqueue x.s h
  · rfl
  all_goals simp_all [Touch,Needs,Barrier,up1,put]
  all_goals grind

theorem lifetime_wDiscard (c : Config P) (x : State P) (p : Fin P) 
    (h : Lifetime x.s) (hen : enabled c x (.wDiscard p) = true) : Lifetime (wDiscard c x p).s := by
  have ns : (x.s.w p).sent = false := by simp [enabled] at hen; grind
  have on : (x.s.w p).on = true := by simp [enabled] at hen; grind
  have hpending := h.pending
  apply lifetime_commit
  apply lifetime_enqueue x.s h
  · rfl
  all_goals simp_all [Touch,Needs,Barrier,up1,put]
  all_goals grind

theorem lifetime_wClose (c : Config P) (x : State P) (p : Fin P) 
    (h : Lifetime x.s) (hen : enabled c x (.wClose p) = true) : Lifetime (wClose c x p).s := by
  have ns : (x.s.w p).sent = false := by simp [enabled] at hen; grind
  have on : (x.s.w p).on = true := by simp [enabled] at hen; grind
  have hpending := h.pending
  apply lifetime_commit
  apply lifetime_enqueue x.s h
  · rfl
  all_goals simp_all [Touch,Needs,Barrier,up1,put]
  all_goals grind

theorem lifetime_wOp (c : Config P) (x : State P) (p q : Fin P) (t u : Int)
    (h : Lifetime x.s) (hen : enabled c x (.wOp p q t u) = true) : Lifetime (wOp c x p q t u).s := by
  have hpq : p ≠ q := by simp [enabled] at hen; grind
  have ns : (x.s.w p).sent = false ∧ (x.s.w q).sent = false := by simp [enabled] at hen; grind
  have on : (x.s.w p).on = true ∧ (x.s.w q).on = true := by simp [enabled] at hen; grind
  have pend : (x.s.w p).pend = false ∧ (x.s.w q).pend = false := by simp [enabled] at hen; grind
  have hpending := h.pending
  apply lifetime_commit
  apply lifetime_enqueue x.s h
  · rfl
  all_goals simp_all [Touch,Needs,Barrier,put]
  all_goals grind

theorem lifetime_wRecv (c : Config P) (mutant : c.mutant = "none") (x : State P) (p : Fin P)
    (h : Lifetime x.s) : Lifetime (wRecv c x p).s := by
  have active := h.active p
  have pending := h.pending p
  have waiting := h.waiting
  have done := h.done p
  have held := h.mailHeld p
  simp only [wRecv,mutant]
  simp only [show ("none" == "MAK") = false from rfl,show ("none" == "MDA") = false from rfl,
    show ("none" == "MWA") = false from rfl,Bool.or_false]
  apply lifetime_commit
  apply lifetime_patch x.s h
  all_goals try simp only [show ({} : Mail).ack = -1 from rfl,show ({} : Mail).on = false from rfl,
    show ({} : Mail).held = false from rfl]
  all_goals (try split) <;> (try split) <;> (try split) <;> (try split) <;> (try split)
  all_goals simp_all
  all_goals grind

end Storage
