import CutFixtureProof80
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
def header_children_leftover_before : Graph := [⟨"pages/A.md",.pages,.md,"title:: A\n- alias:: X\n  - kid\n- source\n"⟩,⟨"pages/B.md",.pages,.md,"- target\n"⟩]
def header_children_leftover_after : Graph := [⟨"pages/B.md",.pages,.md,"alias:: X\n- target\n- \n  title:: B\n  - kid\n- source\n"⟩]
def header_children_leftover_engine : Engine := ⟨fun _ => some header_children_leftover_after⟩
theorem header_children_leftover_admitted : isDone (Cut.decide p dates o header_children_leftover_engine header_children_leftover_before ⟨"A","B",some .confirm,true⟩) = true := by decide +kernel


end CutExamples
end CM
