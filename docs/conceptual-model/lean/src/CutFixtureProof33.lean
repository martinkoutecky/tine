import CutFixtureProof32
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
theorem native_read_99 : nativeRead .org "\n#+BEGIN_QUOTE\n#+title: B\n#+END_QUOTE" = nativeContent_99 := by decide +kernel
theorem fragment_law_99 : fragmentLaw .org nativeContent_99 = true := by decide +kernel
theorem geometry_law_99 : validSites o ⟨"fixture",.pages,.org,"\n#+BEGIN_QUOTE\n#+title: B\n#+END_QUOTE"⟩ = true := by decide +kernel
theorem title_parts_law_99 : titlePartsLaw nativeContent_99 = true := by decide +kernel
theorem native_read_100 : nativeRead .md "<<<<<<< a\n=======\n>>>>>>> b\n" = nativeContent_100 := by decide +kernel
theorem fragment_law_100 : fragmentLaw .md nativeContent_100 = true := by decide +kernel
theorem geometry_law_100 : validSites o ⟨"fixture",.pages,.md,"<<<<<<< a\n=======\n>>>>>>> b\n"⟩ = true := by decide +kernel
theorem title_parts_law_100 : titlePartsLaw nativeContent_100 = true := by decide +kernel
theorem native_read_101 : nativeRead .md "[[Old]]" = nativeContent_101 := by decide +kernel
theorem fragment_law_101 : fragmentLaw .md nativeContent_101 = true := by decide +kernel
theorem geometry_law_101 : validSites o ⟨"fixture",.pages,.md,"[[Old]]"⟩ = true := by decide +kernel
theorem title_parts_law_101 : titlePartsLaw nativeContent_101 = true := by decide +kernel

end CutExamples
end CM
