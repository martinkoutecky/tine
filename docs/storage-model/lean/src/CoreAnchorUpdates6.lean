import CoreAnchorWindow6
namespace Storage21
set_option maxRecDepth 8192
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false

def WindowAnchor (x : State P) (g : Ghost P) (w : Fin P → W) : Prop :=
  ∀ p, (w p).on = true → (x.s.pages p).held = true →
    (w p).bv = (x.s.pages p).ver → Auth g p (x.s.pages p).buf

attribute [reducible] WindowAnchor

theorem window_put_anchor (x : State P) (g : Ghost P) (w : Fin P → W) (p : Fin P) (nw : W)
    (h : WindowAnchor x g w)
    (keep : nw.on = true → (w p).on = true ∧ nw.bv = (w p).bv) :
    WindowAnchor x g (put w p nw) := by
  intro q hon held matchv
  by_cases he : q = p
  · subst q
    simp only [put,ite_true] at hon matchv
    have hk := keep hon
    exact h p hk.1 held (by simpa [hk.2] using matchv)
  · simp only [put,he,ite_false] at hon matchv
    exact h q hon held matchv

theorem window_anchor_mono (x : State P) (g : Ghost P) (h : Anchors x) (ha : AuthMono x.g g) :
    WindowAnchor x g x.s.w := fun p on held hv => ha _ _ (h.window p on held hv)

theorem queue_append_anchor (x : State P) (g : Ghost P) (m : Up P) (h : QueueAnchors x)
    (ha : AuthMono x.g g)
    (src : m.kind = "submit" ∨ m.kind = "op" → (x.s.pages m.p).held = true →
      m.bv = (x.s.pages m.p).ver → Auth g m.p (x.s.pages m.p).buf)
    (dst : m.kind = "op" → (x.s.pages m.q).held = true →
      m.bv2 = (x.s.pages m.q).ver → Auth g m.q (x.s.pages m.q).buf) :
    QueueAnchors ⟨{x.s with up := x.s.up ++ [m]},g⟩ := by
  intro u hu
  rcases List.mem_append.mp hu with hu | hu
  · constructor
    · intro kind held hv; exact ha _ _ ((h u hu).1 kind held hv)
    · intro kind held hv; exact ha _ _ ((h u hu).2 kind held hv)
  · simp only [List.mem_singleton] at hu; subst u; exact ⟨src,dst⟩

theorem anchors_wOpen (c : Config P) (x : State P) (p : Fin P) (h : Anchors x) : Anchors (wOpen c x p) := by
  apply anchors_commit
  apply anchors_window_pre x _ _ _ h
  · exact authMono_refl x.g
  · apply window_put_anchor x x.g x.s.w p _ h.window
    simp
  · exact queue_append_control x x.g _ h.queue (authMono_refl x.g) (Or.inl rfl)
theorem anchors_wClose (c : Config P) (x : State P) (p : Fin P) (h : Anchors x) : Anchors (wClose c x p) := by
  apply anchors_commit
  apply anchors_window_pre x _ _ _ h
  · exact authMono_refl x.g
  · apply window_put_anchor x x.g x.s.w p _ h.window
    simp
  · exact queue_append_control x x.g _ h.queue (authMono_refl x.g) (Or.inr (Or.inl rfl))
theorem anchors_wEdit (c : Config P) (x : State P) (p : Fin P) (v : Int) (h : Anchors x) : Anchors (wEdit c x p v) := by
  apply anchors_commit
  apply anchors_window_pre x _ _ _ h
  · exact authMono_refl x.g
  · apply window_put_anchor x x.g x.s.w p _ h.window
    intro hon; exact ⟨hon,rfl⟩
  · exact h.queue
theorem anchors_wDiscard (c : Config P) (x : State P) (p : Fin P) (h : Anchors x) : Anchors (wDiscard c x p) := by
  apply anchors_commit
  apply anchors_window_pre x _ _ _ h
  · exact authMono_refl x.g
  · apply window_put_anchor x x.g x.s.w p _ h.window
    intro hon; exact ⟨hon,rfl⟩
  · exact queue_append_control x x.g _ h.queue (authMono_refl x.g) (Or.inr (Or.inr rfl))
theorem anchors_wSend (c : Config P) (x : State P) (p : Fin P) (h : Anchors x)
    (on : (x.s.w p).on = true) : Anchors (wSend c x p) := by
  apply anchors_commit
  apply anchors_window_pre x _ _ _ h
  · exact authMono_refl x.g
  · apply window_put_anchor x x.g x.s.w p _ h.window
    intro hon; exact ⟨hon,rfl⟩
  · apply queue_append_anchor x x.g _ h.queue (authMono_refl x.g)
    · intro _ held hv; exact h.window p on held hv
    · simp [up1]
theorem anchors_wResolve (c : Config P) (x : State P) (p : Fin P) (v : Int) (h : Anchors x)
    (on : (x.s.w p).on = true) : Anchors (wResolve c x p v) := by
  have ha := authMono_seen x.g p ([(x.s.w p).obs].filter fun t => t == absent || c.texts.contains t)
  apply anchors_commit
  apply anchors_window_pre x _ _ _ h
  · exact ha
  · apply window_put_anchor x _ x.s.w p _ (window_anchor_mono x _ h ha)
    intro hon; exact ⟨hon,rfl⟩
  · apply queue_append_anchor x _ _ h.queue ha
    · intro _ held hv; exact ha _ _ (h.window p on held hv)
    · simp [up1]
theorem anchors_wOp (c : Config P) (x : State P) (p q : Fin P) (t u : Int) (h : Anchors x)
    (onp : (x.s.w p).on = true) (onq : (x.s.w q).on = true) : Anchors (wOp c x p q t u) := by
  apply anchors_commit
  apply anchors_window_pre x _ _ _ h
  · exact authMono_refl x.g
  · apply window_put_anchor x x.g _ q _
    · apply window_put_anchor x x.g x.s.w p _ h.window
      intro hon; exact ⟨hon,rfl⟩
    · intro hon
      by_cases hpq : q = p
      all_goals constructor
      all_goals first | (simpa [put,hpq] using hon) | simp [put,hpq]
  · apply queue_append_anchor x x.g _ h.queue (authMono_refl x.g)
    · intro _ held hv; exact h.window p onp held hv
    · intro _ held hv; exact h.window q onq held hv
end Storage21
