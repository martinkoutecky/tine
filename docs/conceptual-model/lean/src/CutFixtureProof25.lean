import CutFixtureProof24
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
theorem native_read_75 : nativeRead .md "x:: X\ny:: Y\n- target\n" = nativeContent_75 := by decide +kernel
theorem fragment_law_75 : fragmentLaw .md nativeContent_75 = true := by decide +kernel
theorem geometry_law_75 : validSites o ⟨"fixture",.pages,.md,"x:: X\ny:: Y\n- target\n"⟩ = true := by decide +kernel
theorem title_parts_law_75 : titlePartsLaw nativeContent_75 = true := by decide +kernel
theorem native_read_76 : nativeRead .md "k:: K\nx:: X\nj:: J\ny:: Y\n- target\n- source\n" = nativeContent_76 := by decide +kernel
theorem fragment_law_76 : fragmentLaw .md nativeContent_76 = true := by decide +kernel
theorem geometry_law_76 : validSites o ⟨"fixture",.pages,.md,"k:: K\nx:: X\nj:: J\ny:: Y\n- target\n- source\n"⟩ = true := by decide +kernel
theorem title_parts_law_76 : titlePartsLaw nativeContent_76 = true := by decide +kernel
theorem native_read_77 : nativeRead .md "k:: K\nx:: X\ny:: Y\n- target\n- source\n" = nativeContent_77 := by decide +kernel
theorem fragment_law_77 : fragmentLaw .md nativeContent_77 = true := by decide +kernel
theorem geometry_law_77 : validSites o ⟨"fixture",.pages,.md,"k:: K\nx:: X\ny:: Y\n- target\n- source\n"⟩ = true := by decide +kernel
theorem title_parts_law_77 : titlePartsLaw nativeContent_77 = true := by decide +kernel

end CutExamples
end CM
