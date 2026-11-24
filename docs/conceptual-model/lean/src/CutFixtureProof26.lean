import CutFixtureProof25
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
theorem native_read_78 : nativeRead .org "#+key: target\n* target\n* \n#+BEGIN_QUOTE\n#+key: X\n#+END_QUOTE\n* source\n" = nativeContent_78 := by decide +kernel
theorem fragment_law_78 : fragmentLaw .org nativeContent_78 = true := by decide +kernel
theorem geometry_law_78 : validSites o ⟨"fixture",.pages,.org,"#+key: target\n* target\n* \n#+BEGIN_QUOTE\n#+key: X\n#+END_QUOTE\n* source\n"⟩ = true := by decide +kernel
theorem title_parts_law_78 : titlePartsLaw nativeContent_78 = true := by decide +kernel
theorem native_read_79 : nativeRead .md "key:: [[X]]\n- source\n" = nativeContent_79 := by decide +kernel
theorem fragment_law_79 : fragmentLaw .md nativeContent_79 = true := by decide +kernel
theorem geometry_law_79 : validSites o ⟨"fixture",.pages,.md,"key:: [[X]]\n- source\n"⟩ = true := by decide +kernel
theorem title_parts_law_79 : titlePartsLaw nativeContent_79 = true := by decide +kernel
theorem native_read_80 : nativeRead .md "key:: target\n- target\n" = nativeContent_80 := by decide +kernel
theorem fragment_law_80 : fragmentLaw .md nativeContent_80 = true := by decide +kernel
theorem geometry_law_80 : validSites o ⟨"fixture",.pages,.md,"key:: target\n- target\n"⟩ = true := by decide +kernel
theorem title_parts_law_80 : titlePartsLaw nativeContent_80 = true := by decide +kernel

end CutExamples
end CM
