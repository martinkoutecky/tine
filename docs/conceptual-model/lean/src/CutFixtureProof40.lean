import CutFixtureProof39
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
theorem native_read_120 : nativeRead .md "alias:: B" = nativeContent_120 := by decide +kernel
theorem fragment_law_120 : fragmentLaw .md nativeContent_120 = true := by decide +kernel
theorem geometry_law_120 : validSites o ⟨"fixture",.pages,.md,"alias:: B"⟩ = true := by decide +kernel
theorem title_parts_law_120 : titlePartsLaw nativeContent_120 = true := by decide +kernel
theorem native_read_121 : nativeRead .md "title:: 2026-11-02" = nativeContent_121 := by decide +kernel
theorem fragment_law_121 : fragmentLaw .md nativeContent_121 = true := by decide +kernel
theorem geometry_law_121 : validSites o ⟨"fixture",.pages,.md,"title:: 2026-11-02"⟩ = true := by decide +kernel
theorem title_parts_law_121 : titlePartsLaw nativeContent_121 = true := by decide +kernel
theorem native_read_122 : nativeRead .md "title:: Old/[[New]] [[X]]" = nativeContent_122 := by decide +kernel
theorem fragment_law_122 : fragmentLaw .md nativeContent_122 = true := by decide +kernel
theorem geometry_law_122 : validSites o ⟨"fixture",.pages,.md,"title:: Old/[[New]] [[X]]"⟩ = true := by decide +kernel
theorem title_parts_law_122 : titlePartsLaw nativeContent_122 = true := by decide +kernel

end CutExamples
end CM
