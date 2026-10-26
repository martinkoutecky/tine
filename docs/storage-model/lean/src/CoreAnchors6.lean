import CoreEvolution6
import CoreCarriers6
namespace Storage21
set_option maxRecDepth 8192
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false

def QueueAnchors (x : State P) : Prop := ∀ m ∈ x.s.up,
  (m.kind = "submit" ∨ m.kind = "op" →
    (x.s.pages m.p).held = true → m.bv = (x.s.pages m.p).ver → Auth x.g m.p (x.s.pages m.p).buf) ∧
  (m.kind = "op" → (x.s.pages m.q).held = true →
    m.bv2 = (x.s.pages m.q).ver → Auth x.g m.q (x.s.pages m.q).buf)
structure Anchors (x : State P) : Prop where
  window : ∀ p, (x.s.w p).on = true → (x.s.pages p).held = true →
    (x.s.w p).bv = (x.s.pages p).ver → Auth x.g p (x.s.pages p).buf
  snapshot : ∀ p, (x.s.mb p).on = true → (x.s.pages p).held = true →
    (x.s.mb p).ver = (x.s.pages p).ver → (x.s.mb p).text = (x.s.pages p).buf
  took : ∀ p, (x.s.mb p).on = true → (x.s.mb p).took = true → (x.s.pages p).held = true →
    (x.s.mb p).ack = (x.s.pages p).ver → Auth x.g p (x.s.pages p).buf
  queue : QueueAnchors x

theorem anchors_init (c : Config P) : Anchors (init c) := by
  constructor <;> simp [init,QueueAnchors]

theorem anchors_transport (x : State P) (s : Sys P) (g : Ghost P)
    (h : Anchors x) (hv : VersionBounds x.s x.g.vc) (he : PageEvolution x s)
    (hw : s.w = x.s.w) (hm : s.mb = x.s.mb) (hu : ∀ m ∈ s.up, m ∈ x.s.up)
    (ha : AuthMono x.g g) : Anchors ⟨s,g⟩ := by
  constructor
  · intro p hon held matchv
    rw [hw] at hon matchv
    rcases he p held with ⟨hh,hver,hbuf⟩ | fresh
    · rw [hbuf]; exact ha _ _ (h.window p hon hh (by simpa [hver] using matchv))
    · have hb := hv.w p; dsimp only at matchv; omega
  · intro p hon held matchv
    rw [hm] at hon matchv ⊢
    rcases he p held with ⟨hh,hver,hbuf⟩ | fresh
    · rw [hbuf]; exact h.snapshot p hon hh (by simpa [hver] using matchv)
    · have hb := (hv.mb p).1; dsimp only at matchv; omega
  · intro p hon took held matchv
    rw [hm] at hon took matchv
    rcases he p held with ⟨hh,hver,hbuf⟩ | fresh
    · rw [hbuf]; exact ha _ _ (h.took p hon took hh (by simpa [hver] using matchv))
    · have hb := (hv.mb p).2; dsimp only at matchv; omega
  · intro m mem
    have memold := hu m mem
    constructor
    · intro kind held matchv
      rcases he m.p held with ⟨hh,hver,hbuf⟩ | fresh
      · rw [hbuf]; exact ha _ _ ((h.queue m memold).1 kind hh (by simpa [hver] using matchv))
      · have hb := (hv.up m memold).1; dsimp only at matchv; omega
    · intro kind held matchv
      rcases he m.q held with ⟨hh,hver,hbuf⟩ | fresh
      · rw [hbuf]; exact ha _ _ ((h.queue m memold).2 kind hh (by simpa [hver] using matchv))
      · have hb := (hv.up m memold).2; dsimp only at matchv; omega

theorem anchors_commit (c : Config P) (x : State P) (s : Sys P) (g : Ghost P) (n : String)
    (h : Anchors ⟨s,g⟩) : Anchors (commit c x s g n) := by
  have ha := Auth_mono_commit c x s g n
  constructor
  · intro p hon held matchv
    exact ha _ _ (h.window p hon held matchv)
  · intro p hon held matchv
    change (if _ then _ else s.mb p).on = true at hon
    change (if _ then _ else s.mb p).ver = (s.pages p).ver at matchv
    change (if _ then _ else s.mb p).text = (s.pages p).buf
    split
    · rfl
    · simp_all only [Bool.false_eq_true,ite_true,ite_false]
      exact h.snapshot p hon held matchv
  · intro p hon took held matchv
    apply ha
    change (if _ then _ else s.mb p).on = true at hon
    change (if _ then _ else s.mb p).took = true at took
    change (if _ then _ else s.mb p).ack = (s.pages p).ver at matchv
    split at took
    · simp_all only [ite_true,ite_false]
      have ht : (s.mb p).on = true ∧ (s.mb p).took = true := by simpa using took
      apply h.took p ht.1 ht.2 held
      simpa [ht.1] using matchv
    · simp_all only [Bool.false_eq_true,ite_true,ite_false]
      exact h.took p hon took held matchv
  · intro m mem
    constructor
    · intro kind held matchv; exact ha _ _ ((h.queue m mem).1 kind held matchv)
    · intro kind held matchv; exact ha _ _ ((h.queue m mem).2 kind held matchv)

theorem anchors_ackMail (s : Sys P) (g : Ghost P) (p : Fin P) (ok : Bool) (v : Int) (took : Bool)
    (h : Anchors ⟨s,g⟩)
    (ht : took = true → (s.pages p).held = true → Auth g p (s.pages p).buf) :
    Anchors ⟨ackMail s p ok v took,g⟩ := by
  cases ok
  · exact h
  · constructor
    · exact h.window
    · intro q hon held matchv
      simp only [ackMail,put,Bool.not_true,Bool.false_eq_true,ite_false] at hon matchv ⊢
      by_cases he : q = p
      · subst q; simp
      · simp only [he,ite_false] at hon matchv ⊢
        exact h.snapshot q hon held matchv
    · intro q hon tookq held matchv
      simp only [ackMail,put,Bool.not_true,Bool.false_eq_true,ite_false] at hon tookq matchv ⊢
      by_cases he : q = p
      · subst q; exact ht (by simpa using tookq) held
      · simp only [he,ite_false] at hon tookq matchv ⊢
        exact h.took q hon tookq held matchv
    · exact h.queue

end Storage21
