import CutFixtureProof106
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
def split_alias_before : Graph := [⟨"pages/A.md",.pages,.md,"alias:: X\n- source\n"⟩,⟨"pages/B.md",.pages,.md,"alias:: Y\n- target\n"⟩]
def split_alias_engine : Engine := ⟨fun _ => some [⟨"pages/B.md",.pages,.md,"alias:: Y\nalias:: X\n- target\n- source\n"⟩]⟩
theorem split_alias_refused : Cut.decide p dates o split_alias_engine split_alias_before ⟨"A","B",some .confirm,true⟩ = .refuse (.observationsP2) := by decide +kernel


end CutExamples
end CM
