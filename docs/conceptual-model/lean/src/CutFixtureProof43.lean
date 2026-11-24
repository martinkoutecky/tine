import CutFixtureProof42
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
theorem native_read_129 : nativeRead .md "target prose\n\n" = nativeContent_129 := by decide +kernel
theorem fragment_law_129 : fragmentLaw .md nativeContent_129 = true := by decide +kernel
theorem geometry_law_129 : validSites o ⟨"fixture",.pages,.md,"target prose\n\n"⟩ = true := by decide +kernel
theorem title_parts_law_129 : titlePartsLaw nativeContent_129 = true := by decide +kernel
theorem native_read_130 : nativeRead .md "title:: A" = nativeContent_130 := by decide +kernel
theorem fragment_law_130 : fragmentLaw .md nativeContent_130 = true := by decide +kernel
theorem geometry_law_130 : validSites o ⟨"fixture",.pages,.md,"title:: A"⟩ = true := by decide +kernel
theorem title_parts_law_130 : titlePartsLaw nativeContent_130 = true := by decide +kernel
theorem native_read_131 : nativeRead .md "kid" = nativeContent_131 := by decide +kernel
theorem fragment_law_131 : fragmentLaw .md nativeContent_131 = true := by decide +kernel
theorem geometry_law_131 : validSites o ⟨"fixture",.pages,.md,"kid"⟩ = true := by decide +kernel
theorem title_parts_law_131 : titlePartsLaw nativeContent_131 = true := by decide +kernel

end CutExamples
end CM
