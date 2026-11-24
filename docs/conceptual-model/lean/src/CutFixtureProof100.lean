import CutFixtureProof99
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
def header_children_no_leftover_before : Graph := [⟨"pages/A.md",.pages,.md,"- alias:: X\n  - kid\n- source\n"⟩,⟨"pages/B.md",.pages,.md,"- target\n"⟩]
theorem header_children_no_leftover_refused : select p dates o header_children_no_leftover_before ⟨"A","B",some .confirm,true⟩ = .refuse (.headerChildren) := by decide +kernel


end CutExamples
end CM
