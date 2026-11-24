import CutFixtureProof00
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
theorem native_read_3 : nativeRead .md "- source\n" = nativeContent_3 := by decide +kernel
theorem fragment_law_3 : fragmentLaw .md nativeContent_3 = true := by decide +kernel
theorem geometry_law_3 : validSites o ⟨"fixture",.pages,.md,"- source\n"⟩ = true := by decide +kernel
theorem title_parts_law_3 : titlePartsLaw nativeContent_3 = true := by decide +kernel
theorem native_read_4 : nativeRead .md "- target\n" = nativeContent_4 := by decide +kernel
theorem fragment_law_4 : fragmentLaw .md nativeContent_4 = true := by decide +kernel
theorem geometry_law_4 : validSites o ⟨"fixture",.pages,.md,"- target\n"⟩ = true := by decide +kernel
theorem title_parts_law_4 : titlePartsLaw nativeContent_4 = true := by decide +kernel
theorem native_read_5 : nativeRead .md "- target\n- source\n" = nativeContent_5 := by decide +kernel
theorem fragment_law_5 : fragmentLaw .md nativeContent_5 = true := by decide +kernel
theorem geometry_law_5 : validSites o ⟨"fixture",.pages,.md,"- target\n- source\n"⟩ = true := by decide +kernel
theorem title_parts_law_5 : titlePartsLaw nativeContent_5 = true := by decide +kernel

end CutExamples
end CM
