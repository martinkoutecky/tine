import CutConsequences
import ListedDisk

namespace CM
namespace Cut

open ListedDisk

/-- The page image of the whole graph, using the listed path and exact UTF-8 text. -/
def listedImage (G : Graph) : Image :=
  Std.ExtTreeMap.ofList (G.map (fun f => (f.path, bytes f.text)))

/-- A host listing represents K's files at their exact graph-relative paths and bytes.
The host supplies the page listing and the interpretation of K's path and text fields. -/
def ListedLayout (G : Graph) (image : Image) : Prop :=
  ∀ path raw, image[path]? = some raw ↔ ∃ f ∈ G, f.path = path ∧ bytes f.text = raw

theorem listedImage_layout {G : Graph} (wellFormed : WF G) : ListedLayout G (listedImage G) := by
  intro path raw
  let entries := G.map (fun f => (f.path, bytes f.text))
  have distinct : entries.Pairwise (fun a b => ¬ compare a.1 b.1 = .eq) := by
    simpa [entries, WF, List.Nodup, List.pairwise_map] using wellFormed
  constructor
  · intro h
    by_cases present : ∃ f ∈ G, f.path = path
    · obtain ⟨f, hf, eq⟩ := present
      have item : (f.path, bytes f.text) ∈ entries := List.mem_map.mpr ⟨f, hf, rfl⟩
      have found := Std.ExtTreeMap.getElem?_ofList_of_mem (show compare f.path path = .eq by simp [eq]) distinct item
      change (Std.ExtTreeMap.ofList entries)[path]? = some raw at h
      exact ⟨f, hf, eq, Option.some.inj (found.symm.trans h)⟩
    · have missing : (entries.map Prod.fst).contains path = false := by
        simpa [entries, List.map_map, Function.comp_def, List.mem_map] using present
      have noEntry := Std.ExtTreeMap.getElem?_ofList_of_contains_eq_false (cmp := compare) missing
      change (Std.ExtTreeMap.ofList entries)[path]? = some raw at h
      rw [noEntry] at h
      contradiction
  · rintro ⟨f, hf, rfl, rfl⟩
    exact Std.ExtTreeMap.getElem?_ofList_of_mem (by simp) distinct (List.mem_map.mpr ⟨f, hf, rfl⟩)

private def entryEffects (before after : Image) : Std.ExtTreeMap Path Effect :=
  (before ∪ after).filterMap fun path _ =>
    let old := before[path]?
    let new := after[path]?
    if old = new then none else some ⟨old, new⟩

private def consumedBinding (p : Plan) : List Binding :=
  match p.source, p.survivor with
  | some s, some t => [⟨s.path, t.path⟩]
  | _, _ => []

/-- Compare the whole before and candidate images, then attach the checked file origins. -/
def patchOf (G : Graph) (d : Done) : Patch :=
  let effects := entryEffects (listedImage G) (listedImage d.candidate)
  let bindings := ((retained G d.plan).zip d.candidate).filterMap fun (f, k) =>
    if (effects[k.path]?).isSome ∨ (d.plan.source.isSome ∧ d.plan.survivor = some f)
    then some (⟨f.path, k.path⟩ : Binding) else none
  ⟨effects, bindings ++ consumedBinding d.plan⟩

theorem patchOf_effect (G : Graph) (d : Done) (path : Path) :
    (patchOf G d).effects[path]? =
      if (listedImage G)[path]? = (listedImage d.candidate)[path]? then none
      else some ⟨(listedImage G)[path]?, (listedImage d.candidate)[path]?⟩ := by
  simp only [patchOf, entryEffects, Std.ExtTreeMap.getElem?_filterMap', Std.ExtTreeMap.getElem?_union]
  cases (listedImage G)[path]? <;> cases (listedImage d.candidate)[path]? <;> simp

theorem patchOf_replay (G : Graph) (d : Done) :
    (patchOf G d).replay (listedImage G) = listedImage d.candidate := by
  apply Std.ExtTreeMap.ext_getElem?
  intro path
  rw [Patch.replay_lookup, patchOf_effect]
  split <;> simp_all

theorem patchOf_frame (G : Graph) (d : Done) (image : Image) (path : Path)
    (outside : (patchOf G d).effects[path]? = none) :
    ((patchOf G d).replay image)[path]? = image[path]? := by
  simp [Patch.replay_lookup, outside]

variable {K : Type} [DecidableEq K]
variable {O : Observations} {key : String → K} {url : String → Bool} {D : Date13.Dates K}
variable {E : Engine} {G : Graph} {r : Request} {d : Done}

theorem patchOf_replay_listed {image : Image}
    (success : decideRequest O key url D E G r = .success d)
    (layout : ListedLayout G image) :
    (patchOf G d).replay image = listedImage d.candidate ∧
    ListedLayout d.candidate ((patchOf G d).replay image) := by
  have safe := done_name_safety (readingInputs O key url) D success
  have beforeEq : image = listedImage G := by
    apply Std.ExtTreeMap.ext_getElem?
    intro path
    cases observed : image[path]? with
    | some raw => exact ((listedImage_layout safe.1.wellFormed path raw).mpr ((layout path raw).mp observed)).symm
    | none =>
      cases listed : (listedImage G)[path]? with
      | none => rfl
      | some raw =>
        have found := (layout path raw).mpr ((listedImage_layout safe.1.wellFormed path raw).mp listed)
        rw [observed] at found
        contradiction
  rw [beforeEq, patchOf_replay]
  exact ⟨rfl, listedImage_layout safe.2.wellFormed⟩

private theorem mem_consumedBinding {p : Plan} {b : Binding} :
    b ∈ consumedBinding p ↔ ∃ s t, p.source = some s ∧ p.survivor = some t ∧ b = ⟨s.path, t.path⟩ := by
  cases hs : p.source <;> cases ht : p.survivor <;> simp [consumedBinding, hs, ht]

theorem patchOf_origin (success : decideRequest O key url D E G r = .success d) (b : Binding) :
    b ∈ (patchOf G d).bindings ↔
      (∃ f k, (f, k) ∈ (retained G d.plan).zip d.candidate ∧
        pairMatches (readingInputs O key url) D O r.source d.plan f k ∧
        ((patchOf G d).effects[k.path]?.isSome ∨ (d.plan.source.isSome ∧ d.plan.survivor = some f)) ∧
        b = ⟨f.path, k.path⟩) ∨
      (∃ s t, d.plan.source = some s ∧ d.plan.survivor = some t ∧ b = ⟨s.path, t.path⟩) := by
  simp only [patchOf, List.mem_append, List.mem_filterMap, mem_consumedBinding]
  apply or_congr_left
  constructor
  · rintro ⟨⟨f, k⟩, hz, found⟩
    split at found
    · rename_i records
      exact ⟨f, k, hz, (done_reading (readingInputs O key url) D success).2 f k hz,
        records, (Option.some.inj found).symm⟩
    · contradiction
  · rintro ⟨f, k, hz, _, records, rfl⟩
    exact ⟨(f, k), hz, if_pos records⟩

private theorem authorized_plan {P : Consolidated13.Inputs K} {p p' : Plan} {q : Option Question}
    (h : authorize P D O G r p q = .success p') : p' = p := by
  unfold authorize at h
  split at h <;> try contradiction
  split at h <;> cases h
  rfl

private theorem merge_plan {P : Consolidated13.Inputs K} {s : Option File} {t : File}
    {q : Option Question} {p : Plan}
    (h : mergeSelection P D O G r s t q = .success p) : p.source = s ∧ p.survivor = some t := by
  unfold mergeSelection at h
  split at h <;> try contradiction
  rw [authorized_plan h]
  exact ⟨rfl, rfl⟩

private theorem selected_merge_members {P : Consolidated13.Inputs K} {p : Plan} {s t : File}
    (h : select P D O G r = .success p) (hs : p.source = some s) (ht : p.survivor = some t) :
    s ∈ G ∧ t ∈ G ∧ s ≠ t := by
  have selected := attendance_applied h
  unfold selectCore at selected
  split at selected <;> try contradiction
  split at selected <;> try contradiction
  split at selected <;> try contradiction
  dsimp only at selected
  split at selected
  · contradiction
  · next receiver holders =>
    have origins : p.source = sourceFile P D G r.source ∧ p.survivor = some receiver := by
      split at selected <;> exact merge_plan selected
    have sourceEq : sourceFile P D G r.source = some s := origins.1.symm.trans hs
    have sourceMember : s ∈ G := by
      unfold sourceFile at sourceEq
      split at sourceEq
      · next f claimants =>
        cases sourceEq
        have member : s ∈ Date13.claimants P D G (Date13.target P D G r.source) := by
          rw [claimants]
          exact List.mem_cons_self ..
        exact (List.mem_filter.mp member).1
      · contradiction
    have held : receiver ∈ (Date13.claimants P D G (Date13.target P D G r.typed)).filter
        (fun f => some f != sourceFile P D G r.source) := by
      rw [holders]
      exact List.mem_cons_self ..
    obtain ⟨held, different⟩ := List.mem_filter.mp held
    have receiverEq : receiver = t := Option.some.inj (origins.2.symm.trans ht)
    subst receiver
    refine ⟨sourceMember, (List.mem_filter.mp held).1, ?_⟩
    simpa [sourceEq, ne_comm] using different
  · split at selected <;> try contradiction
    split at selected <;> try contradiction
    rw [authorized_plan selected] at ht
    contradiction

theorem patchOf_merge (success : decideRequest O key url D E G r = .success d) {s t : File}
    (source : d.plan.source = some s) (survivor : d.plan.survivor = some t) :
    (patchOf G d).effects[s.path]? = some ⟨some (bytes s.text), none⟩ ∧
    ⟨s.path, t.path⟩ ∈ (patchOf G d).bindings ∧ ⟨t.path, t.path⟩ ∈ (patchOf G d).bindings := by
  let P := readingInputs O key url
  obtain ⟨plan, selected, planEq, _, _⟩ := done_checked P D success
  rw [← planEq] at selected
  obtain ⟨hs, ht, different⟩ := selected_merge_members selected source survivor
  have safe := done_name_safety P D success
  have reading := done_reading P D success
  have projections := zip_projections reading.1.symm
  have retainedSurvivor : t ∈ retained G d.plan := by
    simp [retained, source, survivor, ht, Ne.symm different]
  obtain ⟨pair, hp, pairOrigin⟩ := List.mem_map.mp (projections.1 ▸ retainedSurvivor)
  rcases pair with ⟨f, k⟩
  change f = t at pairOrigin
  subst f
  have matched := reading.2 t k hp
  have destination : k.path = t.path := by simpa [expectedPath, survivor] using matched.1
  have before : (listedImage G)[s.path]? = some (bytes s.text) :=
    (listedImage_layout safe.1.wellFormed s.path (bytes s.text)).mpr ⟨s, hs, rfl, rfl⟩
  have after : (listedImage d.candidate)[s.path]? = none := by
    cases found : (listedImage d.candidate)[s.path]? with
    | none => rfl
    | some raw =>
      obtain ⟨out, hout, outPath, _⟩ := (listedImage_layout safe.2.wellFormed s.path raw).mp found
      obtain ⟨pair, hz, pairOutput⟩ := List.mem_map.mp (projections.2 ▸ hout)
      rcases pair with ⟨origin, out'⟩
      change out' = out at pairOutput
      subst out'
      have retainedOrigin := (List.of_mem_zip hz).1
      have notSource : origin ≠ s := by
        have member := retainedOrigin
        simp [retained, source, survivor] at member
        exact member.2
      have pathNe : out.path ≠ origin.path := by
        intro eq
        exact notSource (wf_injective safe.1.wellFormed (retained_mem retainedOrigin) hs (eq.symm.trans outPath))
      have expected := (reading.2 origin out hz).1
      exact False.elim (safe.1.freePath origin retainedOrigin s hs notSource
        (expected ▸ pathNe) (congrArg O.foldedPath (expected.symm.trans outPath)))
  refine ⟨by simp [patchOf_effect, before, after], (patchOf_origin success _).mpr (Or.inr ⟨s, t, source, survivor, rfl⟩), ?_⟩
  exact (patchOf_origin success _).mpr (Or.inl ⟨t, k, hp, matched, Or.inr ⟨by simp [source], survivor⟩,
    by rw [destination]⟩)

theorem patchOf_written_origin (success : decideRequest O key url D E G r = .success d)
    {path : Path} {before : Entry} {raw : ListedDisk.Bytes}
    (written : (patchOf G d).effects[path]? = some ⟨before, some raw⟩) :
    ∃ f k, (f, k) ∈ (retained G d.plan).zip d.candidate ∧ k.path = path ∧ bytes k.text = raw ∧
      ⟨f.path, path⟩ ∈ (patchOf G d).bindings := by
  let P := readingInputs O key url
  have reading := done_reading P D success
  have safe := done_name_safety P D success
  have after : (listedImage d.candidate)[path]? = some raw := by
    rw [patchOf_effect] at written
    split at written
    · contradiction
    · exact congrArg Effect.after (Option.some.inj written)
  obtain ⟨k, hk, pathEq, rawEq⟩ := (listedImage_layout safe.2.wellFormed path raw).mp after
  obtain ⟨pair, hz, output⟩ := List.mem_map.mp ((zip_projections reading.1.symm).2 ▸ hk)
  rcases pair with ⟨f, k'⟩
  change k' = k at output
  subst k'
  refine ⟨f, k, hz, pathEq, rawEq, (patchOf_origin success _).mpr (Or.inl ⟨f, k, hz, reading.2 f k hz, ?_, ?_⟩)⟩
  · exact Or.inl (by simp [pathEq, written])
  · rw [pathEq]

theorem patchOf_fileless_deletion_is_move (success : decideRequest O key url D E G r = .success d)
    (source : d.plan.source = none) {path : Path} {raw : ListedDisk.Bytes}
    (deleted : (patchOf G d).effects[path]? = some ⟨some raw, none⟩) :
    ∃ destination, destination ≠ path ∧ ⟨path, destination⟩ ∈ (patchOf G d).bindings := by
  let P := readingInputs O key url
  have reading := done_reading P D success
  have safe := done_name_safety P D success
  have endpoints : (listedImage G)[path]? = some raw ∧ (listedImage d.candidate)[path]? = none := by
    rw [patchOf_effect] at deleted
    split at deleted
    · contradiction
    · have effectEq := Option.some.inj deleted
      exact ⟨congrArg Effect.before effectEq, congrArg Effect.after effectEq⟩
  obtain ⟨f, hf, rfl, _⟩ := (listedImage_layout safe.1.wellFormed path raw).mp endpoints.1
  have retainedOrigin : f ∈ retained G d.plan := by rwa [fileless_cardinality G d.plan source]
  obtain ⟨pair, hz, origin⟩ := List.mem_map.mp ((zip_projections reading.1.symm).1 ▸ retainedOrigin)
  rcases pair with ⟨f', k⟩
  change f' = f at origin
  subst f'
  have after : (listedImage d.candidate)[k.path]? = some (bytes k.text) :=
    (listedImage_layout safe.2.wellFormed k.path (bytes k.text)).mpr ⟨k, (List.of_mem_zip hz).2, rfl, rfl⟩
  have moved : k.path ≠ f.path := by
    intro same
    rw [same, endpoints.2] at after
    contradiction
  have expected := (reading.2 f k hz).1
  have before : (listedImage G)[k.path]? = none := by
    cases found : (listedImage G)[k.path]? with
    | none => rfl
    | some other =>
      obtain ⟨g, hg, pathEq, _⟩ := (listedImage_layout safe.1.wellFormed k.path other).mp found
      have different : f ≠ g := fun eq => moved (pathEq.symm.trans (congrArg File.path eq.symm))
      exact False.elim (safe.1.freePath f retainedOrigin g hg different (expected ▸ moved)
        (congrArg O.foldedPath (expected.symm.trans pathEq.symm)))
  exact ⟨k.path, moved, (patchOf_origin success _).mpr (Or.inl ⟨f, k, hz, reading.2 f k hz,
    Or.inl (by simp [patchOf_effect, before, after]), rfl⟩)⟩

end Cut
end CM
