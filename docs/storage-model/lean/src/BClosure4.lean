import Lineage4
import CoreBSwitch21
import CoreBDelivery21
namespace Storage3
open Storage21
set_option maxRecDepth 8192
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false

theorem B_observe4 (c : Config P) (disk : Fin P → Int) (x : State P) (p : Fin P)
    (mutant : c.mutant = "none") (hr : Reachable c disk x) (h : BInv (project x)) :
    BInv (project (execute c x (.old (.observe p)))) :=
  B_observe c mutant (project x) p h (cleanSafe_reachable4 c disk x mutant hr)

theorem B_check4 (c : Config P) (disk : Fin P → Int) (x : State P)
    (mutant : c.mutant = "none") (hr : Reachable c disk x) (h : BInv (project x)) :
    BInv (project (execute c x (.old .check))) :=
  B_check c mutant (project x) h (cleanSafe_reachable4 c disk x mutant hr)

theorem B_switchFin4 (c : Config P) (disk : Fin P → Int) (x : State P)
    (mutant : c.mutant = "none") (hr : Reachable c disk x) (h : BInv (project x))
    (hen : enabled c x (.old .switchFin) = true) :
    BInv (project (execute c x (.old .switchFin))) :=
  B_switchFin c mutant (project x) h (cleanSafe_reachable4 c disk x mutant hr) hen

theorem B_reachable4_of_delivery (c : Config P) (disk : Fin P → Int)
    (mutant : c.mutant = "none")
    (delivery : ∀ x, Reachable c disk x → BInv (project x) → ∀ ok,
      enabled c x (.old (.deliverUp ok)) = true → BInv (project (execute c x (.old (.deliverUp ok)))))
    (x : State P) (hr : Reachable c disk x) : "B-unsaved-replaced" ∉ x.g.bad := by
  apply B_reachable3_of_residual4 c disk mutant ?_ x hr
  intro y hy hb a hen which
  rcases which with ⟨p,rfl⟩ | ⟨ok,rfl⟩ | rfl | rfl
  · exact B_observe4 c disk y p mutant hy hb
  · exact delivery y hy hb ok hen
  · exact B_check4 c disk y mutant hy hb
  · exact B_switchFin4 c disk y mutant hy hb hen

def StaleHeadObligation4 (c : Config P) (disk : Fin P → Int) : Prop :=
  ∀ x, Reachable c disk x → ∀ ok m tail, x.s.up = m::tail →
    enabled c x (.old (.deliverUp ok)) = true → m.kind = "submit" →
      StaleHeadProtection4 (project x) m

theorem B_reachable4_of_staleHead (c : Config P) (disk : Fin P → Int)
    (mutant : c.mutant = "none") (head : StaleHeadObligation4 c disk)
    (x : State P) (hr : Reachable c disk x) : "B-unsaved-replaced" ∉ x.g.bad := by
  apply B_reachable4_of_delivery c disk mutant ?_ x hr
  intro y hy hb ok hen
  simp only [execute,executeOld,mutant]
  simp only [show ("none" == "MDO") = false from rfl,Bool.false_eq_true,ite_false]
  change BInv (Storage21.execute c (project y) (.deliverUp ok))
  simp only [Storage21.execute]
  cases hq : (project y).s.up with
  | nil => exact hb
  | cons m tail =>
    change y.s.up = m::tail at hq
    have kind := (protocol_reachable3 c disk y mutant hy).kinds m (by simp [hq])
    exact B_delivery_protected4 c mutant (project y) m tail ok hb
      (cleanSafe_reachable4 c disk y mutant hy) kind (head y hy ok m tail hq hen)

theorem guarantee_remaining4_of_staleHead (c : Config P) (disk : Fin P → Int)
    (mutant : c.mutant = "none") (head : StaleHeadObligation4 c disk)
    (x : State P) (hr : Reachable c disk x) :
    guarantee x = true ↔ noLoss x = true ∧ "G-overwrote-unseen" ∉ x.g.bad := by
  have b := B_reachable4_of_staleHead c disk mutant head x hr
  simpa [b] using guarantee_remaining3 c disk x mutant hr
end Storage3
