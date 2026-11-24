import CutFixtureProof85
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
def joined_before_survivor_before : Graph := [⟨"pages/A.md",.pages,.md,"join:: value\n- s\n"⟩,⟨"pages/B.md",.pages,.md,"clash:: target\n- t\n"⟩]
def joined_before_survivor_after : Graph := [⟨"pages/B.md",.pages,.md,"join:: value\nclash:: target\n- t\n- s\n"⟩]
def joined_before_survivor_engine : Engine := ⟨fun _ => some joined_before_survivor_after⟩
theorem joined_before_survivor_admitted : isDone (Cut.decide p dates o joined_before_survivor_engine joined_before_survivor_before ⟨"A","B",some .confirm,true⟩) = true := by decide +kernel


end CutExamples
end CM
