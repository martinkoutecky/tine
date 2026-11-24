import CutFixtureProof92
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
def builtin_same_leftover_before : Graph := [⟨"pages/A.md",.pages,.md,"icon:: star\n- source\n"⟩,⟨"pages/B.md",.pages,.md,"icon:: moon\n- target\n"⟩]
def builtin_same_leftover_after : Graph := [⟨"pages/B.md",.pages,.md,"icon:: moon\n- target\n- \n  icon:: star\n- source\n"⟩]
def builtin_same_leftover_engine : Engine := ⟨fun _ => some builtin_same_leftover_after⟩
theorem builtin_same_leftover_admitted : isDone (Cut.decide p dates o builtin_same_leftover_engine builtin_same_leftover_before ⟨"A","B",some .confirm,true⟩) = true := by decide +kernel


end CutExamples
end CM
