import CutFixtureProof45
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
theorem native_read_138 : nativeRead .md "title:: New/[[New]]" = nativeContent_138 := by decide +kernel
theorem fragment_law_138 : fragmentLaw .md nativeContent_138 = true := by decide +kernel
theorem geometry_law_138 : validSites o ⟨"fixture",.pages,.md,"title:: New/[[New]]"⟩ = true := by decide +kernel
theorem title_parts_law_138 : titlePartsLaw nativeContent_138 = true := by decide +kernel
theorem native_read_139 : nativeRead .md "title:: [[X]]" = nativeContent_139 := by decide +kernel
theorem fragment_law_139 : fragmentLaw .md nativeContent_139 = true := by decide +kernel
theorem geometry_law_139 : validSites o ⟨"fixture",.pages,.md,"title:: [[X]]"⟩ = true := by decide +kernel
theorem title_parts_law_139 : titlePartsLaw nativeContent_139 = true := by decide +kernel
theorem native_read_140 : nativeRead .md "title:: 2026/10/26" = nativeContent_140 := by decide +kernel
theorem fragment_law_140 : fragmentLaw .md nativeContent_140 = true := by decide +kernel
theorem geometry_law_140 : validSites o ⟨"fixture",.pages,.md,"title:: 2026/10/26"⟩ = true := by decide +kernel
theorem title_parts_law_140 : titlePartsLaw nativeContent_140 = true := by decide +kernel

end CutExamples
end CM
