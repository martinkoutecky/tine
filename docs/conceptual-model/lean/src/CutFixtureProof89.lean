import CutFixtureProof88
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
def org_tag_spellable_before : Graph := [⟨"pages/Old.md",.pages,.md,"- notes\n"⟩,⟨"pages/Org.org",.pages,.org,"* h :Old:\n"⟩]
def org_tag_spellable_after : Graph := [⟨"pages/New.md",.pages,.md,"- notes\n"⟩,⟨"pages/Org.org",.pages,.org,"* h :New:\n"⟩]
def org_tag_spellable_engine : Engine := ⟨fun _ => some org_tag_spellable_after⟩
theorem org_tag_spellable_admitted : isDone (Cut.decide p dates o org_tag_spellable_engine org_tag_spellable_before ⟨"Old","New",none,true⟩) = true := by decide +kernel


end CutExamples
end CM
