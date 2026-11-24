import CutFixtureProof84
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
def day_under_source_before : Graph := [⟨"pages/2026.md",.pages,.md,"- notes\n"⟩,⟨"pages/dayphysical.md",.pages,.md,"title:: 2026/10/26\n- day\n"⟩,⟨"pages/R.md",.pages,.md,"- [[2026/10/26]] [[2026/x]]\n"⟩]
def day_under_source_after : Graph := [⟨"pages/B.md",.pages,.md,"- notes\n"⟩,⟨"pages/dayphysical.md",.pages,.md,"title:: 2026/10/26\n- day\n"⟩,⟨"pages/R.md",.pages,.md,"- [[2026/10/26]] [[B/x]]\n"⟩]
def day_under_source_engine : Engine := ⟨fun _ => some day_under_source_after⟩
theorem day_under_source_admitted : isDone (Cut.decide p dates o day_under_source_engine day_under_source_before ⟨"2026","B",none,true⟩) = true := by decide +kernel


end CutExamples
end CM
