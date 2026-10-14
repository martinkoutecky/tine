import Storage3
namespace Storage3
open Storage21
set_option maxRecDepth 8192
set_option maxHeartbeats 4000000

theorem mem_union_int (a b : List Int) (t : Int) :
    t ∈ union a b ↔ t ∈ a ∨ t ∈ b := by
  induction b generalizing a with
  | nil => simp [union]
  | cons v vs ih =>
    simp only [union,List.foldl_cons]
    change t ∈ union (if a.contains v then a else a ++ [v]) vs ↔ _
    rw [ih]
    split <;> simp_all <;> grind

def TrashInv (x : State P) : Prop :=
  (∀ p b, b ∈ x.g.removed p → b ∈ x.s.trash p) ∧
  (∀ p b, b ∈ x.g.delDurable p → b ∈ x.s.trashStable p) ∧
  (∀ p b, b ∈ x.s.trashStable p → b ∈ x.s.trash p)

@[simp] theorem trash_commit (c : Config P) (x : State P) (s : Sys P) (g : Ghost P) (n : String) :
    TrashInv (commit c x s g n) ↔ TrashInv ⟨s,g⟩ := Iff.rfl
@[simp] theorem trash_lift (x : State P) (y : Storage21.State P) : TrashInv (lift x y) ↔ TrashInv x := Iff.rfl

theorem trash_rename (c : Config P) (x : State P) (mutant : c.mutant = "none")
    (hx : TrashInv x) : TrashInv (rename c x) := by
  by_cases hd : x.s.job.bytes = absent ∧ x.s.disk x.s.job.p ≠ absent
  · have del : (x.s.job.bytes == absent && x.s.disk x.s.job.p != absent) = true := by simp [hd.1,hd.2]
    simp only [rename,del]
    simp only [mutant,show ("none" != "MDT") = true by decide,Bool.true_and,ite_true,trash_commit]
    refine ⟨?_,?_,?_⟩
    · intro p b hb
      by_cases hp : p = x.s.job.p
      · subst p; simp only [put,ite_true] at hb ⊢
        rcases (mem_union_int _ _ _).mp hb with old | fresh
        · exact (mem_union_int _ _ _).mpr (Or.inl (hx.1 _ _ old))
        · exact (mem_union_int _ _ _).mpr (Or.inr fresh)
      · simpa [put,hp] using hx.1 p b (by simpa [put,hp] using hb)
    · exact hx.2.1
    · intro p b hb
      by_cases hp : p = x.s.job.p
      · subst p; simp only [put,ite_true]
        exact (mem_union_int _ _ _).mpr (Or.inl (hx.2.2 _ _ hb))
      · simpa [put,hp] using hx.2.2 p b hb
  · have del : (x.s.job.bytes == absent && x.s.disk x.s.job.p != absent) = false := by simp_all
    simpa [rename,del,commit,TrashInv] using hx

theorem trash_dirSync (c : Config P) (x : State P) (ok : Bool) (mutant : c.mutant = "none")
    (hx : TrashInv x) : TrashInv (dirSync c x ok) := by
  by_cases dur : (ok && !c.weak && x.s.disk x.s.job.p == x.s.job.bytes) = true
  · by_cases del : x.s.job.bytes = absent
    · simp only [dirSync,dur]
      simp only [mutant,show ("none" != "MTS") = true by decide,show (x.s.job.bytes == absent) = true by simp [del],Bool.true_and,ite_true]
      refine ⟨?_,?_,?_⟩
      · exact hx.1
      · intro p b hb
        by_cases hp : p = x.s.job.p
        · subst p; simp only [put,ite_true] at hb ⊢; exact hx.1 _ _ hb
        · simpa [put,hp,lift] using hx.2.1 p b (by simpa [put,hp,lift] using hb)
      · intro p b hb
        by_cases hp : p = x.s.job.p
        · subst p; simpa [put,lift] using hb
        · exact hx.2.2 p b (by simpa [put,hp,lift] using hb)
    · simp only [dirSync,dur]
      simp only [mutant,show ("none" != "MTS") = true by decide,show (x.s.job.bytes == absent) = false by simp [del],Bool.true_and,ite_true,Bool.false_eq_true,ite_false]
      refine ⟨?_,?_,?_⟩
      · exact hx.1
      · intro p b hb
        by_cases hp : p = x.s.job.p
        · subst p; simp only [put,ite_true]; exact hx.2.2 _ _ (hx.2.1 _ _ hb)
        · simpa [put,hp,lift] using hx.2.1 p b hb
      · intro p b hb
        by_cases hp : p = x.s.job.p
        · subst p; simpa [put,lift] using hb
        · exact hx.2.2 p b (by simpa [put,hp,lift] using hb)
  · have durF : (ok && !c.weak && x.s.disk x.s.job.p == x.s.job.bytes) = false := by simp_all
    simpa [dirSync,durF,TrashInv,lift] using hx

theorem trash_power (c : Config P) (x : State P) (keep : Fin P → Bool)
    (hx : TrashInv x) : TrashInv (power c x keep) := by
  refine ⟨?_,?_,?_⟩
  · intro p b hb
    cases hk : keep p
    · simpa [power,hk] using hx.2.1 p b (by simpa [power,hk] using hb)
    · simpa [power,hk] using hx.1 p b (by simpa [power,hk] using hb)
  · intro p b hb
    have old : b ∈ x.g.delDurable p := by
      have hf : b ∈ (x.g.delDurable p).filter
        ((fun q => if keep q then x.g.removed q else x.g.delDurable q) p).contains := hb
      exact (List.mem_filter.mp hf).1
    cases hk : keep p
    · simpa [power,hk] using hx.2.1 p b old
    · simpa [power,hk] using hx.2.2 p b (hx.2.1 p b old)
  · intro p b hb; exact hb

theorem trash_execute (c : Config P) (x : State P) (a : Action P) (mutant : c.mutant = "none")
    (hx : TrashInv x) : TrashInv (execute c x a) := by
  cases a with
  | old a =>
    cases a <;> simp only [execute,executeOld]
    case rename => exact trash_rename c x mutant hx
    case dirSync ok => exact trash_dirSync c x ok mutant hx
    case power keep => exact trash_power c x keep hx
    case deliverUp ok => simpa [mutant] using hx
    all_goals exact hx
  | opDelete p => exact hx
  | opRename src dst refs rt => exact hx
  | flushDel p => exact hx

theorem trash_reachable (c : Config P) (disk : Fin P → Int) (x : State P)
    (mutant : c.mutant = "none") (hr : Reachable c disk x) : TrashInv x := by
  induction hr with
  | initial => refine ⟨?_,?_,?_⟩ <;> simp [initWith,embed]
  | @advance x y hr a hs ih =>
    unfold step at hs; split at hs
    · cases hs; exact trash_execute c x a mutant ih
    · contradiction

theorem trashed_reachable (c : Config P) (disk : Fin P → Int) (x : State P)
    (mutant : c.mutant = "none") (hr : Reachable c disk x) : trashed x = true := by
  have hi := trash_reachable c disk x mutant hr
  apply List.all_eq_true.mpr
  intro p hp
  apply List.all_eq_true.mpr
  intro b hb
  simp only [Bool.or_eq_true,List.contains_iff_mem]
  exact Or.inl (hi.1 p b hb)
end Storage3
