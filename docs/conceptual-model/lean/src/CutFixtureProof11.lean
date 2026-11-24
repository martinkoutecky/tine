import CutFixtureProof10
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
theorem native_read_33 : nativeRead .md "title:: Old/[[New]] [[X]]\n- child\n" = nativeContent_33 := by decide +kernel
theorem fragment_law_33 : fragmentLaw .md nativeContent_33 = true := by decide +kernel
theorem geometry_law_33 : validSites o ⟨"fixture",.pages,.md,"title:: Old/[[New]] [[X]]\n- child\n"⟩ = true := by decide +kernel
theorem title_parts_law_33 : titlePartsLaw nativeContent_33 = true := by decide +kernel
theorem native_read_34 : nativeRead .md "- target\n- notes\n" = nativeContent_34 := by decide +kernel
theorem fragment_law_34 : fragmentLaw .md nativeContent_34 = true := by decide +kernel
theorem geometry_law_34 : validSites o ⟨"fixture",.pages,.md,"- target\n- notes\n"⟩ = true := by decide +kernel
theorem title_parts_law_34 : titlePartsLaw nativeContent_34 = true := by decide +kernel
theorem native_read_35 : nativeRead .md "title:: New/[[New]] [[X]]\n- child\n" = nativeContent_35 := by decide +kernel
theorem fragment_law_35 : fragmentLaw .md nativeContent_35 = true := by decide +kernel
theorem geometry_law_35 : validSites o ⟨"fixture",.pages,.md,"title:: New/[[New]] [[X]]\n- child\n"⟩ = true := by decide +kernel
theorem title_parts_law_35 : titlePartsLaw nativeContent_35 = true := by decide +kernel

end CutExamples
end CM
