import CutFixtureProof71
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
def repeated_key_before : Graph := [⟨"pages/A.md",.pages,.md,"x:: one\nx:: two\n- source\n"⟩,⟨"pages/B.md",.pages,.md,"- target\n"⟩]
def repeated_key_after : Graph := [⟨"pages/B.md",.pages,.md,"x:: one\n- target\n- \n  x:: two\n- source\n"⟩]
def repeated_key_engine : Engine := ⟨fun _ => some repeated_key_after⟩
theorem repeated_key_admitted : isDone (Cut.decide p dates o repeated_key_engine repeated_key_before ⟨"A","B",some .confirm,true⟩) = true := by decide +kernel


end CutExamples
end CM
