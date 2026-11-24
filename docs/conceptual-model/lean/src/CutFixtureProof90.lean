import CutFixtureProof89
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
def header_permutation_before : Graph := [⟨"pages/A.md",.pages,.md,"j:: J\nk:: K\n- source\n"⟩,⟨"pages/B.md",.pages,.md,"x:: X\ny:: Y\n- target\n"⟩]
def header_permutation_after : Graph := [⟨"pages/B.md",.pages,.md,"k:: K\nx:: X\nj:: J\ny:: Y\n- target\n- source\n"⟩]
def header_permutation_engine : Engine := ⟨fun _ => some header_permutation_after⟩
theorem header_permutation_admitted : isDone (Cut.decide p dates o header_permutation_engine header_permutation_before ⟨"A","B",some .confirm,true⟩) = true := by decide +kernel


end CutExamples
end CM
