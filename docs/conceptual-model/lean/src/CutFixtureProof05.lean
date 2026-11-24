import CutFixtureProof04
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
theorem native_read_15 : nativeRead .md "- #Old\n" = nativeContent_15 := by decide +kernel
theorem fragment_law_15 : fragmentLaw .md nativeContent_15 = true := by decide +kernel
theorem geometry_law_15 : validSites o ⟨"fixture",.pages,.md,"- #Old\n"⟩ = true := by decide +kernel
theorem title_parts_law_15 : titlePartsLaw nativeContent_15 = true := by decide +kernel
theorem native_read_16 : nativeRead .md "- #[[Two Words]]\n" = nativeContent_16 := by decide +kernel
theorem fragment_law_16 : fragmentLaw .md nativeContent_16 = true := by decide +kernel
theorem geometry_law_16 : validSites o ⟨"fixture",.pages,.md,"- #[[Two Words]]\n"⟩ = true := by decide +kernel
theorem title_parts_law_16 : titlePartsLaw nativeContent_16 = true := by decide +kernel
theorem native_read_17 : nativeRead .md "- target" = nativeContent_17 := by decide +kernel
theorem fragment_law_17 : fragmentLaw .md nativeContent_17 = true := by decide +kernel
theorem geometry_law_17 : validSites o ⟨"fixture",.pages,.md,"- target"⟩ = true := by decide +kernel
theorem title_parts_law_17 : titlePartsLaw nativeContent_17 = true := by decide +kernel

end CutExamples
end CM
