import CutFixtureOwnership
import CutSettings
namespace CM
namespace CutExamples
open Cut
def cfg : Settings := ⟨["A","X","B"],"Favorites","A"⟩
def mergePlan : Plan := ⟨"B",some ⟨"pages/A.md",.pages,.md,"- source\n"⟩,some ⟨"pages/B.md",.pages,.md,"- target\n"⟩⟩
theorem favorites_first_name : (settingsAfter p dates cfg "A" mergePlan).favorites = ["B","X"] := by decide +kernel
theorem home_merge_stays : (settingsAfter p dates cfg "A" mergePlan).home = "A" := by decide +kernel

end CutExamples
end CM
