import CutFixtureProof14
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
theorem native_read_45 : nativeRead .md "alias:: X\n- owner\n" = nativeContent_45 := by decide +kernel
theorem fragment_law_45 : fragmentLaw .md nativeContent_45 = true := by decide +kernel
theorem geometry_law_45 : validSites o ⟨"fixture",.pages,.md,"alias:: X\n- owner\n"⟩ = true := by decide +kernel
theorem title_parts_law_45 : titlePartsLaw nativeContent_45 = true := by decide +kernel
theorem native_read_46 : nativeRead .md "title:: X\n- p\n" = nativeContent_46 := by decide +kernel
theorem fragment_law_46 : fragmentLaw .md nativeContent_46 = true := by decide +kernel
theorem geometry_law_46 : validSites o ⟨"fixture",.pages,.md,"title:: X\n- p\n"⟩ = true := by decide +kernel
theorem title_parts_law_46 : titlePartsLaw nativeContent_46 = true := by decide +kernel
theorem native_read_47 : nativeRead .md "title:: X\n- q\n" = nativeContent_47 := by decide +kernel
theorem fragment_law_47 : fragmentLaw .md nativeContent_47 = true := by decide +kernel
theorem geometry_law_47 : validSites o ⟨"fixture",.pages,.md,"title:: X\n- q\n"⟩ = true := by decide +kernel
theorem title_parts_law_47 : titlePartsLaw nativeContent_47 = true := by decide +kernel

end CutExamples
end CM
