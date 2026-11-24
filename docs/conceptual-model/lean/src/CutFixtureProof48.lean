import CutFixtureProof47
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
theorem native_read_144 : nativeRead .md "" = nativeContent_144 := by decide +kernel
theorem fragment_law_144 : fragmentLaw .md nativeContent_144 = true := by decide +kernel
theorem geometry_law_144 : validSites o ⟨"fixture",.pages,.md,""⟩ = true := by decide +kernel
theorem title_parts_law_144 : titlePartsLaw nativeContent_144 = true := by decide +kernel
theorem native_read_145 : nativeRead .org "h :New:" = nativeContent_145 := by decide +kernel
theorem fragment_law_145 : fragmentLaw .org nativeContent_145 = true := by decide +kernel
theorem geometry_law_145 : validSites o ⟨"fixture",.pages,.org,"h :New:"⟩ = true := by decide +kernel
theorem title_parts_law_145 : titlePartsLaw nativeContent_145 = true := by decide +kernel
theorem native_read_146 : nativeRead .md "[[A/x]]" = nativeContent_146 := by decide +kernel
theorem fragment_law_146 : fragmentLaw .md nativeContent_146 = true := by decide +kernel
theorem geometry_law_146 : validSites o ⟨"fixture",.pages,.md,"[[A/x]]"⟩ = true := by decide +kernel
theorem title_parts_law_146 : titlePartsLaw nativeContent_146 = true := by decide +kernel

end CutExamples
end CM
