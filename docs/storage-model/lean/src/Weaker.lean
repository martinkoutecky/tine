import ProofG
namespace Storage
def RequestAuth (x : State P) (m : Up P) : Prop :=
  (m.kind ≠ "open" ∧ m.kind ≠ "close" ∧ m.kind ≠ "discard" ∧ m.kind ≠ "op" →
    (m.bv = (x.s.pages m.p).ver → Auth x.g m.p (x.s.pages m.p).buf) ∧
    (m.ro ≠ noneBytes → Auth x.g m.p m.ro)) ∧
  (m.kind = "op" →
    (m.bv = (x.s.pages m.p).ver → Auth x.g m.p (x.s.pages m.p).buf) ∧
    (m.bv2 = (x.s.pages m.q).ver → Auth x.g m.q (x.s.pages m.q).buf))
def HeadDeliveryAuth (c : Config P) (x : State P) : Prop :=
  ∀ m tail, x.s.up = m::tail → ∀ ok,
    enabled c x (.deliverUp ok) = true → RequestAuth x m

theorem G_deliverUp_of_requestAuth (c : Config P) (mutant : c.mutant = "none")
    (x : State P) (m : Up P) (tail : List (Up P)) (ok : Bool)
    (hx : GInv x) (hm : RequestAuth x m) : GInv (deliverUp c x m tail ok) := by
  have hrest : GInv ⟨{x.s with up := tail},x.g⟩ := hx
  simp only [deliverUp]
  split
  · exact G_upOpen c x _ m ok hrest
  · split
    · exact G_upClose c x _ m hrest
    · split
      · exact G_upDiscard c x _ m ok hrest
      · split
        · have ha := hm.2 (by simpa using (by assumption : (m.kind == "op") = true))
          exact G_upOp c mutant x _ m ok hrest ha.1 ha.2
        · have hkind : m.kind ≠ "open" ∧ m.kind ≠ "close" ∧ m.kind ≠ "discard" ∧ m.kind ≠ "op" := by
            simp only [beq_iff_eq] at *
            exact ⟨by assumption,by assumption,by assumption,by assumption⟩
          have ha := hm.1 hkind
          exact G_upSubmit c mutant x _ m hrest ha.1 ha.2

theorem G_execute_of_headAuth (c : Config P) (mutant : c.mutant = "none")
    (x : State P) (a : Action P) (hx : GInv x) (hen : enabled c x a = true)
    (diskReal : x.s.disk x.s.job.p ≠ unknown) (auth : HeadDeliveryAuth c x) :
    GInv (execute c x a) := by
  cases a with
  | deliverUp ok =>
    simp only [execute]
    cases hq : x.s.up with
    | nil => exact hx
    | cons m tail => exact G_deliverUp_of_requestAuth c mutant x m tail ok hx (auth m tail hq ok hen)
  | _ => apply G_execute_non_delivery c mutant x _ hx hen (by intro r; simp) diskReal

theorem G_positiveTexts_of_headAuth (c : Config P) (T : Nat)
    (texts : c.texts = positiveTexts T) (mutant : c.mutant = "none")
    (protocol : ∀ x, Reachable c x → HeadDeliveryAuth c x)
    (x : State P) (hx : Reachable c x) : "G-overwrote-unseen" ∉ x.g.bad := by
  have inv : GInv x := by
    induction hx with
    | initial => exact G_init c
    | @advance x y hr a hs ih =>
      have hd := positiveTexts_disk_not_unknown c T texts x hr x.s.job.p
      unfold step at hs
      split at hs
      · cases hs
        exact G_execute_of_headAuth c mutant x a ih (by assumption) hd (protocol x hr)
      · contradiction
  exact inv.1
#print axioms G_positiveTexts_of_headAuth
end Storage
