import CutFixtureProof37
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
theorem native_read_114 : nativeRead .md "\ntitle:: X" = nativeContent_114 := by decide +kernel
theorem fragment_law_114 : fragmentLaw .md nativeContent_114 = true := by decide +kernel
theorem geometry_law_114 : validSites o ⟨"fixture",.pages,.md,"\ntitle:: X"⟩ = true := by decide +kernel
theorem title_parts_law_114 : titlePartsLaw nativeContent_114 = true := by decide +kernel
theorem native_read_115 : nativeRead .md "x:: one" = nativeContent_115 := by decide +kernel
theorem fragment_law_115 : fragmentLaw .md nativeContent_115 = true := by decide +kernel
theorem geometry_law_115 : validSites o ⟨"fixture",.pages,.md,"x:: one"⟩ = true := by decide +kernel
theorem title_parts_law_115 : titlePartsLaw nativeContent_115 = true := by decide +kernel
theorem native_read_116 : nativeRead .md "x:: two" = nativeContent_116 := by decide +kernel
theorem fragment_law_116 : fragmentLaw .md nativeContent_116 = true := by decide +kernel
theorem geometry_law_116 : validSites o ⟨"fixture",.pages,.md,"x:: two"⟩ = true := by decide +kernel
theorem title_parts_law_116 : titlePartsLaw nativeContent_116 = true := by decide +kernel

end CutExamples
end CM
