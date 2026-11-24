import CutFixtureProof49
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
theorem native_read_150 : nativeRead .md "alias:: Y" = nativeContent_150 := by decide +kernel
theorem fragment_law_150 : fragmentLaw .md nativeContent_150 = true := by decide +kernel
theorem geometry_law_150 : validSites o ⟨"fixture",.pages,.md,"alias:: Y"⟩ = true := by decide +kernel
theorem title_parts_law_150 : titlePartsLaw nativeContent_150 = true := by decide +kernel
theorem native_read_151 : nativeRead .md "alias:: B/sub" = nativeContent_151 := by decide +kernel
theorem fragment_law_151 : fragmentLaw .md nativeContent_151 = true := by decide +kernel
theorem geometry_law_151 : validSites o ⟨"fixture",.pages,.md,"alias:: B/sub"⟩ = true := by decide +kernel
theorem title_parts_law_151 : titlePartsLaw nativeContent_151 = true := by decide +kernel
theorem native_read_152 : nativeRead .org "#+key: [[X]]" = nativeContent_152 := by decide +kernel
theorem fragment_law_152 : fragmentLaw .org nativeContent_152 = true := by decide +kernel
theorem geometry_law_152 : validSites o ⟨"fixture",.pages,.org,"#+key: [[X]]"⟩ = true := by decide +kernel
theorem title_parts_law_152 : titlePartsLaw nativeContent_152 = true := by decide +kernel

end CutExamples
end CM
