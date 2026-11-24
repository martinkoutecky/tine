import CutFixtureProof06
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
theorem native_read_21 : nativeRead .md "- Old:: value\n- Old:: another\n" = nativeContent_21 := by decide +kernel
theorem fragment_law_21 : fragmentLaw .md nativeContent_21 = true := by decide +kernel
theorem geometry_law_21 : validSites o ⟨"fixture",.pages,.md,"- Old:: value\n- Old:: another\n"⟩ = true := by decide +kernel
theorem title_parts_law_21 : titlePartsLaw nativeContent_21 = true := by decide +kernel
theorem native_read_22 : nativeRead .md "- two-words:: value\n- two-words:: another\n" = nativeContent_22 := by decide +kernel
theorem fragment_law_22 : fragmentLaw .md nativeContent_22 = true := by decide +kernel
theorem geometry_law_22 : validSites o ⟨"fixture",.pages,.md,"- two-words:: value\n- two-words:: another\n"⟩ = true := by decide +kernel
theorem title_parts_law_22 : titlePartsLaw nativeContent_22 = true := by decide +kernel
theorem native_read_23 : nativeRead .md "alias:: X\n- source\n" = nativeContent_23 := by decide +kernel
theorem fragment_law_23 : fragmentLaw .md nativeContent_23 = true := by decide +kernel
theorem geometry_law_23 : validSites o ⟨"fixture",.pages,.md,"alias:: X\n- source\n"⟩ = true := by decide +kernel
theorem title_parts_law_23 : titlePartsLaw nativeContent_23 = true := by decide +kernel

end CutExamples
end CM
