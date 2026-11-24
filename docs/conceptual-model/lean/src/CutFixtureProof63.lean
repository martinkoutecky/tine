import CutFixtureProof62
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
def join_clash_before : Graph := [⟨"pages/A.md",.pages,.md,"clash:: source\njoin:: value\n- s\n"⟩,⟨"pages/B.md",.pages,.md,"clash:: target\n- t\n"⟩]
def join_clash_after : Graph := [⟨"pages/B.md",.pages,.md,"clash:: target\njoin:: value\n- t\n- \n  clash:: source\n- s\n"⟩]
def join_clash_engine : Engine := ⟨fun _ => some join_clash_after⟩
theorem join_clash_admitted : isDone (Cut.decide p dates o join_clash_engine join_clash_before ⟨"A","B",some .confirm,true⟩) = true := by decide +kernel


end CutExamples
end CM
