import CutFixtureProof01
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
theorem native_read_6 : nativeRead .md "clash:: source\njoin:: value\n- s\n" = nativeContent_6 := by decide +kernel
theorem fragment_law_6 : fragmentLaw .md nativeContent_6 = true := by decide +kernel
theorem geometry_law_6 : validSites o ⟨"fixture",.pages,.md,"clash:: source\njoin:: value\n- s\n"⟩ = true := by decide +kernel
theorem title_parts_law_6 : titlePartsLaw nativeContent_6 = true := by decide +kernel
theorem native_read_7 : nativeRead .md "clash:: target\n- t\n" = nativeContent_7 := by decide +kernel
theorem fragment_law_7 : fragmentLaw .md nativeContent_7 = true := by decide +kernel
theorem geometry_law_7 : validSites o ⟨"fixture",.pages,.md,"clash:: target\n- t\n"⟩ = true := by decide +kernel
theorem title_parts_law_7 : titlePartsLaw nativeContent_7 = true := by decide +kernel
theorem native_read_8 : nativeRead .md "clash:: target\njoin:: value\n- t\n- \n  clash:: source\n- s\n" = nativeContent_8 := by decide +kernel
theorem fragment_law_8 : fragmentLaw .md nativeContent_8 = true := by decide +kernel
theorem geometry_law_8 : validSites o ⟨"fixture",.pages,.md,"clash:: target\njoin:: value\n- t\n- \n  clash:: source\n- s\n"⟩ = true := by decide +kernel
theorem title_parts_law_8 : titlePartsLaw nativeContent_8 = true := by decide +kernel

end CutExamples
end CM
