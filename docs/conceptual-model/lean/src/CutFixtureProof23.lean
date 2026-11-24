import CutFixtureProof22
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
theorem native_read_69 : nativeRead .md "alias:: Y\n- target\n" = nativeContent_69 := by decide +kernel
theorem fragment_law_69 : fragmentLaw .md nativeContent_69 = true := by decide +kernel
theorem geometry_law_69 : validSites o ⟨"fixture",.pages,.md,"alias:: Y\n- target\n"⟩ = true := by decide +kernel
theorem title_parts_law_69 : titlePartsLaw nativeContent_69 = true := by decide +kernel
theorem native_read_70 : nativeRead .md "alias:: Y\nalias:: X\n- target\n- source\n" = nativeContent_70 := by decide +kernel
theorem fragment_law_70 : fragmentLaw .md nativeContent_70 = true := by decide +kernel
theorem geometry_law_70 : validSites o ⟨"fixture",.pages,.md,"alias:: Y\nalias:: X\n- target\n- source\n"⟩ = true := by decide +kernel
theorem title_parts_law_70 : titlePartsLaw nativeContent_70 = true := by decide +kernel
theorem native_read_71 : nativeRead .md "alias:: B/sub\n- source\n" = nativeContent_71 := by decide +kernel
theorem fragment_law_71 : fragmentLaw .md nativeContent_71 = true := by decide +kernel
theorem geometry_law_71 : validSites o ⟨"fixture",.pages,.md,"alias:: B/sub\n- source\n"⟩ = true := by decide +kernel
theorem title_parts_law_71 : titlePartsLaw nativeContent_71 = true := by decide +kernel

end CutExamples
end CM
