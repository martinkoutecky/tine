import CutFixtureProof03
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
theorem native_read_12 : nativeRead .md "<<<<<<< a\n=======\n>>>>>>> b\n- [[Old]]\n" = nativeContent_12 := by decide +kernel
theorem fragment_law_12 : fragmentLaw .md nativeContent_12 = true := by decide +kernel
theorem geometry_law_12 : validSites o ⟨"fixture",.pages,.md,"<<<<<<< a\n=======\n>>>>>>> b\n- [[Old]]\n"⟩ = true := by decide +kernel
theorem title_parts_law_12 : titlePartsLaw nativeContent_12 = true := by decide +kernel
theorem native_read_13 : nativeRead .org "* h :Old:\n" = nativeContent_13 := by decide +kernel
theorem fragment_law_13 : fragmentLaw .org nativeContent_13 = true := by decide +kernel
theorem geometry_law_13 : validSites o ⟨"fixture",.pages,.org,"* h :Old:\n"⟩ = true := by decide +kernel
theorem title_parts_law_13 : titlePartsLaw nativeContent_13 = true := by decide +kernel
theorem native_read_14 : nativeRead .org "<<<<<<< a\n=======\n>>>>>>> b\n* h :Old:\n" = nativeContent_14 := by decide +kernel
theorem fragment_law_14 : fragmentLaw .org nativeContent_14 = true := by decide +kernel
theorem geometry_law_14 : validSites o ⟨"fixture",.pages,.org,"<<<<<<< a\n=======\n>>>>>>> b\n* h :Old:\n"⟩ = true := by decide +kernel
theorem title_parts_law_14 : titlePartsLaw nativeContent_14 = true := by decide +kernel

end CutExamples
end CM
