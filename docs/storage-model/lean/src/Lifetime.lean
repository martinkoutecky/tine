import Accepted
namespace Storage
set_option maxRecDepth 8192
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false

def Touch (m : Up P) (p : Fin P) : Prop := m.p = p ∨ (m.kind = "op" ∧ m.q = p)
def Barrier (m : Up P) (p : Fin P) : Prop := (m.kind = "open" ∨ m.kind = "close") ∧ m.p = p
def Needs (m : Up P) (p : Fin P) : Prop :=
  ((m.kind = "submit" ∨ m.kind = "discard") ∧ m.p = p) ∨
  (m.kind = "op" ∧ (m.p = p ∨ m.q = p))
def Current (m : Up P) (p : Fin P) : Prop := m.cur = true ∧ m.kind ≠ "close" ∧ Touch m p

structure Lifetime (s : Sys P) : Prop where
  ordered : s.up.Pairwise (fun a b => ∀ p, Barrier a p → ¬ Needs b p)
  last : s.up.Pairwise (fun a b => ∀ p, Current a p → ¬ Touch b p)
  held : ∀ m ∈ s.up, ∀ p, Needs m p → (s.pages p).held = true
  active : ∀ p, (s.w p).on = true → (s.pages p).held = true ∧
    ∀ m ∈ s.up, Touch m p → m.cur = true ∧ ¬ Barrier m p
  pending : ∀ p, (s.w p).pend = true → (s.w p).on = true
  waiting : ∀ m ∈ s.up, ∀ p, Current m p → (s.w p).sent = true ∧ (s.mb p).ack < 0
  done : ∀ p, (s.mb p).ack ≥ 0 → (s.w p).sent = true ∧ ∀ m ∈ s.up, ¬ Touch m p
  mailOn : ∀ p, (s.mb p).ack ≥ 0 → (s.mb p).on = true
  mailHeld : ∀ p, (s.mb p).ack ≥ 0 → (s.mb p).held = (s.pages p).held

theorem lifetime_commit (c : Config P) (x : State P) (s : Sys P) (g : Ghost P) (n : String)
    (h : Lifetime s) : Lifetime (commit c x s g n).s := by
  refine ⟨h.ordered,h.last,h.held,h.active,h.pending,?_,?_,?_,?_⟩
  · intro m hm p hc
    obtain ⟨sent,neg⟩ := h.waiting m hm p hc
    constructor
    · exact sent
    · change (if _ then _ else s.mb p).ack < 0
      split
      · split <;> (first | exact neg | (dsimp only; omega))
      · exact neg
  · intro p pos
    change (if _ then _ else s.mb p).ack ≥ 0 at pos
    split at pos
    · split at pos
      · exact h.done p pos
      · dsimp only at pos; omega
    · exact h.done p pos
  · intro p pos
    change (if _ then _ else s.mb p).ack ≥ 0 at pos
    change (if _ then _ else s.mb p).on = true
    split <;> simp_all only
    exact h.mailOn p (by simpa using pos)
  · intro p pos
    change (if _ then _ else s.mb p).ack ≥ 0 at pos
    change (if _ then _ else s.mb p).held = (s.pages p).held
    split <;> simp_all only
    exact h.mailHeld p (by simpa using pos)

theorem lifetime_frame (s t : Sys P) (h : Lifetime s)
    (pages : ∀ p, (t.pages p).held = (s.pages p).held)
    (w : t.w = s.w) (mb : t.mb = s.mb) (up : t.up = s.up) : Lifetime t := by
  refine ⟨?_,?_,?_,?_,?_,?_,?_,?_,?_⟩
  · simpa only [up] using h.ordered
  · simpa only [up] using h.last
  · simpa only [up,pages] using h.held
  · simpa only [up,pages,w] using h.active
  · simpa only [w] using h.pending
  · simpa only [up,w,mb] using h.waiting
  · simpa only [up,w,mb] using h.done
  · simpa only [mb] using h.mailOn
  · simpa only [mb,pages] using h.mailHeld

theorem lifetime_init (c : Config P) : Lifetime (init c).s := by
  constructor <;> simp [init]

theorem lifetime_headHeld (x : State P) (h : Lifetime x.s) : HeadHeld x := by
  intro m tail hq
  have hm : m ∈ x.s.up := by simp [hq]
  constructor
  · intro k; exact h.held m hm m.p (by simp [Needs,k])
  · intro k
    exact ⟨h.held m hm m.p (by simp [Needs,k]),h.held m hm m.q (by simp [Needs,k])⟩

theorem lifetime_no_owed (x : State P) (h : Lifetime x.s) (hc : Carriage x)
    (p : Fin P) (ack : (x.s.mb p).ack ≥ 0) (t : Int) : (p,t) ∉ x.g.owed := by
  intro owed
  obtain ⟨m,hm,carry⟩ := hc (p,t) owed
  have touch : Touch m p := by
    simp only [carries] at carry
    split at carry
    · simp only [List.mem_singleton,Prod.mk.injEq] at carry
      exact Or.inl carry.1.symm
    · split at carry
      · simp only [List.mem_cons,List.mem_singleton,List.not_mem_nil,or_false,Prod.mk.injEq] at carry
        rcases carry with carry | carry
        · exact Or.inl carry.1.symm
        · exact Or.inr ⟨by simpa using (by assumption : (m.kind == "op") = true),carry.1.symm⟩
      · simp at carry
  exact (h.done p ack).2 m hm touch

theorem lifetime_mailFence (x : State P) (h : Lifetime x.s) (hc : Carriage x) : MailFence x := by
  intro p ack sent
  constructor
  · intro unheld
    cases pend : (x.s.w p).pend
    · rfl
    · have po := h.pending p pend
      have held := (h.active p po).1
      have eq := h.mailHeld p ack
      simp_all
  · intro _; exact lifetime_no_owed x h hc p ack _
end Storage
