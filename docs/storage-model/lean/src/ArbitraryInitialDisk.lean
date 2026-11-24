import Resolution11

namespace Storage35
open Storage21 Storage3

/-- The s3.5 safety properties for every reachable state of a disk whose
present entries come from the configured text vocabulary. Reserve the labels
for a missing file, a missing draft and an unknown base. -/
theorem all_targets_arbitrary_initial_disk (c : Config P) (disk : Fin P → Int) (x : State P)
    (mutant : c.mutant = "none") (realDisk : ∀ p, RealByte c (disk p))
    (textNondelete : absent ∉ c.texts) (textNoNone : noneBytes ∉ c.texts)
    (textNoUnknown : unknown ∉ c.texts) (hr : Reachable c disk x) :
    guarantee x = true ∧ orderHolds x = true ∧ renameResolves x = true := by
  have noNone : NoNone6 c := by
    intro b hb
    rcases hb with rfl | mem
    · decide
    · exact fun eq => textNoNone (eq ▸ mem)
  have noUnknown : NoUnknown6 c := by
    intro b hb
    rcases hb with rfl | mem
    · decide
    · exact fun eq => textNoUnknown (eq ▸ mem)
  exact ⟨Storage31.guarantee_of_inv7 c _ (inv_reachable35_10 c disk x mutant realDisk noNone noUnknown hr),
    orderHolds_reachable35_10 c disk x mutant hr,
    pairSafety_resolves10 x (pairSafety_reachable35_11 c disk x mutant textNondelete realDisk noNone noUnknown hr)⟩

#print axioms all_targets_arbitrary_initial_disk
end Storage35
