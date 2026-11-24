import CutFixtureProof103
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
def claimed_own_child_before : Graph := [⟨"pages/A.md",.pages,.md,"- notes\n"⟩,⟨"pages/A%2Fsub.md",.pages,.md,"- child\n"⟩]
theorem claimed_own_child_refused : select p dates o claimed_own_child_before ⟨"A","A/sub",none,true⟩ = .refuse (.ownChild) := by decide +kernel


end CutExamples
end CM
