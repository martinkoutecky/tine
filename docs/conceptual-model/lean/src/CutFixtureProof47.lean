import CutFixtureProof46
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
theorem native_read_141 : nativeRead .md "day" = nativeContent_141 := by decide +kernel
theorem fragment_law_141 : fragmentLaw .md nativeContent_141 = true := by decide +kernel
theorem geometry_law_141 : validSites o ⟨"fixture",.pages,.md,"day"⟩ = true := by decide +kernel
theorem title_parts_law_141 : titlePartsLaw nativeContent_141 = true := by decide +kernel
theorem native_read_142 : nativeRead .md "[[2026/10/26]] [[2026/x]]" = nativeContent_142 := by decide +kernel
theorem fragment_law_142 : fragmentLaw .md nativeContent_142 = true := by decide +kernel
theorem geometry_law_142 : validSites o ⟨"fixture",.pages,.md,"[[2026/10/26]] [[2026/x]]"⟩ = true := by decide +kernel
theorem title_parts_law_142 : titlePartsLaw nativeContent_142 = true := by decide +kernel
theorem native_read_143 : nativeRead .md "[[2026/10/26]] [[B/x]]" = nativeContent_143 := by decide +kernel
theorem fragment_law_143 : fragmentLaw .md nativeContent_143 = true := by decide +kernel
theorem geometry_law_143 : validSites o ⟨"fixture",.pages,.md,"[[2026/10/26]] [[B/x]]"⟩ = true := by decide +kernel
theorem title_parts_law_143 : titlePartsLaw nativeContent_143 = true := by decide +kernel

end CutExamples
end CM
