import CutFixtureProof15
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
theorem native_read_48 : nativeRead .md "title:: A/x\n- p\n" = nativeContent_48 := by decide +kernel
theorem fragment_law_48 : fragmentLaw .md nativeContent_48 = true := by decide +kernel
theorem geometry_law_48 : validSites o ⟨"fixture",.pages,.md,"title:: A/x\n- p\n"⟩ = true := by decide +kernel
theorem title_parts_law_48 : titlePartsLaw nativeContent_48 = true := by decide +kernel
theorem native_read_49 : nativeRead .md "title:: A/x\n- q\n" = nativeContent_49 := by decide +kernel
theorem fragment_law_49 : fragmentLaw .md nativeContent_49 = true := by decide +kernel
theorem geometry_law_49 : validSites o ⟨"fixture",.pages,.md,"title:: A/x\n- q\n"⟩ = true := by decide +kernel
theorem title_parts_law_49 : titlePartsLaw nativeContent_49 = true := by decide +kernel
theorem native_read_50 : nativeRead .md "title:: C\n- occupied\n" = nativeContent_50 := by decide +kernel
theorem fragment_law_50 : fragmentLaw .md nativeContent_50 = true := by decide +kernel
theorem geometry_law_50 : validSites o ⟨"fixture",.pages,.md,"title:: C\n- occupied\n"⟩ = true := by decide +kernel
theorem title_parts_law_50 : titlePartsLaw nativeContent_50 = true := by decide +kernel

end CutExamples
end CM
