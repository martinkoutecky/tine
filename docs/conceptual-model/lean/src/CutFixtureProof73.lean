import CutFixtureProof72
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
def fileless_before : Graph := [⟨"pages/R.md",.pages,.md,"- [[A]]\n"⟩]
def fileless_after : Graph := [⟨"pages/R.md",.pages,.md,"- [[B]]\n"⟩]
def fileless_engine : Engine := ⟨fun _ => some fileless_after⟩
theorem fileless_admitted : isDone (Cut.decide p dates o fileless_engine fileless_before ⟨"A","B",none,false⟩) = true := by decide +kernel


end CutExamples
end CM
