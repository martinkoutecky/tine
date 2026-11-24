import CutFixtureProof95
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
def alias_held_before : Graph := [⟨"pages/A.md",.pages,.md,"- notes\n"⟩,⟨"pages/C.md",.pages,.md,"alias:: X\n- owner\n"⟩]
theorem alias_held_refused : select p dates o alias_held_before ⟨"A","X",none,true⟩ = .refuse (.aliasHeld "pages/C.md") := by decide +kernel


end CutExamples
end CM
