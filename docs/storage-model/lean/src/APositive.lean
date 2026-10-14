import PromiseBytes
namespace Storage
set_option maxRecDepth 8192
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false

theorem custody_launch (c : Config P) (T : Nat) (texts : c.texts = positiveTexts T)
    (mutant : c.mutant = "none") (x : State P) (hx : Reachable c x)
    (h : Custody c x) (hen : enabled c x .launch = true) : Custody c (launch c x) := by
  have dead : x.s.alive = false := by simpa [enabled] using hen
  have inactive := inactive_reachable c x hx
  have pb := promiseBytes_reachable c mutant x hx
  apply custody_commit
  · intro p need
    have oldneed : NeedsDraft c x p := need
    have eq := (h p oldneed).1
    have real := pb p oldneed.1.1
    have nonempty : (x.s.drafts p).bytes ≠ noneBytes := by
      rw [eq]; exact real_positive_not_none c T texts _ real
    refine ⟨eq,?_⟩
    intro _; simp [restored,nonempty]
  · exact postPromises_unheld_old x _ _ (inactive dead)

structure CustodyResidual4 (c : Config P) : Prop where
  delivery : ∀ x ok, Reachable c x → Custody c x → enabled c x (.deliverUp ok) = true →
    Custody c (execute c x (.deliverUp ok))
  rename : ∀ x, Reachable c x → Custody c x → enabled c x .rename = true →
    Custody c (execute c x .rename)
  dirSync : ∀ x ok, Reachable c x → Custody c x → enabled c x (.dirSync ok) = true →
    Custody c (execute c x (.dirSync ok))
  draftSync : ∀ x p, Reachable c x → Custody c x → enabled c x (.draftSync p) = true →
    Custody c (execute c x (.draftSync p))

theorem custody_positiveTexts_of_residual4 (c : Config P) (T : Nat) (texts : c.texts = positiveTexts T)
    (mutant : c.mutant = "none") (residual : CustodyResidual4 c) (x : State P) (hx : Reachable c x) :
    Custody c x := by
  apply custody_reachable_of_residual c mutant
  exact ⟨residual.delivery,residual.rename,residual.dirSync,residual.draftSync,
    fun x hx h hen => custody_launch c T texts mutant x hx h hen⟩
  exact hx

theorem noLoss_positiveTexts_of_residual4 (c : Config P) (T : Nat) (texts : c.texts = positiveTexts T)
    (mutant : c.mutant = "none") (residual : CustodyResidual4 c) (x : State P) (hx : Reachable c x) :
    noLoss x = true := noLoss_of_custody c x (custody_positiveTexts_of_residual4 c T texts mutant residual x hx)

#print axioms promiseBytes_reachable
#print axioms custody_launch
#print axioms noLoss_positiveTexts_of_residual4
end Storage
