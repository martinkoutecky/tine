import CutFixtureProof86
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
def blank_header_before : Graph := [⟨"pages/A.md",.pages,.md,"\n- source\n"⟩,⟨"pages/B.md",.pages,.md,"- target\n"⟩]
def blank_header_after : Graph := [⟨"pages/B.md",.pages,.md,"- target\n- source\n"⟩]
def blank_header_engine : Engine := ⟨fun _ => some blank_header_after⟩
theorem blank_header_admitted : isDone (Cut.decide p dates o blank_header_engine blank_header_before ⟨"A","B",some .confirm,true⟩) = true := by decide +kernel


end CutExamples
end CM
