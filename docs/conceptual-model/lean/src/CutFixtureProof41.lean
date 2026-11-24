import CutFixtureProof40
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
theorem native_read_123 : nativeRead .md "child" = nativeContent_123 := by decide +kernel
theorem fragment_law_123 : fragmentLaw .md nativeContent_123 = true := by decide +kernel
theorem geometry_law_123 : validSites o ⟨"fixture",.pages,.md,"child"⟩ = true := by decide +kernel
theorem title_parts_law_123 : titlePartsLaw nativeContent_123 = true := by decide +kernel
theorem native_read_124 : nativeRead .md "title:: New/[[New]] [[X]]" = nativeContent_124 := by decide +kernel
theorem fragment_law_124 : fragmentLaw .md nativeContent_124 = true := by decide +kernel
theorem geometry_law_124 : validSites o ⟨"fixture",.pages,.md,"title:: New/[[New]] [[X]]"⟩ = true := by decide +kernel
theorem title_parts_law_124 : titlePartsLaw nativeContent_124 = true := by decide +kernel
theorem native_read_125 : nativeRead .md "title:: Old/[[Old]]" = nativeContent_125 := by decide +kernel
theorem fragment_law_125 : fragmentLaw .md nativeContent_125 = true := by decide +kernel
theorem geometry_law_125 : validSites o ⟨"fixture",.pages,.md,"title:: Old/[[Old]]"⟩ = true := by decide +kernel
theorem title_parts_law_125 : titlePartsLaw nativeContent_125 = true := by decide +kernel

end CutExamples
end CM
