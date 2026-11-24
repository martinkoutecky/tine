import CutFixtureProof33
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
theorem native_read_102 : nativeRead .org "h :Old:" = nativeContent_102 := by decide +kernel
theorem fragment_law_102 : fragmentLaw .org nativeContent_102 = true := by decide +kernel
theorem geometry_law_102 : validSites o ⟨"fixture",.pages,.org,"h :Old:"⟩ = true := by decide +kernel
theorem title_parts_law_102 : titlePartsLaw nativeContent_102 = true := by decide +kernel
theorem native_read_103 : nativeRead .org "<<<<<<< a\n=======\n>>>>>>> b\n" = nativeContent_103 := by decide +kernel
theorem fragment_law_103 : fragmentLaw .org nativeContent_103 = true := by decide +kernel
theorem geometry_law_103 : validSites o ⟨"fixture",.pages,.org,"<<<<<<< a\n=======\n>>>>>>> b\n"⟩ = true := by decide +kernel
theorem title_parts_law_103 : titlePartsLaw nativeContent_103 = true := by decide +kernel
theorem native_read_104 : nativeRead .md "#Old" = nativeContent_104 := by decide +kernel
theorem fragment_law_104 : fragmentLaw .md nativeContent_104 = true := by decide +kernel
theorem geometry_law_104 : validSites o ⟨"fixture",.pages,.md,"#Old"⟩ = true := by decide +kernel
theorem title_parts_law_104 : titlePartsLaw nativeContent_104 = true := by decide +kernel

end CutExamples
end CM
