import CutFixtureProof111
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
def leftover_changed_text_md_before : Graph := [⟨"pages/A.md",.pages,.md,"key:: [[X]]\n- source\n"⟩,⟨"pages/B.md",.pages,.md,"key:: target\n- target\n"⟩]
def leftover_changed_text_md_engine : Engine := ⟨fun _ => some [⟨"pages/B.md",.pages,.md,"key:: target\n- target\n- \n  key:: X\n- source\n"⟩]⟩
theorem leftover_changed_text_md_refused : Cut.decide p dates o leftover_changed_text_md_engine leftover_changed_text_md_before ⟨"A","B",some .confirm,true⟩ = .refuse (.observationsP2) := by decide +kernel


end CutExamples
end CM
