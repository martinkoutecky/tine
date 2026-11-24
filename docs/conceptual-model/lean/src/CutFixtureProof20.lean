import CutFixtureProof19
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
theorem native_read_60 : nativeRead .md "- \n- source\n" = nativeContent_60 := by decide +kernel
theorem fragment_law_60 : fragmentLaw .md nativeContent_60 = true := by decide +kernel
theorem geometry_law_60 : validSites o ⟨"fixture",.pages,.md,"- \n- source\n"⟩ = true := by decide +kernel
theorem title_parts_law_60 : titlePartsLaw nativeContent_60 = true := by decide +kernel
theorem native_read_61 : nativeRead .org "* h :New:\n" = nativeContent_61 := by decide +kernel
theorem fragment_law_61 : fragmentLaw .org nativeContent_61 = true := by decide +kernel
theorem geometry_law_61 : validSites o ⟨"fixture",.pages,.org,"* h :New:\n"⟩ = true := by decide +kernel
theorem title_parts_law_61 : titlePartsLaw nativeContent_61 = true := by decide +kernel
theorem native_read_62 : nativeRead .md "- [[A/x]]\n" = nativeContent_62 := by decide +kernel
theorem fragment_law_62 : fragmentLaw .md nativeContent_62 = true := by decide +kernel
theorem geometry_law_62 : validSites o ⟨"fixture",.pages,.md,"- [[A/x]]\n"⟩ = true := by decide +kernel
theorem title_parts_law_62 : titlePartsLaw nativeContent_62 = true := by decide +kernel

end CutExamples
end CM
