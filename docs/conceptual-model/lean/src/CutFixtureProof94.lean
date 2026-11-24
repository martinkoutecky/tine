import CutFixtureProof93
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
def builtin_different_survivor_before : Graph := [⟨"pages/A.md",.pages,.md,"icon:: star\n- source\n"⟩,⟨"pages/B.md",.pages,.md,"public:: true\n- target\n"⟩]
def builtin_different_survivor_after : Graph := [⟨"pages/B.md",.pages,.md,"public:: true\nicon:: star\n- target\n- source\n"⟩]
def builtin_different_survivor_engine : Engine := ⟨fun _ => some builtin_different_survivor_after⟩
theorem builtin_different_survivor_admitted : isDone (Cut.decide p dates o builtin_different_survivor_engine builtin_different_survivor_before ⟨"A","B",some .confirm,true⟩) = true := by decide +kernel


end CutExamples
end CM
