import CutFixtureProof27
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
theorem native_read_84 : nativeRead .md "notes" = nativeContent_84 := by decide +kernel
theorem fragment_law_84 : fragmentLaw .md nativeContent_84 = true := by decide +kernel
theorem geometry_law_84 : validSites o ⟨"fixture",.pages,.md,"notes"⟩ = true := by decide +kernel
theorem title_parts_law_84 : titlePartsLaw nativeContent_84 = true := by decide +kernel
theorem native_read_85 : nativeRead .md "[[A]]" = nativeContent_85 := by decide +kernel
theorem fragment_law_85 : fragmentLaw .md nativeContent_85 = true := by decide +kernel
theorem geometry_law_85 : validSites o ⟨"fixture",.pages,.md,"[[A]]"⟩ = true := by decide +kernel
theorem title_parts_law_85 : titlePartsLaw nativeContent_85 = true := by decide +kernel
theorem native_read_86 : nativeRead .md "[[B]]" = nativeContent_86 := by decide +kernel
theorem fragment_law_86 : fragmentLaw .md nativeContent_86 = true := by decide +kernel
theorem geometry_law_86 : validSites o ⟨"fixture",.pages,.md,"[[B]]"⟩ = true := by decide +kernel
theorem title_parts_law_86 : titlePartsLaw nativeContent_86 = true := by decide +kernel

end CutExamples
end CM
