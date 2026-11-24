import CutFixtureData
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
theorem native_read_0 : nativeRead .md "- notes\n" = nativeContent_0 := by decide +kernel
theorem fragment_law_0 : fragmentLaw .md nativeContent_0 = true := by decide +kernel
theorem geometry_law_0 : validSites o ⟨"fixture",.pages,.md,"- notes\n"⟩ = true := by decide +kernel
theorem title_parts_law_0 : titlePartsLaw nativeContent_0 = true := by decide +kernel
theorem native_read_1 : nativeRead .md "- [[A]]\n" = nativeContent_1 := by decide +kernel
theorem fragment_law_1 : fragmentLaw .md nativeContent_1 = true := by decide +kernel
theorem geometry_law_1 : validSites o ⟨"fixture",.pages,.md,"- [[A]]\n"⟩ = true := by decide +kernel
theorem title_parts_law_1 : titlePartsLaw nativeContent_1 = true := by decide +kernel
theorem native_read_2 : nativeRead .md "- [[B]]\n" = nativeContent_2 := by decide +kernel
theorem fragment_law_2 : fragmentLaw .md nativeContent_2 = true := by decide +kernel
theorem geometry_law_2 : validSites o ⟨"fixture",.pages,.md,"- [[B]]\n"⟩ = true := by decide +kernel
theorem title_parts_law_2 : titlePartsLaw nativeContent_2 = true := by decide +kernel

end CutExamples
end CM
