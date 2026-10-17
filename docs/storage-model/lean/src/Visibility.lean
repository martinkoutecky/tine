import Shape
namespace Storage
set_option maxRecDepth 8192
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false

abbrev VisiblePage (pg : Page) (seen : List Int) : Prop := pg.held = true →
  (pg.typed = true ∨ pg.risk = true) → pg.buf ∈ seen
def Visibility (x : State P) : Prop :=
  (∀ p, VisiblePage (x.s.pages p) (x.g.seen p)) ∧
  (∀ p, (x.s.drafts p).bytes ≠ noneBytes → (x.s.drafts p).bytes ∈ x.g.seen p)

theorem visibility_commit (c : Config P) (x : State P) (s : Sys P) (g : Ghost P) (n : String)
    (h : Visibility ⟨s,g⟩) : Visibility (commit c x s g n) :=
  ⟨fun p hh hf => seen_commit c x s g n p _ (h.1 p hh hf),
   fun p hn => seen_commit c x s g n p _ (h.2 p hn)⟩
theorem visibility_mono (s : Sys P) (g k : Ghost P) (h : Visibility ⟨s,g⟩)
    (hm : SeenMono g k) : Visibility ⟨s,k⟩ :=
  ⟨fun p hh hf => hm p _ (h.1 p hh hf),fun p hn => hm p _ (h.2 p hn)⟩
theorem visibility_ackMail (s : Sys P) (g : Ghost P) (p : Fin P) (ok : Bool) (v : Int) (t : Bool)
    (h : Visibility ⟨s,g⟩) : Visibility ⟨ackMail s p ok v t,g⟩ := by cases ok <;> simpa [Visibility,ackMail] using h

theorem visible_put (s : Sys P) (g : Ghost P) (p : Fin P) (pg : Page) (h : Visibility ⟨s,g⟩)
    (hp : VisiblePage pg (g.seen p)) : Visibility ⟨{s with pages := put s.pages p pg},g⟩ := by
  refine ⟨?_,h.2⟩
  intro q hh hf; by_cases he : q = p
  · subst q; simpa [put] using hp (by simpa [put] using hh) (by simpa [put] using hf)
  · simpa [put,he] using h.1 q (by simpa [put,he] using hh) (by simpa [put,he] using hf)

theorem draftEntry_seen (pg : Page) (seen : List Int) (h : VisiblePage pg seen) :
    (draftEntry pg).bytes ≠ noneBytes → (draftEntry pg).bytes ∈ seen := by
  simp only [draftEntry]; split
  · intro _
    have go : pg.held = true ∧ pg.risk = true := by simpa using (by assumption : (pg.held && pg.risk) = true)
    exact h go.1 (Or.inr go.2)
  · simp

theorem visible_draft_put (s : Sys P) (g : Ghost P) (p : Fin P) (dr : Draft)
    (h : Visibility ⟨s,g⟩) (hd : dr.bytes ≠ noneBytes → dr.bytes ∈ g.seen p) :
    Visibility ⟨{s with drafts := put s.drafts p dr},g⟩ := by
  refine ⟨h.1,?_⟩
  intro q hn; by_cases he : q = p
  · subst q; simpa [put] using hd (by simpa [put] using hn)
  · simpa [put,he] using h.2 q (by simpa [put,he] using hn)

theorem visible_table (c : Config P) (mutant : c.mutant = "none") (pg : Page) (seen : List Int)
    (d v : Int) (h : VisiblePage pg seen) (hs : PageShape pg) :
    VisiblePage (table c pg d v) seen := by
  simp only [table,mutant]
  simp only [show ("none" == "MQ") = false from rfl,show ("none" == "MLO") = false from rfl,
    Bool.false_and,Bool.or_false,Bool.false_eq_true,ite_false]
  split <;> (try split) <;> (try split)
  all_goals intro hh hf
  · exact h hh hf
  · have hr : pg.risk = false := by simp [clean] at *; grind
    simp_all
  · exact h hh hf
  · have hd : dirty pg = true := by
      have hc : clean pg ≠ true := by simpa only [clean] using (by assumption : ¬ clean {pg with obs := d} = true)
      cases he : clean pg <;> simp_all [dirty]
    exact h hh (dirty_flags pg hs hd)

theorem visible_submit (c : Config P) (mutant : c.mutant = "none") (pg : Page)
    (t bv v : Int) (seen : List Int) (h : t ∈ seen) : VisiblePage (submitTo c pg t bv v) seen := by
  intro _ _; simpa [submitTo_takes c mutant] using h

theorem visibility_upOpen (c : Config P) (x : State P) (s : Sys P) (m : Up P) (ok : Bool)
    (h : Visibility ⟨s,x.g⟩) : Visibility (upOpen c x s m ok) := by
  simp only [upOpen]; split <;> (try split)
  all_goals apply visibility_commit
  all_goals apply visibility_ackMail
  all_goals apply visible_put
  all_goals first | exact h | exact h.1 m.p | simp [VisiblePage]

theorem visibility_upClose (c : Config P) (x : State P) (s : Sys P) (m : Up P)
    (h : Visibility ⟨s,x.g⟩) : Visibility (upClose c x s m) := by
  apply visibility_commit; split
  · exact visible_put s x.g m.p {} h (by simp [VisiblePage])
  · exact h

theorem visibility_upDiscard (c : Config P) (mutant : c.mutant = "none")
    (x : State P) (s : Sys P) (m : Up P) (ok : Bool)
    (h : Visibility ⟨s,x.g⟩) (hdr : s.drafts = x.s.drafts)
    (hd : x.s.disk m.p ≠ noneBytes) : Visibility (upDiscard c x s m ok) := by
  simp only [upDiscard]; split
  · exact visibility_commit c x _ _ _ h
  · split
    · exact visibility_commit c x _ _ _ (visibility_ackMail _ _ _ _ _ _ h)
    · apply visibility_commit; apply visibility_ackMail
      apply visible_put
      · exact h
      · intro hh hf
        have hold : ((s.pages m.p).risk && (x.s.disk m.p == (s.pages m.p).buf ||
          x.s.disk m.p == (x.s.drafts m.p).bytes) && c.mutant != "MDE") = true := by simpa using hf
        simp only [Bool.and_eq_true,Bool.or_eq_true,beq_iff_eq] at hold
        rcases hold.1.2 with same | same
        · rw [same]; exact h.1 m.p hh (Or.inr hold.1.1)
        · rw [same]; rw [←hdr]
          exact h.2 m.p (by rw [hdr,←same]; exact hd)

theorem visibility_upSubmit (c : Config P) (mutant : c.mutant = "none")
    (x : State P) (s : Sys P) (m : Up P) (h : Visibility ⟨s,x.g⟩)
    (ht : m.t ∈ x.g.seen m.p) : Visibility (upSubmit c x s m) := by
  simp only [upSubmit,mutant]
  simp only [show ("none" == "MDO") = false from rfl,Bool.false_eq_true,ite_false]
  split
  · exact visibility_commit c x _ _ _ h
  · apply visibility_commit; apply visibility_ackMail
    apply visible_put
    · exact h
    · split
      · intro _ _; exact ht
      · exact visible_submit c mutant _ _ _ _ _ ht

theorem visibility_upOp (c : Config P) (mutant : c.mutant = "none")
    (x : State P) (s : Sys P) (m : Up P) (ok : Bool) (h : Visibility ⟨s,x.g⟩)
    (ht : m.t ∈ x.g.seen m.p) (ht2 : m.t2 ∈ x.g.seen m.q) : Visibility (upOp c x s m ok) := by
  simp only [upOp]; split <;> (try split)
  · exact visibility_commit c x _ _ _ h
  · exact visibility_commit c x _ _ _ (visibility_ackMail _ _ _ _ _ _ (visibility_ackMail _ _ _ _ _ _ h))
  · apply visibility_commit; apply visibility_ackMail; apply visibility_ackMail
    apply visible_put {s with drafts := if c.mutant == "MOD" then s.drafts else put s.drafts m.q (draftEntry {submitTo c (s.pages m.q) m.t2 m.bv2 (x.g.vc+1) with risk := true}), pages := put s.pages m.q {submitTo c (s.pages m.q) m.t2 m.bv2 (x.g.vc+1) with risk := true}}
    · apply visible_put {s with drafts := if c.mutant == "MOD" then s.drafts else put s.drafts m.q (draftEntry {submitTo c (s.pages m.q) m.t2 m.bv2 (x.g.vc+1) with risk := true})}
      · split
        · exact h
        · apply visible_draft_put _ _ _ _ h
          apply draftEntry_seen
          intro _ _; simpa [submitTo_takes c mutant] using ht2
      · intro _ _; simpa [submitTo_takes c mutant] using ht2
    · exact visible_submit c mutant _ _ _ _ _ ht

-- All newly risky save failures retain the dirty-at-flush buffer.
theorem visible_reply (c : Config P) (mutant : c.mutant = "none") (pg : Page) (seen : List Int)
    (out : String) (b : Int) (hb : pg.buf ∈ seen) : VisiblePage (onReply c pg out b) seen := by
  intro hh hf; simp only [onReply]; split <;> exact hb

theorem visibility_execute (c : Config P) (mutant : c.mutant = "none")
    (x : State P) (a : Action P) (h : Visibility x) (hs : Shapes x.s)
    (hj : JobSnapshot x.s) (hb : ByteDomain c x.s) (hc : CarrierSeen x)
    (hn : ∀ p, x.s.disk p ≠ noneBytes)
    (hen : enabled c x a = true) : Visibility (execute c x a) := by
  have mono := seenMono_execute c x a
  cases a <;> simp only [execute] at mono ⊢
  case deliverUp ok =>
    cases hq : x.s.up with
    | nil => exact h
    | cons m tail =>
      have ht : Visibility ⟨{x.s with up := tail},x.g⟩ := h
      simp only [deliverUp]; split <;> (try split) <;> (try split) <;> (try split)
      · exact visibility_upOpen c x _ m ok ht
      · exact visibility_upClose c x _ m ht
      · exact visibility_upDiscard c mutant x _ m ok ht rfl (hn m.p)
      · exact visibility_upOp c mutant x _ m ok ht (hc m (by simp [hq]) (m.p,m.t) (by
          have he : m.kind = "op" := by simpa using (by assumption : (m.kind == "op") = true)
          simp [carries,he]))
          (hc m (by simp [hq]) (m.q,m.t2) (by
          have he : m.kind = "op" := by simpa using (by assumption : (m.kind == "op") = true)
          simp [carries,he]))
      · exact visibility_upSubmit c mutant x _ m ht (hc m (by simp [hq]) (m.p,m.t) (by
          have kind := (hb.up m (by simp [hq])).1
          simp_all [carries]))
  case observe p =>
    exact visibility_commit c x _ _ _ (visible_put _ _ _ _ h (visible_table c mutant _ _ _ _ (h.1 p) (hs p)))
  case check =>
    simp only [check]; split
    · exact visibility_commit c x _ _ _ (visible_put _ _ _ _ h (visible_table c mutant _ _ _ _ (h.1 _) (hs _)))
    · exact visibility_commit c x _ _ _ h
  case dirSync ok =>
    have hon : x.s.job.on = true := by simp [enabled] at hen; grind
    have snap := hj hon
    have seen := h.1 _ snap.1 (dirty_flags _ (hs _) snap.2.2.2.2)
    apply visibility_commit
    apply visible_put
    · exact h
    · simp only [dirSync,mutant]
      simp only [show ("none" == "MRE") = false from rfl,Bool.and_false,Bool.false_eq_true,ite_false]
      exact visible_reply c mutant _ _ _ _ seen
  case saveFail =>
    have hon : x.s.job.on = true := by simp [enabled] at hen; grind
    have snap := hj hon
    have seen := h.1 _ snap.1 (dirty_flags _ (hs _) snap.2.2.2.2)
    exact visibility_commit c x _ _ _ (visible_put _ _ _ _ h (visible_reply c mutant _ _ _ _ seen))
  case switchReq =>
    apply visibility_commit; refine ⟨?_,h.2⟩
    intro p hh hf; dsimp only at hh hf ⊢; split at * <;> simp_all only [ite_true,ite_false]
    · have go : ((x.s.pages p).held && dirty (x.s.pages p)) = true := by simpa only [hh] using (by assumption : (true && dirty (x.s.pages p)) = true)
      have hd : dirty (x.s.pages p) = true := by simp only [Bool.and_eq_true] at go; exact go.2
      exact h.1 p hh (dirty_flags _ (hs p) hd)
    · exact h.1 p hh hf
  case draftSync p =>
    simp only [draftSync,mutant]
    simp only [show ("none" == "MBD") = false from rfl,Bool.false_and,Bool.false_eq_true,ite_false]
    apply visibility_commit
    split
    all_goals apply visible_draft_put
    all_goals first | exact h | exact draftEntry_seen _ _ (h.1 p)
  case launch =>
    apply visibility_commit; refine ⟨?_,h.2⟩
    intro p hh hf; simp only [restored] at hh hf ⊢; split at * <;> simp_all only [ite_true,ite_false]
    · contradiction
    · exact h.2 p (by simpa using (by assumption : ¬ ((x.s.drafts p).bytes == noneBytes) = true))
  case switchFin => exact visibility_commit c x _ _ _ ⟨by simp [down,VisiblePage],h.2⟩
  case crash => exact visibility_commit c x _ _ _ ⟨by simp [down,VisiblePage],h.2⟩
  case power keep => exact visibility_commit c x _ _ _ ⟨by simp [down,VisiblePage],h.2⟩
  case wResolve p v =>
    apply visibility_commit
    apply visibility_mono {x.s with w := put x.s.w p {x.s.w p with text := v,pend := false,sent := true},up := x.s.up ++ [up1 c "submit" p v (x.s.w p).bv (x.s.w p).obs]} x.g _ h
    intro q b hh; by_cases he : q = p
    · subst q; simpa [Seen,put,mem_union_general] using Or.inl hh
    · simpa [Seen,put,he] using hh
  all_goals apply visibility_commit; exact h

theorem real_positive_not_none (c : Config P) (T : Nat) (texts : c.texts = positiveTexts T)
    (b : Int) (h : RealByte c b) : b ≠ noneBytes := by
  rcases h with rfl | h
  · decide
  · rw [texts] at h
    obtain ⟨n,_,rfl⟩ := List.mem_map.mp h
    simp only [noneBytes]; omega

theorem visibility_reachable (c : Config P) (T : Nat) (texts : c.texts = positiveTexts T)
    (mutant : c.mutant = "none") (x : State P) (hx : Reachable c x) : Visibility x := by
  induction hx with
  | initial => simp [Visibility,VisiblePage,init]
  | @advance x y hr a hs ih =>
    have shape := shapes_reachable c mutant x hr
    have job := snapshot_reachable c mutant x hr
    have bytes := bytes_reachable c x hr
    have carrier := carrierSeen_reachable c x hr
    unfold step at hs; split at hs
    · cases hs
      exact visibility_execute c mutant x a ih shape job bytes carrier
        (fun p => real_positive_not_none c T texts _ (bytes.disk p)) (by assumption)
    · contradiction

theorem typedSeen_reachable (c : Config P) (T : Nat) (texts : c.texts = positiveTexts T)
    (mutant : c.mutant = "none") (x : State P) (hx : Reachable c x) : TypedSeen x :=
  fun p hh ht => (visibility_reachable c T texts mutant x hx).1 p hh (Or.inl ht)
#print axioms visibility_reachable
#print axioms typedSeen_reachable
end Storage
