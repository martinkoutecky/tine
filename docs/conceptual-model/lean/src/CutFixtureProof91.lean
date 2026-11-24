import CutFixtureProof90
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
def leftover_reference_md_before : Graph := [⟨"pages/A.md",.pages,.md,"key:: [[X]]\n- source\n"⟩,⟨"pages/B.md",.pages,.md,"key:: target\n- target\n"⟩]
def leftover_reference_md_after : Graph := [⟨"pages/B.md",.pages,.md,"key:: target\n- target\n- \n  key:: [[X]]\n- source\n"⟩]
def leftover_reference_md_engine : Engine := ⟨fun _ => some leftover_reference_md_after⟩
theorem leftover_reference_md_admitted : isDone (Cut.decide p dates o leftover_reference_md_engine leftover_reference_md_before ⟨"A","B",some .confirm,true⟩) = true := by decide +kernel


end CutExamples
end CM
