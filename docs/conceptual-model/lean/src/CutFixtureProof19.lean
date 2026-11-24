import CutFixtureProof18
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
theorem native_read_57 : nativeRead .md "join:: value\n- s\n" = nativeContent_57 := by decide +kernel
theorem fragment_law_57 : fragmentLaw .md nativeContent_57 = true := by decide +kernel
theorem geometry_law_57 : validSites o ⟨"fixture",.pages,.md,"join:: value\n- s\n"⟩ = true := by decide +kernel
theorem title_parts_law_57 : titlePartsLaw nativeContent_57 = true := by decide +kernel
theorem native_read_58 : nativeRead .md "join:: value\nclash:: target\n- t\n- s\n" = nativeContent_58 := by decide +kernel
theorem fragment_law_58 : fragmentLaw .md nativeContent_58 = true := by decide +kernel
theorem geometry_law_58 : validSites o ⟨"fixture",.pages,.md,"join:: value\nclash:: target\n- t\n- s\n"⟩ = true := by decide +kernel
theorem title_parts_law_58 : titlePartsLaw nativeContent_58 = true := by decide +kernel
theorem native_read_59 : nativeRead .md "\n- source\n" = nativeContent_59 := by decide +kernel
theorem fragment_law_59 : fragmentLaw .md nativeContent_59 = true := by decide +kernel
theorem geometry_law_59 : validSites o ⟨"fixture",.pages,.md,"\n- source\n"⟩ = true := by decide +kernel
theorem title_parts_law_59 : titlePartsLaw nativeContent_59 = true := by decide +kernel

end CutExamples
end CM
