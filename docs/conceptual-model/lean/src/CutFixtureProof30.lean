import CutFixtureProof29
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
theorem native_read_90 : nativeRead .md "join:: value" = nativeContent_90 := by decide +kernel
theorem fragment_law_90 : fragmentLaw .md nativeContent_90 = true := by decide +kernel
theorem geometry_law_90 : validSites o ⟨"fixture",.pages,.md,"join:: value"⟩ = true := by decide +kernel
theorem title_parts_law_90 : titlePartsLaw nativeContent_90 = true := by decide +kernel
theorem native_read_91 : nativeRead .md "s" = nativeContent_91 := by decide +kernel
theorem fragment_law_91 : fragmentLaw .md nativeContent_91 = true := by decide +kernel
theorem geometry_law_91 : validSites o ⟨"fixture",.pages,.md,"s"⟩ = true := by decide +kernel
theorem title_parts_law_91 : titlePartsLaw nativeContent_91 = true := by decide +kernel
theorem native_read_92 : nativeRead .md "clash:: target" = nativeContent_92 := by decide +kernel
theorem fragment_law_92 : fragmentLaw .md nativeContent_92 = true := by decide +kernel
theorem geometry_law_92 : validSites o ⟨"fixture",.pages,.md,"clash:: target"⟩ = true := by decide +kernel
theorem title_parts_law_92 : titlePartsLaw nativeContent_92 = true := by decide +kernel

end CutExamples
end CM
