import CutFixtureProof82
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
def coincident_before : Graph := [⟨"pages/A.md",.pages,.md,"- notes\n"⟩,⟨"pages/T.md",.pages,.md,"title:: [[X]]\n- notes\n"⟩]
def coincident_after : Graph := [⟨"pages/B.md",.pages,.md,"- notes\n"⟩,⟨"pages/T.md",.pages,.md,"title:: [[X]]\n- notes\n"⟩]
def coincident_engine : Engine := ⟨fun _ => some coincident_after⟩
theorem coincident_admitted : isDone (Cut.decide p dates o coincident_engine coincident_before ⟨"A","B",none,true⟩) = true := by decide +kernel


end CutExamples
end CM
