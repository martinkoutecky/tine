import CutFixtureProof55
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
theorem native_read_168 : nativeRead .md "icon:: moon\n- target\n- \n  icon:: star\n- source\n" = nativeContent_168 := by decide +kernel
theorem fragment_law_168 : fragmentLaw .md nativeContent_168 = true := by decide +kernel
theorem geometry_law_168 : validSites o ⟨"fixture",.pages,.md,"icon:: moon\n- target\n- \n  icon:: star\n- source\n"⟩ = true := by decide +kernel
theorem title_parts_law_168 : titlePartsLaw nativeContent_168 = true := by decide +kernel
theorem native_read_169 : nativeRead .md "public:: true\n- target\n" = nativeContent_169 := by decide +kernel
theorem fragment_law_169 : fragmentLaw .md nativeContent_169 = true := by decide +kernel
theorem geometry_law_169 : validSites o ⟨"fixture",.pages,.md,"public:: true\n- target\n"⟩ = true := by decide +kernel
theorem title_parts_law_169 : titlePartsLaw nativeContent_169 = true := by decide +kernel
theorem native_read_170 : nativeRead .md "public:: true\nicon:: star\n- target\n- source\n" = nativeContent_170 := by decide +kernel
theorem fragment_law_170 : fragmentLaw .md nativeContent_170 = true := by decide +kernel
theorem geometry_law_170 : validSites o ⟨"fixture",.pages,.md,"public:: true\nicon:: star\n- target\n- source\n"⟩ = true := by decide +kernel
theorem title_parts_law_170 : titlePartsLaw nativeContent_170 = true := by decide +kernel

end CutExamples
end CM
