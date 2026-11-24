import CutFixtureProof17
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
theorem native_read_54 : nativeRead .md "title:: 2026/10/26\n- day\n" = nativeContent_54 := by decide +kernel
theorem fragment_law_54 : fragmentLaw .md nativeContent_54 = true := by decide +kernel
theorem geometry_law_54 : validSites o ⟨"fixture",.pages,.md,"title:: 2026/10/26\n- day\n"⟩ = true := by decide +kernel
theorem title_parts_law_54 : titlePartsLaw nativeContent_54 = true := by decide +kernel
theorem native_read_55 : nativeRead .md "- [[2026/10/26]] [[2026/x]]\n" = nativeContent_55 := by decide +kernel
theorem fragment_law_55 : fragmentLaw .md nativeContent_55 = true := by decide +kernel
theorem geometry_law_55 : validSites o ⟨"fixture",.pages,.md,"- [[2026/10/26]] [[2026/x]]\n"⟩ = true := by decide +kernel
theorem title_parts_law_55 : titlePartsLaw nativeContent_55 = true := by decide +kernel
theorem native_read_56 : nativeRead .md "- [[2026/10/26]] [[B/x]]\n" = nativeContent_56 := by decide +kernel
theorem fragment_law_56 : fragmentLaw .md nativeContent_56 = true := by decide +kernel
theorem geometry_law_56 : validSites o ⟨"fixture",.pages,.md,"- [[2026/10/26]] [[B/x]]\n"⟩ = true := by decide +kernel
theorem title_parts_law_56 : titlePartsLaw nativeContent_56 = true := by decide +kernel

end CutExamples
end CM
