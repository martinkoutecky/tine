import CutFixtureProof98
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
def folded_occupant_before : Graph := [⟨"pages/A.md",.pages,.md,"- notes\n"⟩,⟨"pages/b.md",.pages,.md,"title:: C\n- occupied\n"⟩]
theorem folded_occupant_refused : select p dates o folded_occupant_before ⟨"A","B",none,true⟩ = .refuse (.pathTaken) := by decide +kernel


end CutExamples
end CM
