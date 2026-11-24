import CutFixtureProof35
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
theorem native_read_108 : nativeRead .md "Old:: value" = nativeContent_108 := by decide +kernel
theorem fragment_law_108 : fragmentLaw .md nativeContent_108 = true := by decide +kernel
theorem geometry_law_108 : validSites o ⟨"fixture",.pages,.md,"Old:: value"⟩ = true := by decide +kernel
theorem title_parts_law_108 : titlePartsLaw nativeContent_108 = true := by decide +kernel
theorem native_read_109 : nativeRead .md "Old:: another" = nativeContent_109 := by decide +kernel
theorem fragment_law_109 : fragmentLaw .md nativeContent_109 = true := by decide +kernel
theorem geometry_law_109 : validSites o ⟨"fixture",.pages,.md,"Old:: another"⟩ = true := by decide +kernel
theorem title_parts_law_109 : titlePartsLaw nativeContent_109 = true := by decide +kernel
theorem native_read_110 : nativeRead .md "two-words:: value" = nativeContent_110 := by decide +kernel
theorem fragment_law_110 : fragmentLaw .md nativeContent_110 = true := by decide +kernel
theorem geometry_law_110 : validSites o ⟨"fixture",.pages,.md,"two-words:: value"⟩ = true := by decide +kernel
theorem title_parts_law_110 : titlePartsLaw nativeContent_110 = true := by decide +kernel

end CutExamples
end CM
