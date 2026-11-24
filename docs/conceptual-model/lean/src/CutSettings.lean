import Journal14
namespace CM
namespace Cut
open Consolidated13
variable {K : Type} [DecidableEq K] (P : Inputs K) (D : Date13.Dates K)
structure Settings where
  favorites : List String
  favoritesPage : String
  home : String
  deriving DecidableEq, Repr
inductive SettingsStatus | updated | pagesDoneSettingsNotUpdated
  deriving DecidableEq, Repr
def favoritesAfter (cfg : Settings) (old : String) (p : Plan) : List String :=
  let names := cfg.favorites.map (mappedName P D old p)
  if p.survivor.isSome then uniqueNames P D names else names

def settingsAfter (cfg : Settings) (old : String) (p : Plan) : Settings :=
  ⟨favoritesAfter P D cfg old p,mappedName P D old p cfg.favoritesPage,
    if p.survivor.isSome && Decidable.decide (P.key cfg.home = P.key old) then cfg.home else mappedName P D old p cfg.home⟩

structure SettingsStep where
  settings : Settings
  status : SettingsStatus
  deriving DecidableEq, Repr

/-- Called only after a done page outcome; it cannot affect the page decision. -/
def finishSettings (succeeded : Settings → Bool) (cfg : Settings) (old : String) (pages : Done) : SettingsStep :=
  let intended := settingsAfter P D cfg old pages.plan
  if succeeded intended then ⟨intended,.updated⟩
  else ⟨cfg,.pagesDoneSettingsNotUpdated⟩

theorem done_settings_atomic (succeeded : Settings → Bool) (cfg : Settings) (old : String) (pages : Done) :
    let step := finishSettings P D succeeded cfg old pages
    (step.status = .updated ∧ step.settings = settingsAfter P D cfg old pages.plan) ∨
    (step.status = .pagesDoneSettingsNotUpdated ∧ step.settings = cfg) := by
  dsimp only [finishSettings]
  split
  · exact Or.inl ⟨rfl,rfl⟩
  · exact Or.inr ⟨rfl,rfl⟩

theorem favorites_rename (cfg : Settings) (old : String) (p : Plan) (h : p.survivor = none) :
    (settingsAfter P D cfg old p).favorites = cfg.favorites.map (mappedName P D old p) := by
  simp [settingsAfter,favoritesAfter,h]

theorem favorites_merge (cfg : Settings) (old : String) (p : Plan) (h : p.survivor.isSome = true) :
    (settingsAfter P D cfg old p).favorites = uniqueNames P D (cfg.favorites.map (mappedName P D old p)) := by
  simp [settingsAfter,favoritesAfter,h]

theorem favorites_page_follows (cfg : Settings) (old : String) (p : Plan) :
    (settingsAfter P D cfg old p).favoritesPage = mappedName P D old p cfg.favoritesPage := rfl

theorem home_merge_stays (cfg : Settings) (old : String) (p : Plan)
    (h : p.survivor.isSome = true) (hk : P.key cfg.home = P.key old) :
    (settingsAfter P D cfg old p).home = cfg.home := by simp [settingsAfter,h,hk]

theorem home_other_follows (cfg : Settings) (old : String) (p : Plan)
    (h : p.survivor.isSome = false ∨ P.key cfg.home ≠ P.key old) :
    (settingsAfter P D cfg old p).home = mappedName P D old p cfg.home := by
  rcases h with h | h <;> simp [settingsAfter,h]

end Cut
end CM
