import CutFixtureProof78
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
def empty_alias_before : Graph := [⟨"pages/A.md",.pages,.md,"- source\n"⟩,⟨"pages/B.md",.pages,.md,"alias::\n- target\n"⟩]
def empty_alias_after : Graph := [⟨"pages/B.md",.pages,.md,"alias::\n- target\n- source\n"⟩]
def empty_alias_engine : Engine := ⟨fun _ => some empty_alias_after⟩
theorem empty_alias_admitted : isDone (Cut.decide p dates o empty_alias_engine empty_alias_before ⟨"A","B",some .confirm,true⟩) = true := by decide +kernel


end CutExamples
end CM
