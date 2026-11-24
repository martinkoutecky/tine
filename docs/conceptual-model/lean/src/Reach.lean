import MinBy
namespace CM

inductive Reach {α : Type} (E : List (α × α)) : α → α → Prop
  | refl (a : α) : Reach E a a
  | fwd {a b c : α} : Reach E a b → (b, c) ∈ E → Reach E a c
  | bwd {a b c : α} : Reach E a b → (c, b) ∈ E → Reach E a c

namespace Reach

variable {α : Type} {E E' : List (α × α)}

theorem edge {a b : α} (h : (a, b) ∈ E) : Reach E a b := fwd (refl a) h
theorem edge' {a b : α} (h : (b, a) ∈ E) : Reach E a b := bwd (refl a) h

theorem trans {a b c : α} (h1 : Reach E a b) (h2 : Reach E b c) : Reach E a c := by
  induction h2 with
  | refl => exact h1
  | fwd _ e ih => exact fwd ih e
  | bwd _ e ih => exact bwd ih e

theorem symm {a b : α} (h : Reach E a b) : Reach E b a := by
  induction h with
  | refl => exact refl _
  | fwd _ e ih => exact trans (edge' e) ih
  | bwd _ e ih => exact trans (edge e) ih

theorem mono (hE : ∀ e, e ∈ E → e ∈ E') {a b : α} (h : Reach E a b) : Reach E' a b := by
  induction h with
  | refl => exact refl _
  | fwd _ e ih => exact fwd ih (hE _ e)
  | bwd _ e ih => exact bwd ih (hE _ e)

theorem cons_iff {a b x y : α} :
    Reach ((a, b) :: E) x y ↔
      Reach E x y ∨ (Reach E x a ∧ Reach E b y) ∨ (Reach E x b ∧ Reach E a y) := by
  constructor
  · intro h
    induction h with
    | refl => exact Or.inl (refl _)
    | @fwd u v _ e ih =>
      rcases List.mem_cons.mp e with e | e
      · cases e
        rcases ih with h | ⟨h1, h2⟩ | ⟨h1, _⟩
        · exact Or.inr (Or.inl ⟨h, refl _⟩)
        · exact Or.inl (trans h1 (symm h2))
        · exact Or.inl h1
      · rcases ih with h | ⟨h1, h2⟩ | ⟨h1, h2⟩
        · exact Or.inl (fwd h e)
        · exact Or.inr (Or.inl ⟨h1, fwd h2 e⟩)
        · exact Or.inr (Or.inr ⟨h1, fwd h2 e⟩)
    | @bwd u v _ e ih =>
      rcases List.mem_cons.mp e with e | e
      · cases e
        rcases ih with h | ⟨h1, _⟩ | ⟨h1, h2⟩
        · exact Or.inr (Or.inr ⟨h, refl _⟩)
        · exact Or.inl h1
        · exact Or.inl (trans h1 (symm h2))
      · rcases ih with h | ⟨h1, h2⟩ | ⟨h1, h2⟩
        · exact Or.inl (bwd h e)
        · exact Or.inr (Or.inl ⟨h1, bwd h2 e⟩)
        · exact Or.inr (Or.inr ⟨h1, bwd h2 e⟩)
  · have sub : ∀ e, e ∈ E → e ∈ (a, b) :: E := fun e h => List.mem_cons_of_mem _ h
    have hab : Reach ((a, b) :: E) a b := edge (List.mem_cons_self ..)
    rintro (h | ⟨h1, h2⟩ | ⟨h1, h2⟩)
    · exact mono sub h
    · exact trans (mono sub h1) (trans hab (mono sub h2))
    · exact trans (mono sub h1) (trans (symm hab) (mono sub h2))

theorem nil_iff {x y : α} : Reach ([] : List (α × α)) x y ↔ x = y := by
  constructor
  · intro h
    induction h with
    | refl => rfl
    | fwd _ e _ => simp at e
    | bwd _ e _ => simp at e
  · intro h; subst h; exact refl _

end Reach

def cls {α : Type} (E : List (α × α)) (k : α) : α → Prop := Reach E k

variable {α : Type}

def labOf {α : Type} [DecidableEq α] : List (α × α) → α → α
  | [] => fun x => x
  | (a, b) :: E =>
    let l := labOf E
    let ra := l a
    let rb := l b
    fun x => let lx := l x; if lx = rb then ra else lx

theorem labOf_iff [DecidableEq α] (E : List (α × α)) (x y : α) :
    labOf E x = labOf E y ↔ Reach E x y := by
  induction E generalizing x y with
  | nil => simp only [labOf]; exact Reach.nil_iff.symm
  | cons e E ih =>
    obtain ⟨a, b⟩ := e
    rw [Reach.cons_iff]
    simp only [labOf]
    have hxb := ih x b
    have hya := ih y a
    have hxa := ih x a
    have hyb := ih y b
    have hxy := ih x y
    have sy : ∀ u v, Reach E u v ↔ Reach E v u := fun u v => ⟨Reach.symm, Reach.symm⟩
    by_cases h1 : labOf E x = labOf E b <;> by_cases h2 : labOf E y = labOf E b
    ·
      rw [if_pos h1, if_pos h2]
      have rx := hxb.mp h1; have ry := hyb.mp h2
      exact ⟨fun _ => Or.inl (Reach.trans rx (Reach.symm ry)), fun _ => rfl⟩
    ·
      rw [if_pos h1, if_neg h2]
      constructor
      · intro e
        exact Or.inr (Or.inr ⟨hxb.mp h1, (sy _ _).mp (hya.mp e.symm)⟩)
      · rintro (r | ⟨_, r2⟩ | ⟨_, r2⟩)
        · exact absurd ((hxy.mpr r).symm.trans h1) h2
        · exact absurd ((ih y b).mpr (Reach.symm r2)) h2
        · exact (hya.mpr ((sy _ _).mp r2)).symm
    ·
      rw [if_neg h1, if_pos h2]
      constructor
      · intro e
        exact Or.inr (Or.inl ⟨hxa.mp e, (sy _ _).mp (hyb.mp h2)⟩)
      · rintro (r | ⟨r1, _⟩ | ⟨r1, _⟩)
        · exact absurd ((hxy.mpr r).trans h2) h1
        · exact hxa.mpr r1
        · exact absurd (hxb.mpr r1) h1
    ·
      rw [if_neg h1, if_neg h2]
      constructor
      · intro e; exact Or.inl (hxy.mp e)
      · rintro (r | ⟨_, r2⟩ | ⟨r1, _⟩)
        · exact hxy.mpr r
        · exact absurd (hyb.mpr ((sy _ _).mp r2)) h2
        · exact absurd (hxb.mpr r1) h1

def sameCls {α : Type} [DecidableEq α] (E : List (α × α)) (x y : α) : Bool := decide (labOf E x = labOf E y)

theorem sameCls_iff [DecidableEq α] (E : List (α × α)) (x y : α) :
    sameCls E x y = true ↔ cls E x y := by
  unfold sameCls cls; rw [decide_eq_true_iff]; exact labOf_iff E x y

end CM
