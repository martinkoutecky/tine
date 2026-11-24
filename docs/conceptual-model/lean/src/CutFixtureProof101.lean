import CutFixtureProof100
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
def child_title_conflict_before : Graph := [⟨"pages/Old.md",.pages,.md,"- notes\n"⟩,⟨"pages/badchild.md",.pages,.md,"title:: Old/[[Old]]\n- child\n"⟩]
def child_title_conflict_engine : Engine := ⟨fun _ => some [⟨"pages/New.md",.pages,.md,"- notes\n"⟩,⟨"pages/New%2F[[Old]].md",.pages,.md,"title:: New/[[New]]\n- child\n"⟩]⟩
theorem child_title_conflict_refused : Cut.decide p dates o child_title_conflict_engine child_title_conflict_before ⟨"Old","New",none,true⟩ = .refuse (.titleIdentity) := by decide +kernel


end CutExamples
end CM
