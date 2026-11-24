import CutFixtureProof69
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
def self_alias_true_before : Graph := [⟨"pages/A.md",.pages,.md,"alias:: X\n- source\n"⟩]
def self_alias_true_after : Graph := [⟨"pages/X.md",.pages,.md,"alias:: X\n- source\n"⟩]
def self_alias_true_engine : Engine := ⟨fun _ => some self_alias_true_after⟩
theorem self_alias_true_admitted : isDone (Cut.decide p dates o self_alias_true_engine self_alias_true_before ⟨"A","X",none,true⟩) = true := by decide +kernel


end CutExamples
end CM
