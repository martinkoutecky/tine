import CutFixtureProof57
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
theorem native_read_174 : nativeRead .md "icon:: star" = nativeContent_174 := by decide +kernel
theorem fragment_law_174 : fragmentLaw .md nativeContent_174 = true := by decide +kernel
theorem geometry_law_174 : validSites o ⟨"fixture",.pages,.md,"icon:: star"⟩ = true := by decide +kernel
theorem title_parts_law_174 : titlePartsLaw nativeContent_174 = true := by decide +kernel
theorem native_read_175 : nativeRead .md "public:: true" = nativeContent_175 := by decide +kernel
theorem fragment_law_175 : fragmentLaw .md nativeContent_175 = true := by decide +kernel
theorem geometry_law_175 : validSites o ⟨"fixture",.pages,.md,"public:: true"⟩ = true := by decide +kernel
theorem title_parts_law_175 : titlePartsLaw nativeContent_175 = true := by decide +kernel
theorem native_read_176 : nativeRead .md "icon:: moon" = nativeContent_176 := by decide +kernel
theorem fragment_law_176 : fragmentLaw .md nativeContent_176 = true := by decide +kernel
theorem geometry_law_176 : validSites o ⟨"fixture",.pages,.md,"icon:: moon"⟩ = true := by decide +kernel
theorem title_parts_law_176 : titlePartsLaw nativeContent_176 = true := by decide +kernel

end CutExamples
end CM
