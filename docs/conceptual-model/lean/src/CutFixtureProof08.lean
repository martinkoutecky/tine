import CutFixtureProof07
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
theorem native_read_24 : nativeRead .md "title:: X\n- source\n" = nativeContent_24 := by decide +kernel
theorem fragment_law_24 : fragmentLaw .md nativeContent_24 = true := by decide +kernel
theorem geometry_law_24 : validSites o ⟨"fixture",.pages,.md,"title:: X\n- source\n"⟩ = true := by decide +kernel
theorem title_parts_law_24 : titlePartsLaw nativeContent_24 = true := by decide +kernel
theorem native_read_25 : nativeRead .md "title:: X\n- target\n" = nativeContent_25 := by decide +kernel
theorem fragment_law_25 : fragmentLaw .md nativeContent_25 = true := by decide +kernel
theorem geometry_law_25 : validSites o ⟨"fixture",.pages,.md,"title:: X\n- target\n"⟩ = true := by decide +kernel
theorem title_parts_law_25 : titlePartsLaw nativeContent_25 = true := by decide +kernel
theorem native_read_26 : nativeRead .md "title:: X\n- target\n- \n  title:: X\n- source\n" = nativeContent_26 := by decide +kernel
theorem fragment_law_26 : fragmentLaw .md nativeContent_26 = true := by decide +kernel
theorem geometry_law_26 : validSites o ⟨"fixture",.pages,.md,"title:: X\n- target\n- \n  title:: X\n- source\n"⟩ = true := by decide +kernel
theorem title_parts_law_26 : titlePartsLaw nativeContent_26 = true := by decide +kernel

end CutExamples
end CM
