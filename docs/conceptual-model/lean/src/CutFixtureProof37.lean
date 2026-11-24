import CutFixtureProof36
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
theorem native_read_111 : nativeRead .md "two-words:: another" = nativeContent_111 := by decide +kernel
theorem fragment_law_111 : fragmentLaw .md nativeContent_111 = true := by decide +kernel
theorem geometry_law_111 : validSites o ⟨"fixture",.pages,.md,"two-words:: another"⟩ = true := by decide +kernel
theorem title_parts_law_111 : titlePartsLaw nativeContent_111 = true := by decide +kernel
theorem native_read_112 : nativeRead .md "alias:: X" = nativeContent_112 := by decide +kernel
theorem fragment_law_112 : fragmentLaw .md nativeContent_112 = true := by decide +kernel
theorem geometry_law_112 : validSites o ⟨"fixture",.pages,.md,"alias:: X"⟩ = true := by decide +kernel
theorem title_parts_law_112 : titlePartsLaw nativeContent_112 = true := by decide +kernel
theorem native_read_113 : nativeRead .md "title:: X" = nativeContent_113 := by decide +kernel
theorem fragment_law_113 : fragmentLaw .md nativeContent_113 = true := by decide +kernel
theorem geometry_law_113 : validSites o ⟨"fixture",.pages,.md,"title:: X"⟩ = true := by decide +kernel
theorem title_parts_law_113 : titlePartsLaw nativeContent_113 = true := by decide +kernel

end CutExamples
end CM
