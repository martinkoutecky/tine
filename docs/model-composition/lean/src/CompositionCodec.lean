import ListedDisk
import Storage33

namespace ModelComposition
open ListedDisk Storage21

private def distinct [BEq α] (xs : List α) : List α :=
  xs.foldr (fun x acc => acc.insert x) []

private theorem mem_distinct [BEq α] [LawfulBEq α] {x : α} {xs : List α} :
    x ∈ distinct xs ↔ x ∈ xs := by
  induction xs with
  | nil => simp [distinct]
  | cons y ys ih =>
    change x ∈ (distinct ys).insert y ↔ x ∈ y :: ys
    simp [ih]

private theorem nodup_distinct [BEq α] [LawfulBEq α] (xs : List α) :
    (distinct xs).Nodup := by
  induction xs with
  | nil => simp [distinct]
  | cons y ys ih =>
    change ((distinct ys).insert y).Nodup
    by_cases h : y ∈ distinct ys <;> simp [List.insert, h, ih]

structure Codec where
  paths : List Path
  uniquePaths : paths.Nodup
  payloads : List Bytes

namespace Codec

def forPatch (image : Image) (patch : Patch) : Codec :=
  let paths := image.toList.map Prod.fst ++ patch.effects.toList.map Prod.fst ++
    patch.bindings.flatMap (fun b => [b.origin, b.destination])
  let payloads := image.toList.map Prod.snd ++ patch.effects.toList.flatMap
    (fun (_, e) => e.before.toList ++ e.after.toList)
  ⟨distinct paths, nodup_distinct paths, distinct ([] :: payloads)⟩

abbrev Slot (k : Codec) := Fin k.paths.length

def path (k : Codec) (p : k.Slot) : Path := k.paths[p]

def slot? (k : Codec) (path : Path) : Option k.Slot := k.paths.finIdxOf? path

theorem slot_spec {k : Codec} {path : Path} {p : k.Slot} (h : k.slot? path = some p) :
    k.path p = path := (List.finIdxOf?_eq_some_iff.mp h).1

theorem path_injective (k : Codec) : Function.Injective k.path := by
  intro a b eq
  apply Fin.ext
  exact (List.getElem_inj k.uniquePaths).mp eq

private theorem slot_exists {k : Codec} {path : Path} (mem : path ∈ k.paths) :
    ∃ p, k.slot? path = some p := by
  cases h : k.slot? path with
  | some p => exact ⟨p, rfl⟩
  | none => exact False.elim ((List.finIdxOf?_eq_none_iff.mp h) mem)

theorem slot_path (k : Codec) (p : k.Slot) : k.slot? (k.path p) = some p := by
  obtain ⟨q, h⟩ := slot_exists (k := k) (path := k.path p) (List.mem_of_getElem rfl)
  rw [h, k.path_injective (slot_spec h)]

theorem slot_of_image {image : Image} {patch : Patch} {path : Path} {b : Bytes}
    (he : image[path]? = some b) : ∃ p, (forPatch image patch).slot? path = some p := by
  apply slot_exists
  simp only [forPatch, mem_distinct, List.mem_append]
  exact Or.inl (Or.inl (List.mem_map.mpr ⟨_, Std.ExtTreeMap.mem_toList_iff_getElem?_eq_some.mpr he, rfl⟩))

theorem slot_of_effect {image : Image} {patch : Patch} {path : Path} {e : Effect}
    (he : patch.effects[path]? = some e) : ∃ p, (forPatch image patch).slot? path = some p := by
  apply slot_exists
  simp only [forPatch, mem_distinct, List.mem_append]
  exact Or.inl (Or.inr (List.mem_map.mpr ⟨_, Std.ExtTreeMap.mem_toList_iff_getElem?_eq_some.mpr he, rfl⟩))

def Supports (k : Codec) (entry : Entry) : Prop := ∀ b ∈ entry.toList, b ∈ k.payloads

theorem image_supported {image : Image} {patch : Patch} (path : Path) :
    (forPatch image patch).Supports image[path]? := by
  cases h : image[path]? with
  | none => simp [Supports]
  | some b =>
    have pair : (path, b) ∈ image.toList := Std.ExtTreeMap.mem_toList_iff_getElem?_eq_some.mpr h
    intro raw hr
    have eq : raw = b := by simpa using hr
    subst raw
    simp only [forPatch, mem_distinct, List.mem_cons, List.mem_append]
    exact Or.inr (Or.inl (List.mem_map.mpr ⟨_, pair, rfl⟩))

theorem effect_supported {image : Image} {patch : Patch} {path : Path} {e : Effect}
    (he : patch.effects[path]? = some e) :
    (forPatch image patch).Supports e.before ∧ (forPatch image patch).Supports e.after := by
  have pair : (path, e) ∈ patch.effects.toList := Std.ExtTreeMap.mem_toList_iff_getElem?_eq_some.mpr he
  have both : ∀ b ∈ e.before.toList ++ e.after.toList, b ∈ (forPatch image patch).payloads := by
    intro b hb
    simp only [forPatch, mem_distinct, List.mem_cons, List.mem_append, List.mem_flatMap]
    exact Or.inr (Or.inr ⟨(path, e), pair, List.mem_append.mp hb⟩)
  exact ⟨fun b hb => both b (List.mem_append_left _ hb), fun b hb => both b (List.mem_append_right _ hb)⟩

def encode (k : Codec) : Entry → Int
  | none => absent
  | some b => Int.ofNat ((k.payloads.idxOf? b).getD k.payloads.length)

def decode (k : Codec) (label : Int) : Option Entry :=
  if label = absent then some none
  else if 0 ≤ label then k.payloads[label.toNat]?.map some else none

theorem encode_absent_iff (k : Codec) (entry : Entry) :
    k.encode entry = absent ↔ entry = none := by
  cases entry <;> simp [encode, absent]

theorem decode_encode {k : Codec} {entry : Entry} (supported : k.Supports entry) :
    k.decode (k.encode entry) = some entry := by
  cases entry with
  | none => simp [encode, decode]
  | some b =>
    have mem : b ∈ k.payloads := supported b (by simp)
    cases h : k.payloads.idxOf? b with
    | none => exact False.elim ((List.idxOf?_eq_none_iff.mp h) mem)
    | some i =>
      obtain ⟨bound, entry, _⟩ := List.idxOf?_eq_some_iff.mp h
      simp [decode, encode, absent, h, List.getElem?_eq_getElem bound, entry]

theorem encode_eq_iff {k : Codec} {a b : Entry} (ha : k.Supports a) (hb : k.Supports b) :
    k.encode a = k.encode b ↔ a = b := by
  constructor
  · intro h
    exact Option.some.inj ((decode_encode ha).symm.trans ((congrArg k.decode h).trans (decode_encode hb)))
  · rintro rfl
    rfl

def encodeImage (k : Codec) (image : Image) (p : k.Slot) : Int := k.encode image[k.path p]?

theorem decode_image (image : Image) (patch : Patch) (p : (forPatch image patch).Slot) :
    let k := forPatch image patch
    k.decode (k.encodeImage image p) = some image[k.path p]? :=
  decode_encode (image_supported _)

def texts (k : Codec) : List Int := (List.range k.payloads.length).map Int.ofNat

theorem encode_real {k : Codec} {entry : Entry} (h : k.Supports entry) :
    k.encode entry = absent ∨ k.encode entry ∈ k.texts := by
  cases entry with
  | none => exact Or.inl rfl
  | some b =>
    cases eq : k.payloads.idxOf? b with
    | none => exact False.elim ((List.idxOf?_eq_none_iff.mp eq) (h b (by simp)))
    | some i =>
      have bound := (List.idxOf?_eq_some_iff.mp eq).1
      exact Or.inr (List.mem_map.mpr ⟨i, List.mem_range.mpr bound, by simp [encode, eq]⟩)

theorem text_nonnegative {k : Codec} {label : Int} (h : label ∈ k.texts) : 0 ≤ label := by
  obtain ⟨i, _, rfl⟩ := List.mem_map.mp h
  exact Int.natCast_nonneg i

theorem texts_reserve_sentinels (k : Codec) :
    absent ∉ k.texts ∧ noneBytes ∉ k.texts ∧ unknown ∉ k.texts := by
  exact ⟨fun h => (by decide : ¬0 ≤ absent) (text_nonnegative h),
    fun h => (by decide : ¬0 ≤ noneBytes) (text_nonnegative h),
    fun h => (by decide : ¬0 ≤ unknown) (text_nonnegative h)⟩

theorem decode_rejects_sentinels (k : Codec) : k.decode noneBytes = none ∧ k.decode unknown = none := by
  simp [decode, absent, noneBytes, unknown]

#print axioms slot_path
#print axioms encode_eq_iff

end Codec
end ModelComposition
