import CutFixtureProof09
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
theorem native_read_30 : nativeRead .md "alias:: B\n- owner\n" = nativeContent_30 := by decide +kernel
theorem fragment_law_30 : fragmentLaw .md nativeContent_30 = true := by decide +kernel
theorem geometry_law_30 : validSites o ⟨"fixture",.pages,.md,"alias:: B\n- owner\n"⟩ = true := by decide +kernel
theorem title_parts_law_30 : titlePartsLaw nativeContent_30 = true := by decide +kernel
theorem native_read_31 : nativeRead .md "title:: 2026-11-02\n- target\n" = nativeContent_31 := by decide +kernel
theorem fragment_law_31 : fragmentLaw .md nativeContent_31 = true := by decide +kernel
theorem geometry_law_31 : validSites o ⟨"fixture",.pages,.md,"title:: 2026-11-02\n- target\n"⟩ = true := by decide +kernel
theorem title_parts_law_31 : titlePartsLaw nativeContent_31 = true := by decide +kernel
theorem native_read_32 : nativeRead .md "title:: 2026-11-02\n- target\n- source\n" = nativeContent_32 := by decide +kernel
theorem fragment_law_32 : fragmentLaw .md nativeContent_32 = true := by decide +kernel
theorem geometry_law_32 : validSites o ⟨"fixture",.pages,.md,"title:: 2026-11-02\n- target\n- source\n"⟩ = true := by decide +kernel
theorem title_parts_law_32 : titlePartsLaw nativeContent_32 = true := by decide +kernel

end CutExamples
end CM
