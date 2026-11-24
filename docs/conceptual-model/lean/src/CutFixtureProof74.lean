import CutFixtureProof73
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
def fileless_merge_before : Graph := [⟨"pages/R.md",.pages,.md,"- [[A]]\n"⟩,⟨"pages/B.md",.pages,.md,"- target\n"⟩]
def fileless_merge_after : Graph := [⟨"pages/R.md",.pages,.md,"- [[B]]\n"⟩,⟨"pages/B.md",.pages,.md,"- target\n"⟩]
def fileless_merge_engine : Engine := ⟨fun _ => some fileless_merge_after⟩
theorem fileless_merge_admitted : isDone (Cut.decide p dates o fileless_merge_engine fileless_merge_before ⟨"A","B",some .confirm,true⟩) = true := by decide +kernel


end CutExamples
end CM
