import CutFixtureProof108
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
def org_leftover_reference_before : Graph := [⟨"pages/A.org",.pages,.org,"#+key: [[X]]\n* source\n"⟩,⟨"pages/B.org",.pages,.org,"#+key: target\n* target\n"⟩]
theorem org_leftover_reference_refused : select p dates o org_leftover_reference_before ⟨"A","B",some .confirm,true⟩ = .refuse (.leftoverReference) := by decide +kernel


end CutExamples
end CM
