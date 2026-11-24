import CutFixtureProof60
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
theorem plain_rename_admitted : isDone (Cut.decide p dates o plain_rename_engine plain_rename_before ⟨"A","B",none,false⟩) = true := by decide +kernel


end CutExamples
end CM
