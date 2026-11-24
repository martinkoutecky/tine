import CutFixtureProof05
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
theorem native_read_18 : nativeRead .md "alias:: C#\n- source\n" = nativeContent_18 := by decide +kernel
theorem fragment_law_18 : fragmentLaw .md nativeContent_18 = true := by decide +kernel
theorem geometry_law_18 : validSites o ⟨"fixture",.pages,.md,"alias:: C#\n- source\n"⟩ = true := by decide +kernel
theorem title_parts_law_18 : titlePartsLaw nativeContent_18 = true := by decide +kernel
theorem native_read_19 : nativeRead .md "alias:: C\\#\n- target\n" = nativeContent_19 := by decide +kernel
theorem fragment_law_19 : fragmentLaw .md nativeContent_19 = true := by decide +kernel
theorem geometry_law_19 : validSites o ⟨"fixture",.pages,.md,"alias:: C\\#\n- target\n"⟩ = true := by decide +kernel
theorem title_parts_law_19 : titlePartsLaw nativeContent_19 = true := by decide +kernel
theorem native_read_20 : nativeRead .md "alias:: C\\#\n- target\n- source\n" = nativeContent_20 := by decide +kernel
theorem fragment_law_20 : fragmentLaw .md nativeContent_20 = true := by decide +kernel
theorem geometry_law_20 : validSites o ⟨"fixture",.pages,.md,"alias:: C\\#\n- target\n- source\n"⟩ = true := by decide +kernel
theorem title_parts_law_20 : titlePartsLaw nativeContent_20 = true := by decide +kernel

end CutExamples
end CM
