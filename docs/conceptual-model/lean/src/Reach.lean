/-
Alias classes (SPEC §4.2) as connected components of an undirected graph given by a finite edge list,
and the T2 facts. Nothing here is specific to Tine: the edge list is whatever `aliasEdges` produces.

`Reach E a b` says there is a walk from `a` to `b` using edges of `E` in either direction. This is the
definition of "same connected component"; that it is an equivalence relation is *proved* (T2), not
assumed. `labOf E` is an executable union-by-relabelling that decides `Reach E`; its correctness is
proved from the edge-insertion lemma `reach_cons_iff`, which is also T2's "adding an edge merges at
most two classes".
-/
namespace CM

/-- A walk in the undirected graph with edge list `E` (each pair is one undirected edge). -/
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

/-- Monotone in the edge set. -/
theorem mono (hE : ∀ e, e ∈ E → e ∈ E') {a b : α} (h : Reach E a b) : Reach E' a b := by
  induction h with
  | refl => exact refl _
  | fwd _ e ih => exact fwd ih (hE _ e)
  | bwd _ e ih => exact bwd ih (hE _ e)

/-- Depends on the edge list only through its members. -/
theorem congr (hE : ∀ e, e ∈ E ↔ e ∈ E') {a b : α} : Reach E a b ↔ Reach E' a b :=
  ⟨mono (fun e h => (hE e).mp h), mono (fun e h => (hE e).mpr h)⟩

/-- **Edge insertion** (the heart of T2): a walk using the new edge `(a, b)` either avoids it or
passes from `a`'s old component to `b`'s, or back. -/
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

/-- With no edges, every key is alone. -/
theorem nil_iff {x y : α} : Reach ([] : List (α × α)) x y ↔ x = y := by
  constructor
  · intro h
    induction h with
    | refl => rfl
    | fwd _ e _ => simp at e
    | bwd _ e _ => simp at e
  · intro h; subst h; exact refl _

end Reach

/-! ## T2: classes partition the keys; edge insertion and deletion -/

/-- The alias class of `k`: the members of its connected component. -/
def cls {α : Type} (E : List (α × α)) (k : α) : α → Prop := Reach E k

variable {α : Type}

/-- T2 (partition, 1): every key is in its own class. -/
theorem cls_self (E : List (α × α)) (k : α) : cls E k k := Reach.refl k

/-- T2 (partition, 2): two classes that share a member are equal. So the classes are the blocks of a
partition of the keys: each key lies in exactly one class (`cls E k`), and `cls E k = cls E k'` iff
`k'` is in `cls E k`. -/
theorem cls_eq_of_common (E : List (α × α)) {k k' z : α} (h : cls E k z) (h' : cls E k' z) :
    cls E k = cls E k' := by
  funext y
  apply propext
  exact ⟨fun hy => Reach.trans (Reach.trans h' (Reach.symm h)) hy,
         fun hy => Reach.trans (Reach.trans h (Reach.symm h')) hy⟩

theorem cls_eq_iff (E : List (α × α)) {k k' : α} : cls E k = cls E k' ↔ cls E k k' := by
  constructor
  · intro h; rw [h]; exact cls_self E k'
  · intro h; exact cls_eq_of_common E h (cls_self E k')

/-- A key that no edge mentions is its own singleton class. -/
theorem cls_isolated {E : List (α × α)} {k : α} (h : ∀ e ∈ E, e.1 ≠ k ∧ e.2 ≠ k) (y : α) :
    cls E k y ↔ y = k := by
  constructor
  · intro hy
    induction hy with
    | refl => rfl
    | fwd _ e ih => subst ih; exact absurd rfl (h _ e).1
    | bwd _ e ih => subst ih; exact absurd rfl (h _ e).2
  · intro hy; subst hy; exact cls_self E y

/-- T2 (insertion never splits): a class before the insertion is contained in a class after it. -/
theorem cls_insert_mono (E : List (α × α)) (e : α × α) {x y : α} :
    cls E x y → cls (e :: E) x y :=
  Reach.mono (fun _ h => List.mem_cons_of_mem _ h)

/-- T2 (insertion merges at most two classes): after inserting the edge `(a, b)`, a key outside the
old classes of `a` and `b` keeps exactly its class ... -/
theorem cls_insert_other (E : List (α × α)) {a b x : α} (ha : ¬ cls E x a) (hb : ¬ cls E x b) :
    cls ((a, b) :: E) x = cls E x := by
  funext y; apply propext
  rw [cls, Reach.cons_iff]
  constructor
  · rintro (h | ⟨h1, _⟩ | ⟨h1, _⟩)
    · exact h
    · exact absurd h1 ha
    · exact absurd h1 hb
  · exact Or.inl

/-- ... and a key in either of them gets the union of the two. -/
theorem cls_insert_merged (E : List (α × α)) {a b x : α} (hx : cls E x a ∨ cls E x b) :
    cls ((a, b) :: E) x = fun y => cls E a y ∨ cls E b y := by
  funext y; apply propext
  rw [cls, Reach.cons_iff]
  unfold cls
  constructor
  · rintro (h | ⟨_, h2⟩ | ⟨_, h2⟩)
    · rcases hx with hx | hx
      · exact Or.inl (Reach.trans (Reach.symm hx) h)
      · exact Or.inr (Reach.trans (Reach.symm hx) h)
    · exact Or.inr h2
    · exact Or.inl h2
  · rintro (h | h)
    · rcases hx with hx | hx
      · exact Or.inl (Reach.trans hx h)
      · exact Or.inr (Or.inr ⟨hx, h⟩)
    · rcases hx with hx | hx
      · exact Or.inr (Or.inl ⟨hx, h⟩)
      · exact Or.inl (Reach.trans hx h)

/-- Removing an edge, stated for any edge list `E` whose members are those of `E₀` plus `(a, b)`
(so it covers removing one copy from anywhere in the list). -/
def RemovesEdge (E E₀ : List (α × α)) (a b : α) : Prop := ∀ e, e ∈ E ↔ e = (a, b) ∨ e ∈ E₀

theorem RemovesEdge.reach_iff {E E₀ : List (α × α)} {a b : α} (h : RemovesEdge E E₀ a b)
    {x y : α} : Reach E x y ↔ Reach ((a, b) :: E₀) x y :=
  Reach.congr (fun e => by rw [h e, List.mem_cons])

/-- T2 (removal never merges): a class after removing an edge is inside a class before it. -/
theorem cls_remove_sub {E E₀ : List (α × α)} {a b : α} (h : RemovesEdge E E₀ a b) {x y : α} :
    cls E₀ x y → cls E x y := fun hy =>
  (h.reach_iff).mpr (cls_insert_mono E₀ (a, b) hy)

/-- T2 (removal changes no other class): a key outside the class of the removed edge keeps its class. -/
theorem cls_remove_other {E E₀ : List (α × α)} {a b : α} (h : RemovesEdge E E₀ a b) {x : α}
    (hx : ¬ cls E x a) : cls E₀ x = cls E x := by
  have hab : Reach E a b := (h.reach_iff).mpr (Reach.edge (List.mem_cons_self ..))
  have ha0 : ¬ cls E₀ x a := fun h0 => hx (cls_remove_sub h h0)
  have hb0 : ¬ cls E₀ x b := fun h0 => hx (Reach.trans (cls_remove_sub h h0) (Reach.symm hab))
  have := cls_insert_other E₀ ha0 hb0
  funext y; apply propext
  rw [← this]; unfold cls
  exact (h.reach_iff).symm

/-- T2 (removal splits at most in two): every member of the removed edge's old class is, afterwards,
in the class of `a` or in the class of `b`. -/
theorem cls_remove_split {E E₀ : List (α × α)} {a b : α} (h : RemovesEdge E E₀ a b) {y : α}
    (hy : cls E a y) : cls E₀ a y ∨ cls E₀ b y := by
  have := (h.reach_iff).mp hy
  rcases Reach.cons_iff.mp this with h1 | ⟨_, h2⟩ | ⟨_, h2⟩
  · exact Or.inl h1
  · exact Or.inr h2
  · exact Or.inl h2

/-! ## An executable decision procedure for `Reach` -/

/-- Union by relabelling: after the edges `E`, `labOf E x` is a representative of `x`'s class.
Each edge `(a, b)` relabels every key whose label is `b`'s label to `a`'s label. -/
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
    · -- both relabelled
      rw [if_pos h1, if_pos h2]
      have rx := hxb.mp h1; have ry := hyb.mp h2
      exact ⟨fun _ => Or.inl (Reach.trans rx (Reach.symm ry)), fun _ => rfl⟩
    · -- x relabelled, y not
      rw [if_pos h1, if_neg h2]
      constructor
      · intro e
        exact Or.inr (Or.inr ⟨hxb.mp h1, (sy _ _).mp (hya.mp e.symm)⟩)
      · rintro (r | ⟨_, r2⟩ | ⟨_, r2⟩)
        · exact absurd ((hxy.mpr r).symm.trans h1) h2
        · exact absurd ((ih y b).mpr (Reach.symm r2)) h2
        · exact (hya.mpr ((sy _ _).mp r2)).symm
    · -- y relabelled, x not
      rw [if_neg h1, if_pos h2]
      constructor
      · intro e
        exact Or.inr (Or.inl ⟨hxa.mp e, (sy _ _).mp (hyb.mp h2)⟩)
      · rintro (r | ⟨r1, _⟩ | ⟨r1, _⟩)
        · exact absurd ((hxy.mpr r).trans h2) h1
        · exact hxa.mpr r1
        · exact absurd (hxb.mpr r1) h1
    · -- neither relabelled
      rw [if_neg h1, if_neg h2]
      constructor
      · intro e; exact Or.inl (hxy.mp e)
      · rintro (r | ⟨_, r2⟩ | ⟨r1, _⟩)
        · exact hxy.mpr r
        · exact absurd (hyb.mpr ((sy _ _).mp r2)) h2
        · exact absurd (hxb.mpr r1) h1

/-- Executable decision of "same alias class". -/
def sameCls {α : Type} [DecidableEq α] (E : List (α × α)) (x y : α) : Bool := decide (labOf E x = labOf E y)

theorem sameCls_iff [DecidableEq α] (E : List (α × α)) (x y : α) :
    sameCls E x y = true ↔ cls E x y := by
  unfold sameCls cls; rw [decide_eq_true_iff]; exact labOf_iff E x y

end CM
