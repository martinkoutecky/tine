import CustodySteps
namespace Storage
set_option maxRecDepth 8192
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false

def custodyPending (a : Action P) : Bool := match a with
  | .deliverUp _ | .rename | .dirSync _ | .draftSync _ | .launch => true
  | _ => false

structure CustodyResidual (c : Config P) : Prop where
  delivery : ∀ x ok, Reachable c x → Custody c x → enabled c x (.deliverUp ok) = true →
    Custody c (execute c x (.deliverUp ok))
  rename : ∀ x, Reachable c x → Custody c x → enabled c x .rename = true →
    Custody c (execute c x .rename)
  dirSync : ∀ x ok, Reachable c x → Custody c x → enabled c x (.dirSync ok) = true →
    Custody c (execute c x (.dirSync ok))
  draftSync : ∀ x p, Reachable c x → Custody c x → enabled c x (.draftSync p) = true →
    Custody c (execute c x (.draftSync p))
  launch : ∀ x, Reachable c x → Custody c x → enabled c x .launch = true →
    Custody c (execute c x .launch)

theorem custody_execute_closed (c : Config P) (mutant : c.mutant = "none") (x : State P) (a : Action P)
    (h : Custody c x) (shape : Shapes x.s) (epochs : EpochBounds x.s x.g)
    (closed : custodyPending a = false) : Custody c (execute c x a) := by
  cases a <;> simp only [execute]
  case deliverUp ok => simp [custodyPending] at closed
  case rename => simp [custodyPending] at closed
  case dirSync ok => simp [custodyPending] at closed
  case draftSync p => simp [custodyPending] at closed
  case launch => simp [custodyPending] at closed
  case observe p => exact custody_observe c mutant x p h shape
  case check => exact custody_check c mutant x h shape
  case saveFail => exact custody_saveFail c mutant x h
  case switchReq => exact custody_switchReq c x h
  case switchFin => exact custody_switchFin c x h
  case crash => exact custody_crash c x h
  case power keep => exact custody_power c x keep h
  case extWriteD p v dur => exact custody_extWriteD c x p v dur h epochs
  all_goals apply custody_kept_commit c x _ _ _ h
  all_goals first | rfl | (intro p hp; exact hp) | exact postPromises_same_pages x _ _ rfl

theorem custody_reachable_of_residual (c : Config P) (mutant : c.mutant = "none")
    (residual : CustodyResidual c) (x : State P) (hx : Reachable c x) : Custody c x := by
  induction hx with
  | initial => exact custody_init c
  | @advance x y hr a hs ih =>
    have shape := shapes_reachable c mutant x hr
    have ep := epochs_reachable c x hr
    unfold step at hs; split at hs
    · cases hs
      by_cases closed : custodyPending a = false
      · exact custody_execute_closed c mutant x a ih shape ep closed
      · cases a <;> simp only [custodyPending] at closed
        case deliverUp => exact residual.delivery x _ hr ih (by assumption)
        case rename => exact residual.rename x hr ih (by assumption)
        case dirSync => exact residual.dirSync x _ hr ih (by assumption)
        case draftSync => exact residual.draftSync x _ hr ih (by assumption)
        case launch => exact residual.launch x hr ih (by assumption)
        all_goals contradiction
    · contradiction

theorem noLoss_reachable_of_residual (c : Config P) (mutant : c.mutant = "none")
    (residual : CustodyResidual c) (x : State P) (hx : Reachable c x) : noLoss x = true :=
  noLoss_of_custody c x (custody_reachable_of_residual c mutant residual x hx)

#print axioms noLoss_iff_recoverable
#print axioms epochs_reachable
#print axioms custody_power
#print axioms custody_execute_closed
#print axioms noLoss_reachable_of_residual
end Storage
