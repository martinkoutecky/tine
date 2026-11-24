import CutFixtureProof102
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
def alias_child_capture_before : Graph := [⟨"pages/A.md",.pages,.md,"- notes\n"⟩,⟨"pages/C.md",.pages,.md,"alias:: A/sub\n- owner\n"⟩,⟨"pages/B%2Fsub.md",.pages,.md,"- bs\n"⟩]
theorem alias_child_capture_refused : select p dates o alias_child_capture_before ⟨"A","B",none,true⟩ = .refuse (.capture) := by decide +kernel


end CutExamples
end CM
