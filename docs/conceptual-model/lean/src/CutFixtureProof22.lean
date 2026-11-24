import CutFixtureProof21
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
theorem native_read_66 : nativeRead .md "- child\n" = nativeContent_66 := by decide +kernel
theorem fragment_law_66 : fragmentLaw .md nativeContent_66 = true := by decide +kernel
theorem geometry_law_66 : validSites o ⟨"fixture",.pages,.md,"- child\n"⟩ = true := by decide +kernel
theorem title_parts_law_66 : titlePartsLaw nativeContent_66 = true := by decide +kernel
theorem native_read_67 : nativeRead .md "<<<<<<< a\n=======\n>>>>>>> b\n- source\n" = nativeContent_67 := by decide +kernel
theorem fragment_law_67 : fragmentLaw .md nativeContent_67 = true := by decide +kernel
theorem geometry_law_67 : validSites o ⟨"fixture",.pages,.md,"<<<<<<< a\n=======\n>>>>>>> b\n- source\n"⟩ = true := by decide +kernel
theorem title_parts_law_67 : titlePartsLaw nativeContent_67 = true := by decide +kernel
theorem native_read_68 : nativeRead .md "<<<<<<< a\n=======\n>>>>>>> b\n- target\n" = nativeContent_68 := by decide +kernel
theorem fragment_law_68 : fragmentLaw .md nativeContent_68 = true := by decide +kernel
theorem geometry_law_68 : validSites o ⟨"fixture",.pages,.md,"<<<<<<< a\n=======\n>>>>>>> b\n- target\n"⟩ = true := by decide +kernel
theorem title_parts_law_68 : titlePartsLaw nativeContent_68 = true := by decide +kernel

end CutExamples
end CM
