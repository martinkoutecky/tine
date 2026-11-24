import CutFixtureProof83
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
def reference_destination_before : Graph := [⟨"pages/A.md",.pages,.md,"- notes\n"⟩,⟨"pages/R.md",.pages,.md,"- [[B]]\n"⟩]
def reference_destination_after : Graph := [⟨"pages/B.md",.pages,.md,"- notes\n"⟩,⟨"pages/R.md",.pages,.md,"- [[B]]\n"⟩]
def reference_destination_engine : Engine := ⟨fun _ => some reference_destination_after⟩
theorem reference_destination_admitted : isDone (Cut.decide p dates o reference_destination_engine reference_destination_before ⟨"A","B",none,true⟩) = true := by decide +kernel


end CutExamples
end CM
