import CutFixtureProof96
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
def unchanged_alias_contested_before : Graph := [⟨"pages/A.md",.pages,.md,"alias:: X\n- source\n"⟩,⟨"pages/P.md",.pages,.md,"title:: X\n- p\n"⟩,⟨"pages/Q.md",.pages,.md,"title:: X\n- q\n"⟩]
theorem unchanged_alias_contested_refused : select p dates o unchanged_alias_contested_before ⟨"A","B",none,true⟩ = .refuse (.contested) := by decide +kernel


end CutExamples
end CM
