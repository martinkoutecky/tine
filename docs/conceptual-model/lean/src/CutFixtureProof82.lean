import CutFixtureProof81
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
def child_title_no_holder_before : Graph := [⟨"pages/Old.md",.pages,.md,"- notes\n"⟩,⟨"pages/child.md",.pages,.md,"title:: Old/[[New]] [[X]]\n- child\n"⟩]
def child_title_no_holder_after : Graph := [⟨"pages/New.md",.pages,.md,"- notes\n"⟩,⟨"pages/New%2F[[New]] [[X]].md",.pages,.md,"title:: New/[[New]] [[X]]\n- child\n"⟩]
def child_title_no_holder_engine : Engine := ⟨fun _ => some child_title_no_holder_after⟩
theorem child_title_no_holder_admitted : isDone (Cut.decide p dates o child_title_no_holder_engine child_title_no_holder_before ⟨"Old","New",none,true⟩) = true := by decide +kernel


end CutExamples
end CM
