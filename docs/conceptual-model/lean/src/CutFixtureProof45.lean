import CutFixtureProof44
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
theorem native_read_135 : nativeRead .md "title:: A/x" = nativeContent_135 := by decide +kernel
theorem fragment_law_135 : fragmentLaw .md nativeContent_135 = true := by decide +kernel
theorem geometry_law_135 : validSites o ⟨"fixture",.pages,.md,"title:: A/x"⟩ = true := by decide +kernel
theorem title_parts_law_135 : titlePartsLaw nativeContent_135 = true := by decide +kernel
theorem native_read_136 : nativeRead .md "title:: C" = nativeContent_136 := by decide +kernel
theorem fragment_law_136 : fragmentLaw .md nativeContent_136 = true := by decide +kernel
theorem geometry_law_136 : validSites o ⟨"fixture",.pages,.md,"title:: C"⟩ = true := by decide +kernel
theorem title_parts_law_136 : titlePartsLaw nativeContent_136 = true := by decide +kernel
theorem native_read_137 : nativeRead .md "occupied" = nativeContent_137 := by decide +kernel
theorem fragment_law_137 : fragmentLaw .md nativeContent_137 = true := by decide +kernel
theorem geometry_law_137 : validSites o ⟨"fixture",.pages,.md,"occupied"⟩ = true := by decide +kernel
theorem title_parts_law_137 : titlePartsLaw nativeContent_137 = true := by decide +kernel

end CutExamples
end CM
