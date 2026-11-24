import CutFixtureProof41
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
theorem native_read_126 : nativeRead .md "title:: old/[[old]]" = nativeContent_126 := by decide +kernel
theorem fragment_law_126 : fragmentLaw .md nativeContent_126 = true := by decide +kernel
theorem geometry_law_126 : validSites o ⟨"fixture",.pages,.md,"title:: old/[[old]]"⟩ = true := by decide +kernel
theorem title_parts_law_126 : titlePartsLaw nativeContent_126 = true := by decide +kernel
theorem native_read_127 : nativeRead .md "alias::" = nativeContent_127 := by decide +kernel
theorem fragment_law_127 : fragmentLaw .md nativeContent_127 = true := by decide +kernel
theorem geometry_law_127 : validSites o ⟨"fixture",.pages,.md,"alias::"⟩ = true := by decide +kernel
theorem title_parts_law_127 : titlePartsLaw nativeContent_127 = true := by decide +kernel
theorem native_read_128 : nativeRead .md "source prose\n\n" = nativeContent_128 := by decide +kernel
theorem fragment_law_128 : fragmentLaw .md nativeContent_128 = true := by decide +kernel
theorem geometry_law_128 : validSites o ⟨"fixture",.pages,.md,"source prose\n\n"⟩ = true := by decide +kernel
theorem title_parts_law_128 : titlePartsLaw nativeContent_128 = true := by decide +kernel

end CutExamples
end CM
