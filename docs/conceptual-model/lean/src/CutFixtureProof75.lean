import CutFixtureProof74
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
def alias_only_before : Graph := [⟨"pages/C.md",.pages,.md,"alias:: A\n- owner\n"⟩]
def alias_only_after : Graph := [⟨"pages/C.md",.pages,.md,"alias:: B\n- owner\n"⟩]
def alias_only_engine : Engine := ⟨fun _ => some alias_only_after⟩
theorem alias_only_admitted : isDone (Cut.decide p dates o alias_only_engine alias_only_before ⟨"A","B",none,false⟩) = true := by decide +kernel


end CutExamples
end CM
