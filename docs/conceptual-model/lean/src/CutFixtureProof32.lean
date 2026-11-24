import CutFixtureProof31
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
theorem native_read_96 : nativeRead .org "source" = nativeContent_96 := by decide +kernel
theorem fragment_law_96 : fragmentLaw .org nativeContent_96 = true := by decide +kernel
theorem geometry_law_96 : validSites o ⟨"fixture",.pages,.org,"source"⟩ = true := by decide +kernel
theorem title_parts_law_96 : titlePartsLaw nativeContent_96 = true := by decide +kernel
theorem native_read_97 : nativeRead .org "#+title: B" = nativeContent_97 := by decide +kernel
theorem fragment_law_97 : fragmentLaw .org nativeContent_97 = true := by decide +kernel
theorem geometry_law_97 : validSites o ⟨"fixture",.pages,.org,"#+title: B"⟩ = true := by decide +kernel
theorem title_parts_law_97 : titlePartsLaw nativeContent_97 = true := by decide +kernel
theorem native_read_98 : nativeRead .org "target" = nativeContent_98 := by decide +kernel
theorem fragment_law_98 : fragmentLaw .org nativeContent_98 = true := by decide +kernel
theorem geometry_law_98 : validSites o ⟨"fixture",.pages,.org,"target"⟩ = true := by decide +kernel
theorem title_parts_law_98 : titlePartsLaw nativeContent_98 = true := by decide +kernel

end CutExamples
end CM
