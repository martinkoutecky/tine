import CutFixtureProof16
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
theorem native_read_51 : nativeRead .md "- alias:: X\n  - kid\n- source\n" = nativeContent_51 := by decide +kernel
theorem fragment_law_51 : fragmentLaw .md nativeContent_51 = true := by decide +kernel
theorem geometry_law_51 : validSites o ⟨"fixture",.pages,.md,"- alias:: X\n  - kid\n- source\n"⟩ = true := by decide +kernel
theorem title_parts_law_51 : titlePartsLaw nativeContent_51 = true := by decide +kernel
theorem native_read_52 : nativeRead .md "title:: New/[[New]]\n- child\n" = nativeContent_52 := by decide +kernel
theorem fragment_law_52 : fragmentLaw .md nativeContent_52 = true := by decide +kernel
theorem geometry_law_52 : validSites o ⟨"fixture",.pages,.md,"title:: New/[[New]]\n- child\n"⟩ = true := by decide +kernel
theorem title_parts_law_52 : titlePartsLaw nativeContent_52 = true := by decide +kernel
theorem native_read_53 : nativeRead .md "title:: [[X]]\n- notes\n" = nativeContent_53 := by decide +kernel
theorem fragment_law_53 : fragmentLaw .md nativeContent_53 = true := by decide +kernel
theorem geometry_law_53 : validSites o ⟨"fixture",.pages,.md,"title:: [[X]]\n- notes\n"⟩ = true := by decide +kernel
theorem title_parts_law_53 : titlePartsLaw nativeContent_53 = true := by decide +kernel

end CutExamples
end CM
