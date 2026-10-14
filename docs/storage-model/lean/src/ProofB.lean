import FinalG
namespace Storage
set_option maxRecDepth 8192
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false

def BInv (x : State P) : Prop := "B-unsaved-replaced" ∉ x.g.bad
def PostPromises (x : State P) (s : Sys P) (g : Ghost P) : Fin P → Promise := fun p =>
  let a := x.s.pages p
  let b := s.pages p
  if !(g.mine.contains p) && a.held && dirty a && b.held && clean b && b.buf == a.buf then
    ack (g.promise p) b.buf b.ver false 0 else g.promise p
def CleanSafe (x : State P) : Prop := ∀ p, clean (x.s.pages p) = true →
  (x.s.pages p).buf = (x.s.pages p).obs ∨
    ((x.g.promise p).ver ≥ (x.s.pages p).ver ∧ (x.g.promise p).bytes = (x.s.pages p).buf)
def TypedSeen (x : State P) : Prop := ∀ p, (x.s.pages p).held = true →
  (x.s.pages p).typed = true → (x.s.pages p).buf ∈ x.g.seen p
def NoReplacement (x : State P) (s : Sys P) : Prop := ∀ p, (x.s.pages p).held = true →
  (s.pages p).held = true ∧ (s.pages p).buf = (x.s.pages p).buf

theorem B_commit (c : Config P) (x : State P) (s : Sys P) (g : Ghost P) (n : String)
    (hb : "B-unsaved-replaced" ∉ g.bad)
    (hs : ∀ p, keptB x p s (PostPromises x s g) g.mine n = true) :
    BInv (commit c x s g n) := by
  have hall : (ids P).all (fun p => keptB x p s (PostPromises x s g) g.mine n) = true :=
    List.all_eq_true.mpr (fun p _ => hs p)
  change "B-unsaved-replaced" ∉ union (union g.bad (tags (!(ids P).all (fun p => keptB x p s (PostPromises x s g) g.mine n)) "B-unsaved-replaced")) (tags _ "B'-window-replaced")
  rw [hall]
  simpa using hb

theorem B_commit_noReplacement (c : Config P) (x : State P) (s : Sys P) (g : Ghost P) (n : String)
    (hb : "B-unsaved-replaced" ∉ g.bad) (hn : NoReplacement x s) : BInv (commit c x s g n) := by
  apply B_commit c x s g n hb
  intro p
  by_cases held : (x.s.pages p).held = true
  · have hn := hn p held
    simp [keptB,held,hn.1,hn.2]
  · simp [keptB,held]

theorem noReplacement_refl (x : State P) : NoReplacement x x.s := fun _ h => ⟨h,rfl⟩

theorem B_commit_exempt (c : Config P) (x : State P) (s : Sys P) (g : Ghost P) (n : String)
    (hb : "B-unsaved-replaced" ∉ g.bad) (hn : n = "crash" ∨ n = "power") : BInv (commit c x s g n) := by
  apply B_commit c x s g n hb
  intro p; rcases hn with rfl | rfl <;> simp [keptB]

theorem B_window_execute (c : Config P) (x : State P) (a : Action P) (hx : BInv x)
    (ha : (execute c x a).s.pages = x.s.pages)
    (bad : (execute c x a).g.bad = (commit c x x.s x.g "frame").g.bad) : BInv (execute c x a) := by
  rw [BInv,bad]
  exact B_commit_noReplacement c x x.s x.g "frame" hx (noReplacement_refl x)

theorem B_metadata_execute (c : Config P) (x : State P) (a : Action P) (hx : BInv x)
    (excluded : (∀ p, a ≠ .observe p) ∧ (∀ ok, a ≠ .deliverUp ok) ∧
      a ≠ .check ∧ a ≠ .switchFin ∧ a ≠ .launch) : BInv (execute c x a) := by
  cases a <;> simp only [execute]
  case deliverUp ok => exact False.elim (excluded.2.1 ok rfl)
  case observe p => exact False.elim (excluded.1 p rfl)
  case check => exact False.elim (excluded.2.2.1 rfl)
  case switchFin => exact False.elim (excluded.2.2.2.1 rfl)
  case launch => exact False.elim (excluded.2.2.2.2 rfl)
  case crash => exact B_commit_exempt c x _ _ _ hx (Or.inl rfl)
  case power keep => exact B_commit_exempt c x _ _ _ hx (Or.inr rfl)
  case rename =>
    apply B_commit_noReplacement
    · simpa [BInv] using hx
    · exact noReplacement_refl x
  case dirSync ok =>
    apply B_commit_noReplacement
    · exact hx
    · intro p hp
      simp only [put]
      by_cases he : p = x.s.job.p
      · subst p; simp only [ite_true]
        split
        · exact ⟨hp,by trivial⟩
        · simp only [onReply]; split <;> (try split)
          all_goals exact ⟨hp,by trivial⟩
      · simp only [he,ite_false]; exact ⟨hp,by trivial⟩
  case saveFail =>
    apply B_commit_noReplacement
    · exact hx
    · intro p hp
      simp only [put]
      by_cases he : p = x.s.job.p
      · subst p; simp only [ite_true,onReply]; split <;> exact ⟨hp,by trivial⟩
      · simp only [he,ite_false]; exact ⟨hp,by trivial⟩
  case switchReq =>
    apply B_commit_noReplacement
    · exact hx
    · intro p hp; dsimp only; split <;> exact ⟨hp,by trivial⟩
  case draftSync p =>
    simp only [draftSync]; split <;> (try split) <;> (try split)
    all_goals apply B_commit_noReplacement
    all_goals first | exact hx | exact noReplacement_refl x
  all_goals apply B_commit_noReplacement
  all_goals first | exact hx | exact noReplacement_refl x

theorem B_init (c : Config P) : BInv (init c) := by simp [BInv,init]

/-- Explicit residual actions; this is a partial result, not reachable B. -/
theorem B_reachable_of_residual (c : Config P)
    (residual : ∀ x, Reachable c x → BInv x → ∀ a, enabled c x a = true →
      ((∃ p, a = .observe p) ∨ (∃ ok, a = .deliverUp ok) ∨ a = .check ∨
        a = .switchFin ∨ a = .launch) → BInv (execute c x a))
    (x : State P) (hx : Reachable c x) : "B-unsaved-replaced" ∉ x.g.bad := by
  induction hx with
  | initial => exact B_init c
  | @advance x y hr a hs ih =>
    unfold step at hs; split at hs
    · cases hs
      by_cases he : (∃ p, a = .observe p) ∨ (∃ ok, a = .deliverUp ok) ∨ a = .check ∨ a = .switchFin ∨ a = .launch
      · exact residual x hr ih a (by assumption) he
      · apply B_metadata_execute c x a ih
        constructor
        · intro p ha; exact he (Or.inl ⟨p,ha⟩)
        · constructor
          · intro ok ha; exact he (Or.inr (Or.inl ⟨ok,ha⟩))
          · constructor
            · intro ha; exact he (Or.inr (Or.inr (Or.inl ha)))
            · constructor
              · intro ha; exact he (Or.inr (Or.inr (Or.inr (Or.inl ha))))
              · intro ha; exact he (Or.inr (Or.inr (Or.inr (Or.inr ha))))
    · contradiction
#print axioms B_metadata_execute
#print axioms B_reachable_of_residual
end Storage
