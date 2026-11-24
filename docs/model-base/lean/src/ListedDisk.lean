import Std.Data.ExtTreeMap

namespace ListedDisk

abbrev Path := String
abbrev Bytes := List UInt8
abbrev Entry := Option Bytes
abbrev Image := Std.ExtTreeMap Path Bytes

structure Effect where
  before : Entry
  after : Entry
  deriving DecidableEq, Repr

structure Binding where
  origin : Path
  destination : Path
  deriving DecidableEq, Repr

structure Patch where
  effects : Std.ExtTreeMap Path Effect
  bindings : List Binding
  deriving Repr

/-- Apply the recorded after entries as a pure overlay. -/
def Patch.replay (p : Patch) (image : Image) : Image :=
  p.effects.foldl (fun disk path effect => disk.alter path (fun _ => effect.after)) image

theorem Patch.replay_lookup (p : Patch) (image : Image) (path : Path) :
    (p.replay image)[path]? = ((p.effects[path]?).map Effect.after).getD image[path]? := by
  let applyEffect := fun (disk : Image) (e : Path × Effect) => disk.alter e.1 (fun _ => e.2.after)
  let after := ((p.effects[path]?).map Effect.after).getD none
  have fold_lookup (effects : List (Path × Effect))
      (valid : ∀ e ∈ effects, p.effects[e.1]? = some e.2) (disk : Image) :
      (effects.foldl applyEffect disk)[path]? =
        if effects.any (fun e => e.1 == path) then after else disk[path]? := by
    induction effects generalizing disk with
    | nil => rfl
    | cons e rest ih =>
      change (rest.foldl applyEffect (applyEffect disk e))[path]? = _
      rw [ih (fun a ha => valid a (List.mem_cons_of_mem _ ha))]
      by_cases h : e.1 = path
      · have he := valid e (List.mem_cons_self ..)
        rw [h] at he
        simp [applyEffect, h, after, he]
      · have notSame : (e.1 == path) = false := by simpa using h
        simp only [List.any_cons, notSame, Bool.false_or]
        simp [applyEffect, Std.ExtTreeMap.getElem?_alter, h]
  have listed : ∀ e ∈ p.effects.toList, p.effects[e.1]? = some e.2 :=
    fun e he => Std.ExtTreeMap.mem_toList_iff_getElem?_eq_some.mp he
  have present : (p.effects.toList.any (fun e => e.1 == path)) = (p.effects[path]?).isSome := by
    apply Bool.eq_iff_iff.mpr
    simp only [List.any_eq_true, beq_iff_eq, Option.isSome_iff_exists]
    constructor
    · rintro ⟨e, he, hp⟩
      exact ⟨e.2, hp ▸ listed e he⟩
    · rintro ⟨e, he⟩
      exact ⟨(path, e), Std.ExtTreeMap.mem_toList_iff_getElem?_eq_some.mpr he, rfl⟩
  rw [Patch.replay, Std.ExtTreeMap.foldl_eq_foldl_toList, fold_lookup _ listed, present]
  cases h : p.effects[path]? <;> simp [h, after]

end ListedDisk
