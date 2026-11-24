import CutFixtureProof94
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
def renamed_key_clash_before : Graph := [⟨"pages/Old.md",.pages,.md,"Old:: source\n- source\n"⟩,⟨"pages/Two Words.md",.pages,.md,"two-words:: target\n- target\n"⟩]
def renamed_key_clash_after : Graph := [⟨"pages/Two Words.md",.pages,.md,"two-words:: target\n- target\n- \n  two-words:: source\n- source\n"⟩]
def renamed_key_clash_engine : Engine := ⟨fun _ => some renamed_key_clash_after⟩
theorem renamed_key_clash_admitted : isDone (Cut.decide p dates o renamed_key_clash_engine renamed_key_clash_before ⟨"Old","Two Words",some .confirm,true⟩) = true := by decide +kernel


end CutExamples
end CM
