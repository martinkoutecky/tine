import CutFixtureProof76
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
def child_title_nonconflicting_before : Graph := [⟨"pages/Old.md",.pages,.md,"- notes\n"⟩,⟨"pages/New.md",.pages,.md,"- target\n"⟩,⟨"pages/child.md",.pages,.md,"title:: Old/[[New]] [[X]]\n- child\n"⟩]
def child_title_nonconflicting_after : Graph := [⟨"pages/New.md",.pages,.md,"- target\n- notes\n"⟩,⟨"pages/New%2F[[New]] [[X]].md",.pages,.md,"title:: New/[[New]] [[X]]\n- child\n"⟩]
def child_title_nonconflicting_engine : Engine := ⟨fun _ => some child_title_nonconflicting_after⟩
theorem child_title_nonconflicting_admitted : isDone (Cut.decide p dates o child_title_nonconflicting_engine child_title_nonconflicting_before ⟨"Old","New",some .confirm,true⟩) = true := by decide +kernel


end CutExamples
end CM
