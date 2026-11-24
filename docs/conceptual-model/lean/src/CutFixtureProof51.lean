import CutFixtureProof50
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
theorem native_read_153 : nativeRead .org "#+key: target" = nativeContent_153 := by decide +kernel
theorem fragment_law_153 : fragmentLaw .org nativeContent_153 = true := by decide +kernel
theorem geometry_law_153 : validSites o ⟨"fixture",.pages,.org,"#+key: target"⟩ = true := by decide +kernel
theorem title_parts_law_153 : titlePartsLaw nativeContent_153 = true := by decide +kernel
theorem native_read_154 : nativeRead .md "j:: J" = nativeContent_154 := by decide +kernel
theorem fragment_law_154 : fragmentLaw .md nativeContent_154 = true := by decide +kernel
theorem geometry_law_154 : validSites o ⟨"fixture",.pages,.md,"j:: J"⟩ = true := by decide +kernel
theorem title_parts_law_154 : titlePartsLaw nativeContent_154 = true := by decide +kernel
theorem native_read_155 : nativeRead .md "k:: K" = nativeContent_155 := by decide +kernel
theorem fragment_law_155 : fragmentLaw .md nativeContent_155 = true := by decide +kernel
theorem geometry_law_155 : validSites o ⟨"fixture",.pages,.md,"k:: K"⟩ = true := by decide +kernel
theorem title_parts_law_155 : titlePartsLaw nativeContent_155 = true := by decide +kernel

end CutExamples
end CM
