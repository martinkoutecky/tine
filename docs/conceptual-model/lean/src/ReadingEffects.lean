import Date13
namespace CM
namespace Consolidated13
def added {α : Type} [DecidableEq α] (before after : List α) : List α :=
  after.eraseDups.filter (fun x => !before.contains x)
def removed {α : Type} [DecidableEq α] (before after : List α) : List α := added after before

theorem added_exact {α : Type} [DecidableEq α] (A B : List α) (x : α) :
    x ∈ added A B ↔ x ∈ B ∧ x ∉ A := by simp [added]

theorem removed_exact {α : Type} [DecidableEq α] (A B : List α) (x : α) :
    x ∈ removed A B ↔ x ∈ A ∧ x ∉ B := by simp [removed, added]

theorem changes_complete {α : Type} [DecidableEq α] (A B : List α) (x : α) :
    (x ∈ A ↔ x ∈ B) ↔ x ∉ added A B ∧ x ∉ removed A B := by
  rw [added_exact, removed_exact]
  constructor
  · intro h; exact ⟨fun ⟨hb, ha⟩ => ha (h.mpr hb), fun ⟨ha, hb⟩ => hb (h.mp ha)⟩
  · rintro ⟨hn, hm⟩
    constructor
    · intro ha; by_cases hb : x ∈ B; exact hb; exact False.elim (hm ⟨ha, hb⟩)
    · intro hb; by_cases ha : x ∈ A; exact ha; exact False.elim (hn ⟨hb, ha⟩)

end Consolidated13
end CM
