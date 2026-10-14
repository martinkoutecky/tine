import AnchorDelivery
namespace Storage
set_option maxRecDepth 8192
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false

theorem anchors_window_pre (x : State P) (w : Fin P → W) (up : List (Up P)) (g : Ghost P)
    (h : Anchors x) (ha : AuthMono x.g g)
    (hw : ∀ p, (w p).on = true → (x.s.pages p).held = true →
      (w p).bv = (x.s.pages p).ver → Auth g p (x.s.pages p).buf)
    (hu : QueueAnchors ⟨{x.s with up},g⟩) : Anchors ⟨{x.s with w,up},g⟩ :=
  ⟨hw,h.snapshot,fun p hon took held hv => ha _ _ (h.took p hon took held hv),hu⟩

theorem queue_append_control (x : State P) (g : Ghost P) (m : Up P) (h : QueueAnchors x)
    (ha : AuthMono x.g g) (hkind : m.kind = "open" ∨ m.kind = "close" ∨ m.kind = "discard") :
    QueueAnchors ⟨{x.s with up := x.s.up ++ [m]},g⟩ := by
  intro u hu
  rcases List.mem_append.mp hu with hu | hu
  · constructor
    · intro kind held hv; exact ha _ _ ((h u hu).1 kind held hv)
    · intro kind held hv; exact ha _ _ ((h u hu).2 kind held hv)
  · simp only [List.mem_singleton] at hu; subst u
    rcases hkind with hkind | hkind | hkind <;> simp [hkind]

theorem commit_mail_same_pages (c : Config P) (x : State P) (s : Sys P) (g : Ghost P) (n : String)
    (hp : s.pages = x.s.pages) : (commit c x s g n).s.mb = s.mb := by
  funext p
  simp [commit,hp]

def recvW (c : Config P) (m : Mail) (a : W) : W :=
  let isAck := (m.ack >= 0 || c.mutant == "MAK") && a.sent
  if isAck && !m.held then {}
  else if isAck && a.pend then {a with sent := false,bv := if m.took || c.mutant == "MDA" then m.ack else a.bv,obs := m.obs,conf := m.conf}
  else if isAck then ⟨true,m.text,m.ver,false,false,m.obs,m.conf⟩
  else if a.on && !a.sent && (!a.pend || c.mutant == "MWA") then {a with text := m.text,bv := m.ver,obs := m.obs,conf := m.conf}
  else if a.on then {a with obs := m.obs,conf := m.conf}
  else a

theorem recvW_anchor (c : Config P) (mutant : c.mutant = "none")
    (m : Mail) (a : W) (pg : Page) (g : Ghost P) (p : Fin P)
    (snapshot : m.ver = pg.ver → m.text = pg.buf)
    (old : a.on = true → a.bv = pg.ver → Auth g p pg.buf)
    (took : m.took = true → m.ack = pg.ver → Auth g p pg.buf)
    (shown : (recvW c m a).on = true → Auth g p (recvW c m a).text)
    (hon : (recvW c m a).on = true) (matchv : (recvW c m a).bv = pg.ver) :
    Auth g p pg.buf := by
  simp only [recvW,mutant] at hon matchv shown
  simp only [show ("none" == "MAK") = false from rfl,show ("none" == "MDA") = false from rfl,
    show ("none" == "MWA") = false from rfl,Bool.or_false] at hon matchv shown
  split at hon
  · simp at hon
  · rename_i hd
    simp only [hd,Bool.false_eq_true,ite_false] at matchv shown
    split at hon
    · rename_i hp
      simp only [hp,ite_true] at matchv shown
      change a.on = true at hon
      split at matchv
      · rename_i ht
        exact took ht matchv
      · try simp only [Bool.false_eq_true,ite_false] at matchv
        exact old hon matchv
    · rename_i hp
      simp only [hp,Bool.false_eq_true,ite_false] at matchv shown
      split at hon
      · rename_i ha
        simp only [ha,ite_true] at matchv shown
        have hs := shown (by trivial)
        rw [snapshot matchv] at hs
        exact hs
      · rename_i ha
        simp only [ha,Bool.false_eq_true,ite_false] at matchv shown
        split at hon
        · rename_i hpush
          simp only [hpush,ite_true] at matchv shown
          have hs := shown hon
          rw [snapshot matchv] at hs
          exact hs
        · rename_i hpush
          simp only [hpush,Bool.false_eq_true,ite_false] at matchv shown
          split at hon
          · rename_i honold
            simp only [honold,ite_true] at matchv
            exact old honold matchv
          · rename_i honold
            simp only [honold,Bool.false_eq_true,ite_false] at matchv
            exact old hon matchv

theorem wRecv_current (c : Config P) (x : State P) (p : Fin P) :
    (wRecv c x p).s.w p = recvW c (x.s.mb p) (x.s.w p) := by
  simp [wRecv,recvW,commit,put]

theorem anchors_wRecv (c : Config P) (mutant : c.mutant = "none") (x : State P) (p : Fin P)
    (h : Anchors x) (hb : ByteDomain c x.s) (hw : WindowAuth x)
    (hen : enabled c x (.wRecv p) = true) : Anchors (wRecv c x p) := by
  have ha := authMono_execute c x (.wRecv p)
  have shown := windowAuth_execute c x (.wRecv p) hb hen hw
  simp only [execute] at ha shown
  have mon : (x.s.mb p).on = true := by
    have he : x.s.alive = true ∧ (x.s.mb p).on = true := by simpa [enabled] using hen
    exact he.2
  have hpages : (wRecv c x p).s.pages = x.s.pages := rfl
  have hmail : (wRecv c x p).s.mb = put x.s.mb p {} := by
    unfold wRecv
    exact commit_mail_same_pages c x _ _ _ rfl
  constructor
  · intro q hon held matchv
    by_cases hqp : q = p
    · subst q
      rw [hpages] at held matchv ⊢
      rw [wRecv_current] at hon matchv
      apply recvW_anchor c mutant (x.s.mb p) (x.s.w p) (x.s.pages p) (wRecv c x p).g p
        (h.snapshot p mon held) (fun on bv => ha _ _ (h.window p on held bv))
        (fun took bv => ha _ _ (h.took p mon took held bv)) _ hon matchv
      intro on
      have hs := shown p (by simpa only [wRecv_current] using on)
      simpa only [wRecv_current] using hs
    · apply ha
      apply h.window q
      · simpa [wRecv,commit,put,hqp] using hon
      · simpa [hpages] using held
      · simpa [wRecv,commit,put,hqp] using matchv
  · intro q hon held matchv
    simp only [hmail,hpages] at hon held matchv ⊢
    by_cases hqp : q = p
    · simp [put,hqp] at hon
    · simp only [put,hqp,ite_false] at hon matchv ⊢
      exact h.snapshot q hon held matchv
  · intro q hon took held matchv
    simp only [hmail,hpages] at hon took held matchv ⊢
    by_cases hqp : q = p
    · simp [put,hqp] at hon
    · simp only [put,hqp,ite_false] at hon took matchv
      exact ha _ _ (h.took q hon took held matchv)
  · intro m mem
    constructor
    · intro kind held matchv; exact ha _ _ ((h.queue m mem).1 kind (by simpa [hpages] using held) (by simpa [hpages] using matchv))
    · intro kind held matchv; exact ha _ _ ((h.queue m mem).2 kind (by simpa [hpages] using held) (by simpa [hpages] using matchv))
#print axioms anchors_wRecv
end Storage
