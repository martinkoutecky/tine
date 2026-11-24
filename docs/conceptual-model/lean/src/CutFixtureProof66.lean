import CutFixtureProof65
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
def unterminated_before : Graph := [⟨"pages/A.md",.pages,.md,"- source\n"⟩,⟨"pages/B.md",.pages,.md,"- target"⟩]
def unterminated_after : Graph := [⟨"pages/B.md",.pages,.md,"- target\n- source\n"⟩]
def unterminated_engine : Engine := ⟨fun _ => some unterminated_after⟩
theorem unterminated_admitted : isDone (Cut.decide p dates o unterminated_engine unterminated_before ⟨"A","B",some .confirm,true⟩) = true := by decide +kernel


end CutExamples
end CM
