import Lean
namespace CM

def minBy {α : Type} (le : α → α → Bool) : List α → Option α
  | [] => none
  | a :: l =>
    match minBy le l with
    | none => some a
    | some m => if le a m then some a else some m

variable {α : Type} {le : α → α → Bool}

theorem minBy_eq_none {l : List α} : minBy le l = none ↔ l = [] := by
  cases l with
  | nil => simp [minBy]
  | cons a t =>
    simp only [minBy, reduceCtorEq, iff_false]
    cases minBy le t with
    | none => simp
    | some m => by_cases h : le a m = true <;> simp [h]

theorem minBy_mem {l : List α} {m : α} : minBy le l = some m → m ∈ l := by
  induction l with
  | nil => simp [minBy]
  | cons a t ih =>
    simp only [minBy]
    cases h : minBy le t with
    | none => intro e; simp at e; simp [e]
    | some m0 =>
      simp only
      by_cases hl : le a m0 = true
      · simp [hl]; intro e; simp [e]
      · simp [hl]; intro e; subst e; exact Or.inr (ih h)

end CM
