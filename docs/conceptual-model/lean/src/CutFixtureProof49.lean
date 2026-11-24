import CutFixtureProof48
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
theorem native_read_147 : nativeRead .md "bx" = nativeContent_147 := by decide +kernel
theorem fragment_law_147 : fragmentLaw .md nativeContent_147 = true := by decide +kernel
theorem geometry_law_147 : validSites o ⟨"fixture",.pages,.md,"bx"⟩ = true := by decide +kernel
theorem title_parts_law_147 : titlePartsLaw nativeContent_147 = true := by decide +kernel
theorem native_read_148 : nativeRead .md "alias:: A/sub" = nativeContent_148 := by decide +kernel
theorem fragment_law_148 : fragmentLaw .md nativeContent_148 = true := by decide +kernel
theorem geometry_law_148 : validSites o ⟨"fixture",.pages,.md,"alias:: A/sub"⟩ = true := by decide +kernel
theorem title_parts_law_148 : titlePartsLaw nativeContent_148 = true := by decide +kernel
theorem native_read_149 : nativeRead .md "bs" = nativeContent_149 := by decide +kernel
theorem fragment_law_149 : fragmentLaw .md nativeContent_149 = true := by decide +kernel
theorem geometry_law_149 : validSites o ⟨"fixture",.pages,.md,"bs"⟩ = true := by decide +kernel
theorem title_parts_law_149 : titlePartsLaw nativeContent_149 = true := by decide +kernel

end CutExamples
end CM
