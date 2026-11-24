import CutFixtureProof26
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
theorem native_read_81 : nativeRead .md "key:: target\n- target\n- \n  key:: [[X]]\n- source\n" = nativeContent_81 := by decide +kernel
theorem fragment_law_81 : fragmentLaw .md nativeContent_81 = true := by decide +kernel
theorem geometry_law_81 : validSites o ⟨"fixture",.pages,.md,"key:: target\n- target\n- \n  key:: [[X]]\n- source\n"⟩ = true := by decide +kernel
theorem title_parts_law_81 : titlePartsLaw nativeContent_81 = true := by decide +kernel
theorem native_read_82 : nativeRead .md "key:: target\n- target\n- \n  key:: X\n- source\n" = nativeContent_82 := by decide +kernel
theorem fragment_law_82 : fragmentLaw .md nativeContent_82 = true := by decide +kernel
theorem geometry_law_82 : validSites o ⟨"fixture",.pages,.md,"key:: target\n- target\n- \n  key:: X\n- source\n"⟩ = true := by decide +kernel
theorem title_parts_law_82 : titlePartsLaw nativeContent_82 = true := by decide +kernel
theorem native_read_83 : nativeRead .org "* target\n* \n#+BEGIN_QUOTE\n#+key: [[X]]\n#+END_QUOTE\n* source\n" = nativeContent_83 := by decide +kernel
theorem fragment_law_83 : fragmentLaw .org nativeContent_83 = true := by decide +kernel
theorem geometry_law_83 : validSites o ⟨"fixture",.pages,.org,"* target\n* \n#+BEGIN_QUOTE\n#+key: [[X]]\n#+END_QUOTE\n* source\n"⟩ = true := by decide +kernel
theorem title_parts_law_83 : titlePartsLaw nativeContent_83 = true := by decide +kernel

end CutExamples
end CM
