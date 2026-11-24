import CutFixtureProof79
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
def source_preamble_before : Graph := [⟨"pages/A.md",.pages,.md,"source prose\n\n- source\n"⟩,⟨"pages/B.md",.pages,.md,"target prose\n\n- target\n"⟩]
def source_preamble_after : Graph := [⟨"pages/B.md",.pages,.md,"- target prose\n  \n  \n- target\n- source prose\n  \n  \n- source\n"⟩]
def source_preamble_engine : Engine := ⟨fun _ => some source_preamble_after⟩
theorem source_preamble_admitted : isDone (Cut.decide p dates o source_preamble_engine source_preamble_before ⟨"A","B",some .confirm,true⟩) = true := by decide +kernel


end CutExamples
end CM
