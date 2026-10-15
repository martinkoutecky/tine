import PairActions11
namespace Storage35
open Storage21 Storage3 Storage32
set_option maxHeartbeats 4000000
set_option maxRecDepth 8192
set_option linter.unusedSimpArgs false
theorem pairSafety_reachable35_11 (c : Config P) (disk : Fin P → Int) (x : State P)
    (mutant : c.mutant = "none") (textNondelete : absent ∉ c.texts)
    (realDisk : ∀ p, RealByte c (disk p)) (noNone : NoNone6 c) (noUnknown : NoUnknown6 c)
    (hr : Reachable c disk x) : PairSafety10 x := by
  induction hr with
  | initial =>
    simp [PairSafety10,initWith,Storage34.initWith,Storage33.initWith,Storage32.initWith,
      Storage32.embed,Storage34.embed]
  | @advance x y hr a hs ih =>
    have inv := inv_reachable35_10 c disk x mutant realDisk noNone noUnknown hr
    have own := gateOwnership_reachable35_11 c disk x mutant hr
    have pub := pairPublished_reachable35_11 c disk x mutant hr
    have live := livePairs_reachable35_11 c disk x mutant hr
    have inputs := inputTexts_reachable35_11 c disk x mutant textNondelete hr
    unfold step at hs; split at hs
    · cases hs
      exact pairSafety_execute11 c x a mutant textNondelete ih own pub live inv.protocol.kinds inputs (by assumption)
    · contradiction

theorem positiveTexts_nondelete11 (c : Config P) (T : Nat) (texts : c.texts = positiveTexts4 T) :
    absent ∉ c.texts := by
  rw [texts]
  intro mem
  obtain ⟨n,_,eq⟩ := List.mem_map.mp mem
  change (n+1 : Nat) = (absent : Int) at eq
  unfold absent at eq
  omega

theorem renameResolves_reachable35_11 (c : Config P) (T : Nat)
    (texts : c.texts = positiveTexts4 T) (two : 2 ≤ T) (mutant : c.mutant = "none")
    (x : State P) (hr : Reachable c (Storage31.initDisk c) x) : renameResolves x = true := by
  apply pairSafety_resolves10 x
  exact pairSafety_reachable35_11 c (Storage31.initDisk c) x mutant (positiveTexts_nondelete11 c T texts)
    (positiveTexts_initial_real4 c T texts two) (positiveTexts_noNone6 c T texts)
    (positiveTexts_noUnknown6 c T texts) hr

-- Joint literal statement. The older headline theorems are used unchanged.
theorem all_targets_reachable35_11 (c : Config P) (T : Nat)
    (texts : c.texts = positiveTexts4 T) (two : 2 ≤ T) (mutant : c.mutant = "none")
    (x : State P) (hr : Reachable c (Storage31.initDisk c) x) :
    guarantee x = true ∧ orderHolds x = true ∧ renameResolves x = true :=
  ⟨guarantee_reachable35_10 c T texts two mutant x hr,
    orderHolds_reachable35_10 c _ x mutant hr,renameResolves_reachable35_11 c T texts two mutant x hr⟩
#print axioms pairSafety_reachable35_11
#print axioms renameResolves_reachable35_11
#print axioms all_targets_reachable35_11
end Storage35
