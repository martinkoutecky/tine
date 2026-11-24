import CutFixtureProof11
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
theorem native_read_36 : nativeRead .md "title:: Old/[[Old]]\n- child\n" = nativeContent_36 := by decide +kernel
theorem fragment_law_36 : fragmentLaw .md nativeContent_36 = true := by decide +kernel
theorem geometry_law_36 : validSites o ⟨"fixture",.pages,.md,"title:: Old/[[Old]]\n- child\n"⟩ = true := by decide +kernel
theorem title_parts_law_36 : titlePartsLaw nativeContent_36 = true := by decide +kernel
theorem native_read_37 : nativeRead .md "title:: old/[[old]]\n- child\n" = nativeContent_37 := by decide +kernel
theorem fragment_law_37 : fragmentLaw .md nativeContent_37 = true := by decide +kernel
theorem geometry_law_37 : validSites o ⟨"fixture",.pages,.md,"title:: old/[[old]]\n- child\n"⟩ = true := by decide +kernel
theorem title_parts_law_37 : titlePartsLaw nativeContent_37 = true := by decide +kernel
theorem native_read_38 : nativeRead .md "alias::\n- target\n" = nativeContent_38 := by decide +kernel
theorem fragment_law_38 : fragmentLaw .md nativeContent_38 = true := by decide +kernel
theorem geometry_law_38 : validSites o ⟨"fixture",.pages,.md,"alias::\n- target\n"⟩ = true := by decide +kernel
theorem title_parts_law_38 : titlePartsLaw nativeContent_38 = true := by decide +kernel

end CutExamples
end CM
