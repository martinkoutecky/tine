import CompositionAdmission
import Stage6Admission

namespace ModelComposition
open ListedDisk Storage21 CM CM.Cut

/-- A cross-path receiver that already contains its final bytes still participates
in publication order. Only present entries can supply this extra write. -/
def physicalWrite (image : Image) (patch : Patch) (path : Path) : Option Effect :=
  match patch.effects[path]? with
  | some e => some e
  | none =>
    if (crossBindings patch).any (fun b => b.destination == path) then
      image[path]?.map (fun raw => ⟨some raw, some raw⟩)
    else none

theorem physicalWrite_iff (image : Image) (patch : Patch) (path : Path) :
    (physicalWrite image patch path).isSome = true ↔
      (patch.effects[path]?).isSome = true ∨
        ((∃ b ∈ crossBindings patch, b.destination = path) ∧ image[path]?.isSome = true) := by
  cases h : patch.effects[path]? with
  | some e => simp [physicalWrite, h]
  | none =>
    have receiver : ((crossBindings patch).any fun b => b.destination == path) = true ↔
        ∃ b ∈ crossBindings patch, b.destination = path := by simp [List.any_eq_true]
    simp only [physicalWrite, h, Option.isSome_none, Bool.false_eq_true, false_or]
    rw [← receiver]
    split <;> simp_all

theorem physicalWrite_slot {image : Image} {patch : Patch} {path : Path} {e : Effect}
    (write : physicalWrite image patch path = some e) :
    ∃ p, (Codec.forPatch image patch).slot? path = some p := by
  have member := (physicalWrite_iff image patch path).mp (by rw [write]; rfl)
  rcases member with effect | ⟨_, present⟩
  · obtain ⟨_, h⟩ := Option.isSome_iff_exists.mp effect
    exact Codec.slot_of_effect h
  · obtain ⟨_, h⟩ := Option.isSome_iff_exists.mp present
    exact Codec.slot_of_image h

theorem physicalWrite_supported {image : Image} {patch : Patch} {path : Path} {e : Effect}
    (write : physicalWrite image patch path = some e) :
    (Codec.forPatch image patch).Supports e.before ∧ (Codec.forPatch image patch).Supports e.after := by
  unfold physicalWrite at write
  split at write
  · rename_i h
    exact Codec.effect_supported (h.trans write)
  · split at write
    · cases h : image[path]? with
      | none => simp [h] at write
      | some raw =>
        have eq : (⟨some raw, some raw⟩ : Effect) = e := by simpa [h] using write
        subst e
        have supported := Codec.image_supported (image := image) (patch := patch) path
        rw [h] at supported
        exact ⟨supported, supported⟩
    · contradiction

private def encodeMove (k : Codec) (b : Binding) : Option (Stage6.Move k.paths.length) :=
  return ⟨← k.slot? b.destination, ← k.slot? b.origin⟩

private theorem encodeMoves_paths (k : Codec) (bs : List Binding) (ms : List (Stage6.Move k.paths.length))
    (encoded : bs.mapM (encodeMove k) = some ms) :
    ms.map (fun m => (⟨k.path m.src, k.path m.dst⟩ : Binding)) = bs := by
  induction bs generalizing ms with
  | nil => simpa using encoded.symm
  | cons b bs ih =>
    cases hd : k.slot? b.destination with
    | none => simp [List.mapM_cons, encodeMove, hd] at encoded
    | some dst =>
      cases hs : k.slot? b.origin with
      | none => simp [List.mapM_cons, encodeMove, hd, hs] at encoded
      | some src =>
        cases tail : bs.mapM (encodeMove k) with
        | none => simp [List.mapM_cons, encodeMove, hd, hs, tail] at encoded
        | some moves =>
          have eq : ⟨dst, src⟩ :: moves = ms := by
            simpa [List.mapM_cons, encodeMove, hd, hs, tail] using encoded
          subst ms
          cases b
          simp [Codec.slot_spec hs, Codec.slot_spec hd, ih moves tail]

private theorem encodeMoves_exists (k : Codec) (bs : List Binding)
    (slots : ∀ b ∈ bs, (∃ s, k.slot? b.origin = some s) ∧ (∃ d, k.slot? b.destination = some d)) :
    ∃ ms, bs.mapM (encodeMove k) = some ms := by
  induction bs with
  | nil => exact ⟨[], rfl⟩
  | cons b bs ih =>
    obtain ⟨⟨s, hs⟩, ⟨d, hd⟩⟩ := slots b (by simp)
    obtain ⟨ms, hm⟩ := ih (fun k hk => slots k (List.mem_cons_of_mem _ hk))
    exact ⟨⟨d, s⟩ :: ms, by simp [List.mapM_cons, encodeMove, hs, hd, hm]⟩

private def batchWithMoves (image : Image) (patch : Patch)
    (moves : List (Stage6.Move (Codec.forPatch image patch).paths.length)) :
    Stage6.Batch (Codec.forPatch image patch).paths.length :=
  let k := Codec.forPatch image patch
  let write := fun p => physicalWrite image patch (k.path p)
  ⟨fun p => (write p).map (fun e => k.encode e.after),
    fun p => (write p).map (fun e => k.encode e.before), moves⟩

/-- Encode a finite listed patch in a fixed physical slot space. Fail on aliasing,
an empty write set, or a move layout the batch storage protocol cannot admit. -/
def encodeBatch (slotKey : Path → String) (image : Image) (patch : Patch) :
    Option (Stage6.Batch (Codec.forPatch image patch).paths.length) :=
  let k := Codec.forPatch image patch
  match (crossBindings patch).mapM (encodeMove k) with
  | none => none
  | some moves =>
    let b := batchWithMoves image patch moves
    if (k.paths.map slotKey).Nodup ∧ b.Shape then some b else none

theorem encodeBatch_spec {slotKey : Path → String} {image : Image} {patch : Patch}
    {b : Stage6.Batch (Codec.forPatch image patch).paths.length}
    (encoded : encodeBatch slotKey image patch = some b) :
    let k := Codec.forPatch image patch
    (k.paths.map slotKey).Nodup ∧ b.Shape ∧
    b.moves.map (fun m => (⟨k.path m.src, k.path m.dst⟩ : Binding)) = crossBindings patch ∧
    ∀ p, b.plan p = (physicalWrite image patch (k.path p)).map (fun e => k.encode e.before) ∧
      b.eff p = (physicalWrite image patch (k.path p)).map (fun e => k.encode e.after) := by
  cases moves : (crossBindings patch).mapM (encodeMove (Codec.forPatch image patch)) with
  | none => simp [encodeBatch, moves] at encoded
  | some ms =>
    simp only [encodeBatch, moves] at encoded
    split at encoded
    · rename_i fits
      cases Option.some.inj encoded
      exact ⟨fits.1, fits.2, encodeMoves_paths _ _ _ moves, fun _ => ⟨rfl, rfl⟩⟩
    · contradiction

theorem encodeBatch_keys {slotKey : Path → String} {image : Image} {patch : Patch}
    {b : Stage6.Batch (Codec.forPatch image patch).paths.length}
    (encoded : encodeBatch slotKey image patch = some b) (p : (Codec.forPatch image patch).Slot) :
    p ∈ b.keys ↔ (physicalWrite image patch ((Codec.forPatch image patch).path p)).isSome = true := by
  rw [Stage6.keys_iff, ((encodeBatch_spec encoded).2.2.2 p).2, Option.isSome_map]

/-- A path-level criterion for encoding a batch of any finite size. -/
theorem encodeBatch_exists (slotKey : Path → String) (image : Image) (patch : Patch)
    (physical : ((Codec.forPatch image patch).paths.map slotKey).Nodup)
    (nonempty : ∃ path e, physicalWrite image patch path = some e)
    (disjoint : ((crossBindings patch).flatMap fun b => [b.destination, b.origin]).Nodup)
    (moves : ∀ b ∈ crossBindings patch, ∃ old new before,
      physicalWrite image patch b.origin = some ⟨some old, none⟩ ∧
      physicalWrite image patch b.destination = some ⟨before, some new⟩) :
    ∃ b, encodeBatch slotKey image patch = some b := by
  let k := Codec.forPatch image patch
  obtain ⟨ms, hm⟩ := encodeMoves_exists k (crossBindings patch) (fun b hb => by
    obtain ⟨_, _, _, source, dest⟩ := moves b hb
    exact ⟨physicalWrite_slot source, physicalWrite_slot dest⟩)
  let batch := batchWithMoves image patch ms
  have paths := encodeMoves_paths k _ _ hm
  have ends : ((Stage6.endpoints ms).map k.path).Nodup := by
    rw [← paths] at disjoint
    simpa [Stage6.endpoints, List.map_flatMap, List.flatMap_map, Function.comp_def] using disjoint
  have shape : batch.Shape := by
    refine ⟨?_, List.Pairwise.of_map k.path (fun _ _ h eq => h (congrArg k.path eq)) ends, ?_⟩
    · obtain ⟨path, e, he⟩ := nonempty
      obtain ⟨p, hp⟩ := physicalWrite_slot he
      have written : p ∈ batch.keys := by
        rw [Stage6.keys_iff]
        change ((physicalWrite image patch (k.path p)).map _).isSome = true
        rw [Codec.slot_spec hp, he]
        rfl
      intro empty
      rw [empty] at written
      cases written
    · intro m mem
      have member : (⟨k.path m.src, k.path m.dst⟩ : Binding) ∈ crossBindings patch :=
        paths ▸ List.mem_map_of_mem mem
      obtain ⟨old, new, _, source, dest⟩ := moves _ member
      change (physicalWrite image patch (k.path m.src)).map _ = some absent ∧
        ((physicalWrite image patch (k.path m.dst)).map _).getD absent ≠ absent ∧
        ((physicalWrite image patch (k.path m.src)).map _).getD absent ≠ absent
      simp only [source, dest, Option.map_some, Option.getD_some]
      exact ⟨rfl, fun h => (by simp : (some new : Entry) ≠ none) ((k.encode_absent_iff (some new)).mp h),
        fun h => (by simp : (some old : Entry) ≠ none) ((k.encode_absent_iff (some old)).mp h)⟩
  dsimp only [k] at hm
  dsimp only [batch] at shape
  exact ⟨batch, by simp [encodeBatch, hm, physical, shape, batch]⟩

theorem physicalWrite_decision (G : Graph) (d : Done) {path : Path} {e : Effect}
    (write : physicalWrite (listedImage G) (patchOf G d) path = some e) :
    e = ⟨(listedImage G)[path]?, (listedImage d.candidate)[path]?⟩ := by
  by_cases same : (listedImage G)[path]? = (listedImage d.candidate)[path]?
  · simp only [physicalWrite, patchOf_effect, if_pos same] at write
    rw [← same]
    split at write
    · cases h : (listedImage G)[path]? <;> simp [h] at write ⊢
      exact write.symm
    · contradiction
  · simp only [physicalWrite, patchOf_effect, if_neg same] at write
    exact (Option.some.inj write).symm

variable {K : Type} [DecidableEq K]
variable {O : Observations} {key : String → K} {url : String → Bool} {D : Date13.Dates K}
variable {E : Engine} {G : Graph} {r : Request} {d : Done}

theorem unchanged_survivor_write (success : decideRequest O key url D E G r = .success d) {s t : File}
    (source : d.plan.source = some s) (survivor : d.plan.survivor = some t)
    (unchanged : (patchOf G d).effects[t.path]? = none) :
    ∃ raw, physicalWrite (listedImage G) (patchOf G d) t.path = some ⟨some raw, some raw⟩ := by
  obtain ⟨deleted, linked, _⟩ := patchOf_merge success source survivor
  have distinct : s.path ≠ t.path := by intro same; rw [same, unchanged] at deleted; contradiction
  have receiver : ∃ b ∈ crossBindings (patchOf G d), b.destination = t.path :=
    ⟨⟨s.path, t.path⟩, by simp [crossBindings, linked, distinct], rfl⟩
  have old : ((listedImage G)[t.path]?).isSome = true := by
    have origins := patchOf_origin success (⟨t.path, t.path⟩ : Binding)
    obtain ⟨_, _, sameBinding⟩ := patchOf_merge success source survivor
    rcases origins.mp sameBinding with ⟨f, out, pair, _, _, eq⟩ | ⟨_, _, s', t', eq⟩
    · have pathEq : t.path = f.path := congrArg Binding.origin eq
      rw [pathEq]
      have wf := (done_name_safety (readingInputs O key url) D success).1.wellFormed
      have present := (listedImage_layout wf f.path (bytes f.text)).mpr
        ⟨f, retained_mem (List.of_mem_zip pair).1, rfl, rfl⟩
      simp [present]
    · rw [source] at s'
      rw [survivor] at t'
      cases s'
      cases t'
      exact False.elim (distinct (congrArg Binding.origin eq).symm)
  obtain ⟨raw, found⟩ := Option.isSome_iff_exists.mp old
  refine ⟨raw, ?_⟩
  simp [physicalWrite, unchanged, found, List.any_eq_true, receiver]

#print axioms encodeBatch_spec
#print axioms encodeBatch_keys
#print axioms encodeBatch_exists
#print axioms physicalWrite_decision
#print axioms unchanged_survivor_write

end ModelComposition
