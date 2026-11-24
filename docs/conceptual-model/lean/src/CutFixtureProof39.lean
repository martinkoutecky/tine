import CutFixtureProof38
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
theorem native_read_117 : nativeRead .md "\nx:: two" = nativeContent_117 := by decide +kernel
theorem fragment_law_117 : fragmentLaw .md nativeContent_117 = true := by decide +kernel
theorem geometry_law_117 : validSites o ⟨"fixture",.pages,.md,"\nx:: two"⟩ = true := by decide +kernel
theorem title_parts_law_117 : titlePartsLaw nativeContent_117 = true := by decide +kernel
theorem native_read_118 : nativeRead .md "alias:: A" = nativeContent_118 := by decide +kernel
theorem fragment_law_118 : fragmentLaw .md nativeContent_118 = true := by decide +kernel
theorem geometry_law_118 : validSites o ⟨"fixture",.pages,.md,"alias:: A"⟩ = true := by decide +kernel
theorem title_parts_law_118 : titlePartsLaw nativeContent_118 = true := by decide +kernel
theorem native_read_119 : nativeRead .md "owner" = nativeContent_119 := by decide +kernel
theorem fragment_law_119 : fragmentLaw .md nativeContent_119 = true := by decide +kernel
theorem geometry_law_119 : validSites o ⟨"fixture",.pages,.md,"owner"⟩ = true := by decide +kernel
theorem title_parts_law_119 : titlePartsLaw nativeContent_119 = true := by decide +kernel

end CutExamples
end CM
