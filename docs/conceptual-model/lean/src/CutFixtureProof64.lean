import CutFixtureProof63
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
def org_title_before : Graph := [⟨"pages/A.org",.pages,.org,"#+title: A\n* source\n"⟩,⟨"pages/B.org",.pages,.org,"#+title: B\n* target\n"⟩]
def org_title_after : Graph := [⟨"pages/B.org",.pages,.org,"#+title: B\n* target\n* \n#+BEGIN_QUOTE\n#+title: B\n#+END_QUOTE\n* source\n"⟩]
def org_title_engine : Engine := ⟨fun _ => some org_title_after⟩
theorem org_title_admitted : isDone (Cut.decide p dates o org_title_engine org_title_before ⟨"A","B",some .confirm,true⟩) = true := by decide +kernel


end CutExamples
end CM
