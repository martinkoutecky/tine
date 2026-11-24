import CutFixtureProof91
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
def builtin_distinct_join_before : Graph := [⟨"pages/A.md",.pages,.md,"icon:: star\npublic:: true\n- source\n"⟩,⟨"pages/B.md",.pages,.md,"- target\n"⟩]
def builtin_distinct_join_after : Graph := [⟨"pages/B.md",.pages,.md,"icon:: star\npublic:: true\n- target\n- source\n"⟩]
def builtin_distinct_join_engine : Engine := ⟨fun _ => some builtin_distinct_join_after⟩
theorem builtin_distinct_join_admitted : isDone (Cut.decide p dates o builtin_distinct_join_engine builtin_distinct_join_before ⟨"A","B",some .confirm,true⟩) = true := by decide +kernel


end CutExamples
end CM
