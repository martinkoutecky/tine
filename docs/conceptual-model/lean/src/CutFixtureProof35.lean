import CutFixtureProof34
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
theorem native_read_105 : nativeRead .md "#[[Two Words]]" = nativeContent_105 := by decide +kernel
theorem fragment_law_105 : fragmentLaw .md nativeContent_105 = true := by decide +kernel
theorem geometry_law_105 : validSites o ⟨"fixture",.pages,.md,"#[[Two Words]]"⟩ = true := by decide +kernel
theorem title_parts_law_105 : titlePartsLaw nativeContent_105 = true := by decide +kernel
theorem native_read_106 : nativeRead .md "alias:: C#" = nativeContent_106 := by decide +kernel
theorem fragment_law_106 : fragmentLaw .md nativeContent_106 = true := by decide +kernel
theorem geometry_law_106 : validSites o ⟨"fixture",.pages,.md,"alias:: C#"⟩ = true := by decide +kernel
theorem title_parts_law_106 : titlePartsLaw nativeContent_106 = true := by decide +kernel
theorem native_read_107 : nativeRead .md "alias:: C\\#" = nativeContent_107 := by decide +kernel
theorem fragment_law_107 : fragmentLaw .md nativeContent_107 = true := by decide +kernel
theorem geometry_law_107 : validSites o ⟨"fixture",.pages,.md,"alias:: C\\#"⟩ = true := by decide +kernel
theorem title_parts_law_107 : titlePartsLaw nativeContent_107 = true := by decide +kernel

end CutExamples
end CM
