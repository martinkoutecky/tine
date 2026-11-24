import CutFixtureProof56
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
theorem native_read_171 : nativeRead .md "Old:: source\n- source\n" = nativeContent_171 := by decide +kernel
theorem fragment_law_171 : fragmentLaw .md nativeContent_171 = true := by decide +kernel
theorem geometry_law_171 : validSites o ⟨"fixture",.pages,.md,"Old:: source\n- source\n"⟩ = true := by decide +kernel
theorem title_parts_law_171 : titlePartsLaw nativeContent_171 = true := by decide +kernel
theorem native_read_172 : nativeRead .md "two-words:: target\n- target\n" = nativeContent_172 := by decide +kernel
theorem fragment_law_172 : fragmentLaw .md nativeContent_172 = true := by decide +kernel
theorem geometry_law_172 : validSites o ⟨"fixture",.pages,.md,"two-words:: target\n- target\n"⟩ = true := by decide +kernel
theorem title_parts_law_172 : titlePartsLaw nativeContent_172 = true := by decide +kernel
theorem native_read_173 : nativeRead .md "two-words:: target\n- target\n- \n  two-words:: source\n- source\n" = nativeContent_173 := by decide +kernel
theorem fragment_law_173 : fragmentLaw .md nativeContent_173 = true := by decide +kernel
theorem geometry_law_173 : validSites o ⟨"fixture",.pages,.md,"two-words:: target\n- target\n- \n  two-words:: source\n- source\n"⟩ = true := by decide +kernel
theorem title_parts_law_173 : titlePartsLaw nativeContent_173 = true := by decide +kernel

end CutExamples
end CM
