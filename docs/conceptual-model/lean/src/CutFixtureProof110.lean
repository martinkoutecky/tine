import CutFixtureProof109
set_option maxRecDepth 16384
set_option maxHeartbeats 8000000
namespace CM
namespace CutExamples
open Cut
def header_missing_entry_before : Graph := [⟨"pages/A.md",.pages,.md,"j:: J\nk:: K\n- source\n"⟩,⟨"pages/B.md",.pages,.md,"x:: X\ny:: Y\n- target\n"⟩]
def header_missing_entry_engine : Engine := ⟨fun _ => some [⟨"pages/B.md",.pages,.md,"k:: K\nx:: X\ny:: Y\n- target\n- source\n"⟩]⟩
theorem header_missing_entry_refused : Cut.decide p dates o header_missing_entry_engine header_missing_entry_before ⟨"A","B",some .confirm,true⟩ = .refuse (.observationsP2) := by decide +kernel


end CutExamples
end CM
