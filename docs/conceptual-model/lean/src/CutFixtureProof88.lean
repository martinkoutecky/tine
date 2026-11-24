import CutFixtureProof87
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
def bare_bullet_header_before : Graph := [⟨"pages/A.md",.pages,.md,"- \n- source\n"⟩,⟨"pages/B.md",.pages,.md,"- target\n"⟩]
def bare_bullet_header_after : Graph := [⟨"pages/B.md",.pages,.md,"- target\n- source\n"⟩]
def bare_bullet_header_engine : Engine := ⟨fun _ => some bare_bullet_header_after⟩
theorem bare_bullet_header_admitted : isDone (Cut.decide p dates o bare_bullet_header_engine bare_bullet_header_before ⟨"A","B",some .confirm,true⟩) = true := by decide +kernel


end CutExamples
end CM
