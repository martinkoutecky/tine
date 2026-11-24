import CutFixtureProof66
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
def alias_spellings_before : Graph := [⟨"pages/A.md",.pages,.md,"alias:: C#\n- source\n"⟩,⟨"pages/B.md",.pages,.md,"alias:: C\\#\n- target\n"⟩]
def alias_spellings_after : Graph := [⟨"pages/B.md",.pages,.md,"alias:: C\\#\n- target\n- source\n"⟩]
def alias_spellings_engine : Engine := ⟨fun _ => some alias_spellings_after⟩
theorem alias_spellings_admitted : isDone (Cut.decide p dates o alias_spellings_engine alias_spellings_before ⟨"A","B",some .confirm,true⟩) = true := by decide +kernel


end CutExamples
end CM
