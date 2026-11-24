import CutFixtureProof67
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
def md_tag_before : Graph := [⟨"pages/Old.md",.pages,.md,"- notes\n"⟩,⟨"pages/R.md",.pages,.md,"- #Old\n"⟩]
def md_tag_after : Graph := [⟨"pages/Two Words.md",.pages,.md,"- notes\n"⟩,⟨"pages/R.md",.pages,.md,"- #[[Two Words]]\n"⟩]
def md_tag_engine : Engine := ⟨fun _ => some md_tag_after⟩
theorem md_tag_admitted : isDone (Cut.decide p dates o md_tag_engine md_tag_before ⟨"Old","Two Words",none,true⟩) = true := by decide +kernel


end CutExamples
end CM
