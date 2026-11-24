import CutFixtureProof54
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
theorem native_read_165 : nativeRead .md "icon:: star\npublic:: true\n- target\n- source\n" = nativeContent_165 := by decide +kernel
theorem fragment_law_165 : fragmentLaw .md nativeContent_165 = true := by decide +kernel
theorem geometry_law_165 : validSites o ⟨"fixture",.pages,.md,"icon:: star\npublic:: true\n- target\n- source\n"⟩ = true := by decide +kernel
theorem title_parts_law_165 : titlePartsLaw nativeContent_165 = true := by decide +kernel
theorem native_read_166 : nativeRead .md "icon:: star\n- source\n" = nativeContent_166 := by decide +kernel
theorem fragment_law_166 : fragmentLaw .md nativeContent_166 = true := by decide +kernel
theorem geometry_law_166 : validSites o ⟨"fixture",.pages,.md,"icon:: star\n- source\n"⟩ = true := by decide +kernel
theorem title_parts_law_166 : titlePartsLaw nativeContent_166 = true := by decide +kernel
theorem native_read_167 : nativeRead .md "icon:: moon\n- target\n" = nativeContent_167 := by decide +kernel
theorem fragment_law_167 : fragmentLaw .md nativeContent_167 = true := by decide +kernel
theorem geometry_law_167 : validSites o ⟨"fixture",.pages,.md,"icon:: moon\n- target\n"⟩ = true := by decide +kernel
theorem title_parts_law_167 : titlePartsLaw nativeContent_167 = true := by decide +kernel

end CutExamples
end CM
