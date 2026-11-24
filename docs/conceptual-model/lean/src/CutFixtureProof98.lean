import CutFixtureProof97
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
def unchanged_child_contested_before : Graph := [⟨"pages/A.md",.pages,.md,"- notes\n"⟩,⟨"pages/P.md",.pages,.md,"title:: A/x\n- p\n"⟩,⟨"pages/Q.md",.pages,.md,"title:: A/x\n- q\n"⟩]
theorem unchanged_child_contested_refused : select p dates o unchanged_child_contested_before ⟨"A","A",none,true⟩ = .refuse (.contested) := by decide +kernel


end CutExamples
end CM
