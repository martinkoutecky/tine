import Bytes4
namespace Storage3
open Storage21
set_option maxRecDepth 8192
set_option maxHeartbeats 4000000

def positiveTexts4 (T : Nat) : List Int := (List.range T).map fun n => (n+1 : Nat)

theorem positiveTexts_initial_real4 (c : Config P) (T : Nat)
    (texts : c.texts = positiveTexts4 T) (scope : 2 ≤ T) : ∀ p, RealByte c (initDisk c p) := by
  intro p
  by_cases first : p.val = 0
  · right; simp only [initDisk,first,beq_self_eq_true,ite_true,texts]
    apply List.mem_map.mpr; exact ⟨0,by simp; omega,rfl⟩
  · by_cases second : p.val = 1
    · right; simp only [initDisk,first,second,beq_iff_eq,ite_false,ite_true,texts]
      apply List.mem_map.mpr; exact ⟨1,by simp; omega,rfl⟩
    · left; simp [initDisk,first,second]

theorem bytes_positiveTexts4 (c : Config P) (T : Nat) (texts : c.texts = positiveTexts4 T)
    (scope : 2 ≤ T) (mutant : c.mutant = "none") (x : State P)
    (hr : Reachable c (initDisk c) x) : ByteDomain c x.s.toSys :=
  bytes_reachable4 c (initDisk c) x mutant (positiveTexts_initial_real4 c T texts scope) hr

theorem carrierSeen_positiveTexts4 (c : Config P) (T : Nat) (texts : c.texts = positiveTexts4 T)
    (scope : 2 ≤ T) (mutant : c.mutant = "none") (x : State P)
    (hr : Reachable c (initDisk c) x) : CarrierSeen (project x) :=
  carrierSeen_reachable4 c (initDisk c) x mutant (positiveTexts_initial_real4 c T texts scope) hr
end Storage3
