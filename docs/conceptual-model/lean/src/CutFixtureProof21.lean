import CutFixtureProof20
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
theorem native_read_63 : nativeRead .md "- bx\n" = nativeContent_63 := by decide +kernel
theorem fragment_law_63 : fragmentLaw .md nativeContent_63 = true := by decide +kernel
theorem geometry_law_63 : validSites o ⟨"fixture",.pages,.md,"- bx\n"⟩ = true := by decide +kernel
theorem title_parts_law_63 : titlePartsLaw nativeContent_63 = true := by decide +kernel
theorem native_read_64 : nativeRead .md "alias:: A/sub\n- owner\n" = nativeContent_64 := by decide +kernel
theorem fragment_law_64 : fragmentLaw .md nativeContent_64 = true := by decide +kernel
theorem geometry_law_64 : validSites o ⟨"fixture",.pages,.md,"alias:: A/sub\n- owner\n"⟩ = true := by decide +kernel
theorem title_parts_law_64 : titlePartsLaw nativeContent_64 = true := by decide +kernel
theorem native_read_65 : nativeRead .md "- bs\n" = nativeContent_65 := by decide +kernel
theorem fragment_law_65 : fragmentLaw .md nativeContent_65 = true := by decide +kernel
theorem geometry_law_65 : validSites o ⟨"fixture",.pages,.md,"- bs\n"⟩ = true := by decide +kernel
theorem title_parts_law_65 : titlePartsLaw nativeContent_65 = true := by decide +kernel

end CutExamples
end CM
