import CutFixtureProof08
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
theorem native_read_27 : nativeRead .md "x:: one\nx:: two\n- source\n" = nativeContent_27 := by decide +kernel
theorem fragment_law_27 : fragmentLaw .md nativeContent_27 = true := by decide +kernel
theorem geometry_law_27 : validSites o ⟨"fixture",.pages,.md,"x:: one\nx:: two\n- source\n"⟩ = true := by decide +kernel
theorem title_parts_law_27 : titlePartsLaw nativeContent_27 = true := by decide +kernel
theorem native_read_28 : nativeRead .md "x:: one\n- target\n- \n  x:: two\n- source\n" = nativeContent_28 := by decide +kernel
theorem fragment_law_28 : fragmentLaw .md nativeContent_28 = true := by decide +kernel
theorem geometry_law_28 : validSites o ⟨"fixture",.pages,.md,"x:: one\n- target\n- \n  x:: two\n- source\n"⟩ = true := by decide +kernel
theorem title_parts_law_28 : titlePartsLaw nativeContent_28 = true := by decide +kernel
theorem native_read_29 : nativeRead .md "alias:: A\n- owner\n" = nativeContent_29 := by decide +kernel
theorem fragment_law_29 : fragmentLaw .md nativeContent_29 = true := by decide +kernel
theorem geometry_law_29 : validSites o ⟨"fixture",.pages,.md,"alias:: A\n- owner\n"⟩ = true := by decide +kernel
theorem title_parts_law_29 : titlePartsLaw nativeContent_29 = true := by decide +kernel

end CutExamples
end CM
