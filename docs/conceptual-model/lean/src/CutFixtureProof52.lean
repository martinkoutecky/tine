import CutFixtureProof51
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
theorem native_read_156 : nativeRead .md "x:: X" = nativeContent_156 := by decide +kernel
theorem fragment_law_156 : fragmentLaw .md nativeContent_156 = true := by decide +kernel
theorem geometry_law_156 : validSites o ⟨"fixture",.pages,.md,"x:: X"⟩ = true := by decide +kernel
theorem title_parts_law_156 : titlePartsLaw nativeContent_156 = true := by decide +kernel
theorem native_read_157 : nativeRead .md "y:: Y" = nativeContent_157 := by decide +kernel
theorem fragment_law_157 : fragmentLaw .md nativeContent_157 = true := by decide +kernel
theorem geometry_law_157 : validSites o ⟨"fixture",.pages,.md,"y:: Y"⟩ = true := by decide +kernel
theorem title_parts_law_157 : titlePartsLaw nativeContent_157 = true := by decide +kernel
theorem native_read_158 : nativeRead .org "\n#+BEGIN_QUOTE\n#+key: X\n#+END_QUOTE" = nativeContent_158 := by decide +kernel
theorem fragment_law_158 : fragmentLaw .org nativeContent_158 = true := by decide +kernel
theorem geometry_law_158 : validSites o ⟨"fixture",.pages,.org,"\n#+BEGIN_QUOTE\n#+key: X\n#+END_QUOTE"⟩ = true := by decide +kernel
theorem title_parts_law_158 : titlePartsLaw nativeContent_158 = true := by decide +kernel

end CutExamples
end CM
