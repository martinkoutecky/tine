import CutFixtureProof43
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
theorem native_read_132 : nativeRead .md "\ntitle:: B" = nativeContent_132 := by decide +kernel
theorem fragment_law_132 : fragmentLaw .md nativeContent_132 = true := by decide +kernel
theorem geometry_law_132 : validSites o ⟨"fixture",.pages,.md,"\ntitle:: B"⟩ = true := by decide +kernel
theorem title_parts_law_132 : titlePartsLaw nativeContent_132 = true := by decide +kernel
theorem native_read_133 : nativeRead .md "p" = nativeContent_133 := by decide +kernel
theorem fragment_law_133 : fragmentLaw .md nativeContent_133 = true := by decide +kernel
theorem geometry_law_133 : validSites o ⟨"fixture",.pages,.md,"p"⟩ = true := by decide +kernel
theorem title_parts_law_133 : titlePartsLaw nativeContent_133 = true := by decide +kernel
theorem native_read_134 : nativeRead .md "q" = nativeContent_134 := by decide +kernel
theorem fragment_law_134 : fragmentLaw .md nativeContent_134 = true := by decide +kernel
theorem geometry_law_134 : validSites o ⟨"fixture",.pages,.md,"q"⟩ = true := by decide +kernel
theorem title_parts_law_134 : titlePartsLaw nativeContent_134 = true := by decide +kernel

end CutExamples
end CM
