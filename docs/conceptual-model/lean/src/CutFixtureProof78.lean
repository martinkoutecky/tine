import CutFixtureProof77
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
def child_title_case_only_before : Graph := [⟨"pages/Old.md",.pages,.md,"- notes\n"⟩,⟨"pages/childcase.md",.pages,.md,"title:: Old/[[Old]]\n- child\n"⟩]
def child_title_case_only_after : Graph := [⟨"pages/old.md",.pages,.md,"- notes\n"⟩,⟨"pages/old%2F[[Old]].md",.pages,.md,"title:: old/[[old]]\n- child\n"⟩]
def child_title_case_only_engine : Engine := ⟨fun _ => some child_title_case_only_after⟩
theorem child_title_case_only_admitted : isDone (Cut.decide p dates o child_title_case_only_engine child_title_case_only_before ⟨"Old","old",none,true⟩) = true := by decide +kernel


end CutExamples
end CM
