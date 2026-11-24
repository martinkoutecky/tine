import CutFixtureProof53
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
theorem native_read_162 : nativeRead .md "\nkey:: X" = nativeContent_162 := by decide +kernel
theorem fragment_law_162 : fragmentLaw .md nativeContent_162 = true := by decide +kernel
theorem geometry_law_162 : validSites o ⟨"fixture",.pages,.md,"\nkey:: X"⟩ = true := by decide +kernel
theorem title_parts_law_162 : titlePartsLaw nativeContent_162 = true := by decide +kernel
theorem native_read_163 : nativeRead .org "\n#+BEGIN_QUOTE\n#+key: [[X]]\n#+END_QUOTE" = nativeContent_163 := by decide +kernel
theorem fragment_law_163 : fragmentLaw .org nativeContent_163 = true := by decide +kernel
theorem geometry_law_163 : validSites o ⟨"fixture",.pages,.org,"\n#+BEGIN_QUOTE\n#+key: [[X]]\n#+END_QUOTE"⟩ = true := by decide +kernel
theorem title_parts_law_163 : titlePartsLaw nativeContent_163 = true := by decide +kernel
theorem native_read_164 : nativeRead .md "icon:: star\npublic:: true\n- source\n" = nativeContent_164 := by decide +kernel
theorem fragment_law_164 : fragmentLaw .md nativeContent_164 = true := by decide +kernel
theorem geometry_law_164 : validSites o ⟨"fixture",.pages,.md,"icon:: star\npublic:: true\n- source\n"⟩ = true := by decide +kernel
theorem title_parts_law_164 : titlePartsLaw nativeContent_164 = true := by decide +kernel

end CutExamples
end CM
