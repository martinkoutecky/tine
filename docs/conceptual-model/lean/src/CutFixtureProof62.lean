import CutFixtureProof61
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
def plain_merge_before : Graph := [⟨"pages/A.md",.pages,.md,"- source\n"⟩,⟨"pages/B.md",.pages,.md,"- target\n"⟩]
def plain_merge_after : Graph := [⟨"pages/B.md",.pages,.md,"- target\n- source\n"⟩]
def plain_merge_engine : Engine := ⟨fun _ => some plain_merge_after⟩
theorem plain_merge_admitted : isDone (Cut.decide p dates o plain_merge_engine plain_merge_before ⟨"A","B",some .confirm,true⟩) = true := by decide +kernel


end CutExamples
end CM
