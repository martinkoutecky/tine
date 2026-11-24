import CutFixtureProof23
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
theorem native_read_72 : nativeRead .org "#+key: [[X]]\n* source\n" = nativeContent_72 := by decide +kernel
theorem fragment_law_72 : fragmentLaw .org nativeContent_72 = true := by decide +kernel
theorem geometry_law_72 : validSites o ⟨"fixture",.pages,.org,"#+key: [[X]]\n* source\n"⟩ = true := by decide +kernel
theorem title_parts_law_72 : titlePartsLaw nativeContent_72 = true := by decide +kernel
theorem native_read_73 : nativeRead .org "#+key: target\n* target\n" = nativeContent_73 := by decide +kernel
theorem fragment_law_73 : fragmentLaw .org nativeContent_73 = true := by decide +kernel
theorem geometry_law_73 : validSites o ⟨"fixture",.pages,.org,"#+key: target\n* target\n"⟩ = true := by decide +kernel
theorem title_parts_law_73 : titlePartsLaw nativeContent_73 = true := by decide +kernel
theorem native_read_74 : nativeRead .md "j:: J\nk:: K\n- source\n" = nativeContent_74 := by decide +kernel
theorem fragment_law_74 : fragmentLaw .md nativeContent_74 = true := by decide +kernel
theorem geometry_law_74 : validSites o ⟨"fixture",.pages,.md,"j:: J\nk:: K\n- source\n"⟩ = true := by decide +kernel
theorem title_parts_law_74 : titlePartsLaw nativeContent_74 = true := by decide +kernel

end CutExamples
end CM
