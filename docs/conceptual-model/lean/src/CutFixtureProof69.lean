import CutFixtureProof68
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
def key_spelling_before : Graph := [⟨"pages/Old.md",.pages,.md,"- notes\n"⟩,⟨"pages/R.md",.pages,.md,"- Old:: value\n- Old:: another\n"⟩]
def key_spelling_after : Graph := [⟨"pages/Two Words.md",.pages,.md,"- notes\n"⟩,⟨"pages/R.md",.pages,.md,"- two-words:: value\n- two-words:: another\n"⟩]
def key_spelling_engine : Engine := ⟨fun _ => some key_spelling_after⟩
theorem key_spelling_admitted : isDone (Cut.decide p dates o key_spelling_engine key_spelling_before ⟨"Old","Two Words",none,true⟩) = true := by decide +kernel


end CutExamples
end CM
