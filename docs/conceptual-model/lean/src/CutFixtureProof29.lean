import CutFixtureProof28
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
theorem native_read_87 : nativeRead .md "source" = nativeContent_87 := by decide +kernel
theorem fragment_law_87 : fragmentLaw .md nativeContent_87 = true := by decide +kernel
theorem geometry_law_87 : validSites o ⟨"fixture",.pages,.md,"source"⟩ = true := by decide +kernel
theorem title_parts_law_87 : titlePartsLaw nativeContent_87 = true := by decide +kernel
theorem native_read_88 : nativeRead .md "target" = nativeContent_88 := by decide +kernel
theorem fragment_law_88 : fragmentLaw .md nativeContent_88 = true := by decide +kernel
theorem geometry_law_88 : validSites o ⟨"fixture",.pages,.md,"target"⟩ = true := by decide +kernel
theorem title_parts_law_88 : titlePartsLaw nativeContent_88 = true := by decide +kernel
theorem native_read_89 : nativeRead .md "clash:: source" = nativeContent_89 := by decide +kernel
theorem fragment_law_89 : fragmentLaw .md nativeContent_89 = true := by decide +kernel
theorem geometry_law_89 : validSites o ⟨"fixture",.pages,.md,"clash:: source"⟩ = true := by decide +kernel
theorem title_parts_law_89 : titlePartsLaw nativeContent_89 = true := by decide +kernel

end CutExamples
end CM
