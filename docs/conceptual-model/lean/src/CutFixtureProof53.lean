import CutFixtureProof52
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
theorem native_read_159 : nativeRead .md "key:: [[X]]" = nativeContent_159 := by decide +kernel
theorem fragment_law_159 : fragmentLaw .md nativeContent_159 = true := by decide +kernel
theorem geometry_law_159 : validSites o ⟨"fixture",.pages,.md,"key:: [[X]]"⟩ = true := by decide +kernel
theorem title_parts_law_159 : titlePartsLaw nativeContent_159 = true := by decide +kernel
theorem native_read_160 : nativeRead .md "key:: target" = nativeContent_160 := by decide +kernel
theorem fragment_law_160 : fragmentLaw .md nativeContent_160 = true := by decide +kernel
theorem geometry_law_160 : validSites o ⟨"fixture",.pages,.md,"key:: target"⟩ = true := by decide +kernel
theorem title_parts_law_160 : titlePartsLaw nativeContent_160 = true := by decide +kernel
theorem native_read_161 : nativeRead .md "\nkey:: [[X]]" = nativeContent_161 := by decide +kernel
theorem fragment_law_161 : fragmentLaw .md nativeContent_161 = true := by decide +kernel
theorem geometry_law_161 : validSites o ⟨"fixture",.pages,.md,"\nkey:: [[X]]"⟩ = true := by decide +kernel
theorem title_parts_law_161 : titlePartsLaw nativeContent_161 = true := by decide +kernel

end CutExamples
end CM
