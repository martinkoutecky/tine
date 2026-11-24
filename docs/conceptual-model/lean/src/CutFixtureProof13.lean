import CutFixtureProof12
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
theorem native_read_39 : nativeRead .md "alias::\n- target\n- source\n" = nativeContent_39 := by decide +kernel
theorem fragment_law_39 : fragmentLaw .md nativeContent_39 = true := by decide +kernel
theorem geometry_law_39 : validSites o ⟨"fixture",.pages,.md,"alias::\n- target\n- source\n"⟩ = true := by decide +kernel
theorem title_parts_law_39 : titlePartsLaw nativeContent_39 = true := by decide +kernel
theorem native_read_40 : nativeRead .md "source prose\n\n- source\n" = nativeContent_40 := by decide +kernel
theorem fragment_law_40 : fragmentLaw .md nativeContent_40 = true := by decide +kernel
theorem geometry_law_40 : validSites o ⟨"fixture",.pages,.md,"source prose\n\n- source\n"⟩ = true := by decide +kernel
theorem title_parts_law_40 : titlePartsLaw nativeContent_40 = true := by decide +kernel
theorem native_read_41 : nativeRead .md "target prose\n\n- target\n" = nativeContent_41 := by decide +kernel
theorem fragment_law_41 : fragmentLaw .md nativeContent_41 = true := by decide +kernel
theorem geometry_law_41 : validSites o ⟨"fixture",.pages,.md,"target prose\n\n- target\n"⟩ = true := by decide +kernel
theorem title_parts_law_41 : titlePartsLaw nativeContent_41 = true := by decide +kernel

end CutExamples
end CM
