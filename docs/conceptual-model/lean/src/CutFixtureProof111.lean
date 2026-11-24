import CutFixtureProof110
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
def leftover_changed_text_before : Graph := [⟨"pages/A.org",.pages,.org,"#+key: [[X]]\n* source\n"⟩,⟨"pages/B.org",.pages,.org,"#+key: target\n* target\n"⟩]
def leftover_changed_text_engine : Engine := ⟨fun _ => some [⟨"pages/B.org",.pages,.org,"#+key: target\n* target\n* \n#+BEGIN_QUOTE\n#+key: X\n#+END_QUOTE\n* source\n"⟩]⟩
theorem leftover_changed_text_refused : Cut.decide p dates o leftover_changed_text_engine leftover_changed_text_before ⟨"A","B",some .confirm,true⟩ = .refuse (.leftoverReference) := by decide +kernel


end CutExamples
end CM
