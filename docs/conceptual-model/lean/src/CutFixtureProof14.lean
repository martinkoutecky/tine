import CutFixtureProof13
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
theorem native_read_42 : nativeRead .md "- target prose\n  \n  \n- target\n- source prose\n  \n  \n- source\n" = nativeContent_42 := by decide +kernel
theorem fragment_law_42 : fragmentLaw .md nativeContent_42 = true := by decide +kernel
theorem geometry_law_42 : validSites o ⟨"fixture",.pages,.md,"- target prose\n  \n  \n- target\n- source prose\n  \n  \n- source\n"⟩ = true := by decide +kernel
theorem title_parts_law_42 : titlePartsLaw nativeContent_42 = true := by decide +kernel
theorem native_read_43 : nativeRead .md "title:: A\n- alias:: X\n  - kid\n- source\n" = nativeContent_43 := by decide +kernel
theorem fragment_law_43 : fragmentLaw .md nativeContent_43 = true := by decide +kernel
theorem geometry_law_43 : validSites o ⟨"fixture",.pages,.md,"title:: A\n- alias:: X\n  - kid\n- source\n"⟩ = true := by decide +kernel
theorem title_parts_law_43 : titlePartsLaw nativeContent_43 = true := by decide +kernel
theorem native_read_44 : nativeRead .md "alias:: X\n- target\n- \n  title:: B\n  - kid\n- source\n" = nativeContent_44 := by decide +kernel
theorem fragment_law_44 : fragmentLaw .md nativeContent_44 = true := by decide +kernel
theorem geometry_law_44 : validSites o ⟨"fixture",.pages,.md,"alias:: X\n- target\n- \n  title:: B\n  - kid\n- source\n"⟩ = true := by decide +kernel
theorem title_parts_law_44 : titlePartsLaw nativeContent_44 = true := by decide +kernel

end CutExamples
end CM
