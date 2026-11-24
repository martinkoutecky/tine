import CutFixtureProof101
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
def reference_child_capture_before : Graph := [⟨"pages/A.md",.pages,.md,"- notes\n"⟩,⟨"pages/R.md",.pages,.md,"- [[A/x]]\n"⟩,⟨"pages/B%2Fx.md",.pages,.md,"- bx\n"⟩]
theorem reference_child_capture_refused : select p dates o reference_child_capture_before ⟨"A","B",none,true⟩ = .refuse (.capture) := by decide +kernel


end CutExamples
end CM
