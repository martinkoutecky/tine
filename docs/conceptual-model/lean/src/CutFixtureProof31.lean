import CutFixtureProof30
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
theorem native_read_93 : nativeRead .md "t" = nativeContent_93 := by decide +kernel
theorem fragment_law_93 : fragmentLaw .md nativeContent_93 = true := by decide +kernel
theorem geometry_law_93 : validSites o ⟨"fixture",.pages,.md,"t"⟩ = true := by decide +kernel
theorem title_parts_law_93 : titlePartsLaw nativeContent_93 = true := by decide +kernel
theorem native_read_94 : nativeRead .md "\nclash:: source" = nativeContent_94 := by decide +kernel
theorem fragment_law_94 : fragmentLaw .md nativeContent_94 = true := by decide +kernel
theorem geometry_law_94 : validSites o ⟨"fixture",.pages,.md,"\nclash:: source"⟩ = true := by decide +kernel
theorem title_parts_law_94 : titlePartsLaw nativeContent_94 = true := by decide +kernel
theorem native_read_95 : nativeRead .org "#+title: A" = nativeContent_95 := by decide +kernel
theorem fragment_law_95 : fragmentLaw .org nativeContent_95 = true := by decide +kernel
theorem geometry_law_95 : validSites o ⟨"fixture",.pages,.org,"#+title: A"⟩ = true := by decide +kernel
theorem title_parts_law_95 : titlePartsLaw nativeContent_95 = true := by decide +kernel

end CutExamples
end CM
