import CutFixtureProof107
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
def own_alias_child_capture_before : Graph := [⟨"pages/A.md",.pages,.md,"alias:: B/sub\n- source\n"⟩,⟨"pages/A%2Fsub.md",.pages,.md,"- child\n"⟩]
theorem own_alias_child_capture_refused : select p dates o own_alias_child_capture_before ⟨"A","B",none,true⟩ = .refuse (.capture) := by decide +kernel


end CutExamples
end CM
