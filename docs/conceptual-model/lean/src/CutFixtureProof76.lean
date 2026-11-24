import CutFixtureProof75
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
def date_title_before : Graph := [⟨"pages/A.md",.pages,.md,"- source\n"⟩,⟨"pages/physical.md",.pages,.md,"title:: 2026-11-02\n- target\n"⟩]
def date_title_after : Graph := [⟨"pages/physical.md",.pages,.md,"title:: 2026-11-02\n- target\n- source\n"⟩]
def date_title_engine : Engine := ⟨fun _ => some date_title_after⟩
theorem date_title_admitted : isDone (Cut.decide p dates o date_title_engine date_title_before ⟨"A","2026-11-02",some .confirm,true⟩) = true := by decide +kernel


end CutExamples
end CM
