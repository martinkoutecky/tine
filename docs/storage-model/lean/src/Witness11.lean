import Resolution11
import Witness35
namespace Witness35
open Storage21 Storage32 Storage35
set_option maxHeartbeats 4000000
set_option maxRecDepth 8192
theorem completed_targets11 :
    (Storage35.guarantee moved = true ∧ Storage35.orderHolds moved = true ∧ Storage35.renameResolves moved = true) ∧
    (Storage35.guarantee restored = true ∧ Storage35.orderHolds restored = true ∧ Storage35.renameResolves restored = true) ∧
    (Storage35.guarantee applied = true ∧ Storage35.orderHolds applied = true ∧ Storage35.renameResolves applied = true) :=
  ⟨all_targets_reachable35_11 c 3 rfl (by decide) rfl moved move_reachable10,
    all_targets_reachable35_11 c 3 rfl (by decide) rfl restored restoration_reachable10,
    all_targets_reachable35_11 c 3 rfl (by decide) rfl applied apply_move_reachable10⟩

theorem overlapping_not_owned11 : ¬ GateOwnership11 overlapping.s.gates := by
  intro own
  have safe : PairSafety10 waiting := pairSafety_reachable35_11 c (Storage31.initDisk c) waiting rfl
    (positiveTexts_nondelete11 c 3 rfl) (Storage3.positiveTexts_initial_real4 c 3 rfl (by decide))
    (Storage3.positiveTexts_noNone6 c 3 rfl) (Storage3.positiveTexts_noUnknown6 c 3 rfl) waiting_reachable10
  have pairmem : (p0,p2) ∈ waiting.g.renamed := by decide
  rcases safe (p0,p2) pairmem with src | dst | pending
  · have held : (waiting.s.pages p0).held = true := by decide
    have deleted : (waiting.s.pages p0).buf = absent := by decide
    exact src.2.1 held deleted
  · have missing : waiting.s.disk p2 = absent := by decide
    exact dst.1 missing
  · obtain ⟨gt,gm,gs,gd,_,_⟩ := pending.2.2.2.2.2.2
    have gmem : gt ∈ overlapping.s.gates := List.mem_cons_of_mem _ gm
    have foreign : foreignGate ∈ overlapping.s.gates := List.mem_cons_self
    have eq := own.2 foreignGate foreign gt gmem p2 (by decide)
      (by simpa [gd] using (own.1 gt gmem).2.2.1)
    have bad : p1 = p0 := by simpa [foreignGate,gs] using eq.1
    exact (by decide : p1 ≠ p0) bad

theorem overlapping_unreachable11 : ¬ Storage35.Reachable c (Storage31.initDisk c) overlapping := by
  intro hr
  exact overlapping_not_owned11 (gateOwnership_reachable35_11 c (Storage31.initDisk c) overlapping rfl hr)
end Witness35
