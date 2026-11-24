import CutFixtureProof104
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
def conflicted_merge_source_before : Graph := [⟨"pages/A.md",.pages,.md,"<<<<<<< a\n=======\n>>>>>>> b\n- source\n"⟩,⟨"pages/B.md",.pages,.md,"- target\n"⟩]
theorem conflicted_merge_source_refused : select p dates o conflicted_merge_source_before ⟨"A","B",some .confirm,true⟩ = .refuse (.conflictPair) := by decide +kernel


end CutExamples
end CM
