/-
The least element of a finite list under a linear order, and the fact that it depends only on the
*set* of members (not on their order or multiplicity). This is what makes `owner` (SPEC §2.3) and the
choice among direct declarers (SPEC §4.3, open point O2) functions of the graph as a set of files.
-/
namespace CM

/-- `le` is a linear order (total, antisymmetric, transitive), given as a Boolean function so that
the model stays executable. -/
structure Linear {α : Type} (le : α → α → Bool) : Prop where
  total : ∀ a b, le a b = true ∨ le b a = true
  antisymm : ∀ a b, le a b = true → le b a = true → a = b
  trans : ∀ a b c, le a b = true → le b c = true → le a c = true

/-- The least member of a list under `le` (the first one met on ties, which a linear order rules out). -/
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

theorem minBy_le (hL : Linear le) {l : List α} {m : α} :
    minBy le l = some m → ∀ x ∈ l, le m x = true := by
  have refl : ∀ y, le y y = true := fun y => by rcases hL.total y y with h1 | h1 <;> exact h1
  induction l generalizing m with
  | nil => simp [minBy]
  | cons a t ih =>
    simp only [minBy]
    cases h : minBy le t with
    | none =>
      have ht : t = [] := minBy_eq_none.mp h
      subst ht
      intro e x hx
      have e' : a = m := by simpa using e
      have hx' : x = a := by simpa using hx
      rw [hx', e']; exact refl m
    | some m0 =>
      simp only
      have ih0 := ih h
      by_cases hl : le a m0 = true
      · intro e x hx
        have e' : a = m := by simpa [hl] using e
        rw [← e']
        rcases List.mem_cons.mp hx with hx | hx
        · rw [hx]; exact refl a
        · exact hL.trans _ _ _ hl (ih0 x hx)
      · intro e x hx
        have e' : m0 = m := by simpa [hl] using e
        rw [← e']
        rcases List.mem_cons.mp hx with hx | hx
        · rw [hx]
          rcases hL.total m0 a with h1 | h1
          · exact h1
          · exact absurd h1 hl
        · exact ih0 x hx

/-- **Set-independence of the least element**: two lists with the same members have the same least
member. -/
theorem minBy_congr (hL : Linear le) {l₁ l₂ : List α} (h : ∀ x, x ∈ l₁ ↔ x ∈ l₂) :
    minBy le l₁ = minBy le l₂ := by
  cases h1 : minBy le l₁ with
  | none =>
    have e1 : l₁ = [] := minBy_eq_none.mp h1
    have e2 : l₂ = [] := by
      cases l₂ with
      | nil => rfl
      | cons b t =>
        have : b ∈ l₁ := (h b).mpr (List.mem_cons_self ..)
        rw [e1] at this; simp at this
    rw [e2]; rfl
  | some m =>
    have hm2 : m ∈ l₂ := (h m).mp (minBy_mem h1)
    cases h2 : minBy le l₂ with
    | none => rw [minBy_eq_none.mp h2] at hm2; simp at hm2
    | some m' =>
      have hm'1 : m' ∈ l₁ := (h m').mpr (minBy_mem h2)
      have a1 := minBy_le hL h1 m' hm'1
      have a2 := minBy_le hL h2 m hm2
      rw [hL.antisymm _ _ a1 a2]

end CM
