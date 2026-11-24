import CutFixtureProof02
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
theorem native_read_9 : nativeRead .org "#+title: A\n* source\n" = nativeContent_9 := by decide +kernel
theorem fragment_law_9 : fragmentLaw .org nativeContent_9 = true := by decide +kernel
theorem geometry_law_9 : validSites o ⟨"fixture",.pages,.org,"#+title: A\n* source\n"⟩ = true := by decide +kernel
theorem title_parts_law_9 : titlePartsLaw nativeContent_9 = true := by decide +kernel
theorem native_read_10 : nativeRead .org "#+title: B\n* target\n" = nativeContent_10 := by decide +kernel
theorem fragment_law_10 : fragmentLaw .org nativeContent_10 = true := by decide +kernel
theorem geometry_law_10 : validSites o ⟨"fixture",.pages,.org,"#+title: B\n* target\n"⟩ = true := by decide +kernel
theorem title_parts_law_10 : titlePartsLaw nativeContent_10 = true := by decide +kernel
theorem native_read_11 : nativeRead .org "#+title: B\n* target\n* \n#+BEGIN_QUOTE\n#+title: B\n#+END_QUOTE\n* source\n" = nativeContent_11 := by decide +kernel
theorem fragment_law_11 : fragmentLaw .org nativeContent_11 = true := by decide +kernel
theorem geometry_law_11 : validSites o ⟨"fixture",.pages,.org,"#+title: B\n* target\n* \n#+BEGIN_QUOTE\n#+title: B\n#+END_QUOTE\n* source\n"⟩ = true := by decide +kernel
theorem title_parts_law_11 : titlePartsLaw nativeContent_11 = true := by decide +kernel

end CutExamples
end CM
