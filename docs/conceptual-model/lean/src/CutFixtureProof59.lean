import CutFixtureProof58
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
theorem native_read_177 : nativeRead .md "\nicon:: star" = nativeContent_177 := by decide +kernel
theorem fragment_law_177 : fragmentLaw .md nativeContent_177 = true := by decide +kernel
theorem geometry_law_177 : validSites o ⟨"fixture",.pages,.md,"\nicon:: star"⟩ = true := by decide +kernel
theorem title_parts_law_177 : titlePartsLaw nativeContent_177 = true := by decide +kernel
theorem native_read_178 : nativeRead .md "Old:: source" = nativeContent_178 := by decide +kernel
theorem fragment_law_178 : fragmentLaw .md nativeContent_178 = true := by decide +kernel
theorem geometry_law_178 : validSites o ⟨"fixture",.pages,.md,"Old:: source"⟩ = true := by decide +kernel
theorem title_parts_law_178 : titlePartsLaw nativeContent_178 = true := by decide +kernel
theorem native_read_179 : nativeRead .md "two-words:: target" = nativeContent_179 := by decide +kernel
theorem fragment_law_179 : fragmentLaw .md nativeContent_179 = true := by decide +kernel
theorem geometry_law_179 : validSites o ⟨"fixture",.pages,.md,"two-words:: target"⟩ = true := by decide +kernel
theorem title_parts_law_179 : titlePartsLaw nativeContent_179 = true := by decide +kernel

end CutExamples
end CM
