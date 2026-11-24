import CutFixtureProof64
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
def mixed_exceptions_before : Graph := [⟨"pages/Old.md",.pages,.md,"- notes\n"⟩,⟨"pages/Conflict.md",.pages,.md,"<<<<<<< a\n=======\n>>>>>>> b\n- [[Old]]\n"⟩,⟨"pages/Org.org",.pages,.org,"* h :Old:\n"⟩,⟨"pages/OrgConflict.org",.pages,.org,"<<<<<<< a\n=======\n>>>>>>> b\n* h :Old:\n"⟩,⟨"pages/Tag.md",.pages,.md,"- #Old\n"⟩]
def mixed_exceptions_after : Graph := [⟨"pages/Two Words.md",.pages,.md,"- notes\n"⟩,⟨"pages/Conflict.md",.pages,.md,"<<<<<<< a\n=======\n>>>>>>> b\n- [[Old]]\n"⟩,⟨"pages/Org.org",.pages,.org,"* h :Old:\n"⟩,⟨"pages/OrgConflict.org",.pages,.org,"<<<<<<< a\n=======\n>>>>>>> b\n* h :Old:\n"⟩,⟨"pages/Tag.md",.pages,.md,"- #[[Two Words]]\n"⟩]
def mixed_exceptions_engine : Engine := ⟨fun _ => some mixed_exceptions_after⟩
theorem mixed_exceptions_admitted : isDone (Cut.decide p dates o mixed_exceptions_engine mixed_exceptions_before ⟨"Old","Two Words",none,true⟩) = true := by decide +kernel


end CutExamples
end CM
