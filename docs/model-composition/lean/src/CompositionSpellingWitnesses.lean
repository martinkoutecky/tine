import CompositionSpellingAdmission
import CutPatchWitnesses

namespace ModelComposition.Spelling.Witnesses
open ListedDisk (Binding)
open CM CM.Cut CM.CutExamples Stage6Spelling
open Storage21 (Config noneBytes)

private def old : File := ⟨"pages/Old.md", .pages, .md, "- notes\n"⟩
private def new : File := {old with path := "pages/old.md"}
private def reference : File := ⟨"pages/R.md", .pages, .md, "- [[Old]]\n"⟩
private def rewritten : File := {reference with text := "- [[old]]\n"}
private def parsedLink (start : Nat) (name : String) : Content :=
  ⟨[], [⟨0, [.site ⟨start, start + 7, .link, .ordinary, name, "[[" ++ name ++ "]]"⟩], false⟩]⟩
private def observations : Observations :=
  { o with
    read := fun fmt text =>
      if text = reference.text then parsedLink 2 "Old"
      else if text = rewritten.text then parsedLink 2 "old"
      else if text = "[[old]]" then parsedLink 0 "old"
      else o.read fmt text,
    supported := fun f => f.text == reference.text || f.text == rewritten.text || o.supported f }
private def before : Graph := [old, reference]
private def done : Done := ⟨⟨"old", some old, none⟩, [new, rewritten], []⟩
private def engine : Engine := ⟨fun _ => some done.candidate⟩
private def request : Request := ⟨"Old", "old", none, true⟩
private def patch := patchOf before done
private def entryFirst (b : Binding) : Bool := b.origin == old.path && b.destination == new.path
private def prepared := prepare entryFirst false (listedImage before) patch
private def plan := prepared.get (by decide +kernel)
private def codec := plan.codec
private def disk := codec.encodeImage plan.image
private def cfg : Config codec.paths.length :=
  { positive := by decide +kernel, texts := codec.texts, nonempty := by decide +kernel }
private def encoded := encodeBatch o.foldedPath plan.image plan.patch
private def batch := encoded.get (by decide +kernel)
private def page : codec.Slot := (codec.slot? new.path).get (by decide +kernel)
private def referrer : codec.Slot := (codec.slot? reference.path).get (by decide +kernel)
private def move := (plan.move page).get (by decide +kernel)
private def start := Stage6.initWith cfg disk
private def origin := State.start start

private def advanceStorage (x : State codec.paths.length) (a : Stage6.Action codec.paths.length) :
    State codec.paths.length := ⟨Stage6.execute cfg x.core a, afterAction x a (fun _ => false)⟩

private theorem storage_step (x : State codec.paths.length) (a : Stage6.Action codec.paths.length)
    (enabled : Stage6.enabled cfg x.core a = true) : Step plan.move cfg x (advanceStorage x a) :=
  .storage a (fun _ => false) (by simp [Stage6.step, enabled])

private def atPage := advanceStorage origin (.prior (.load page))
private def loaded := advanceStorage atPage (.prior (.load referrer))

private def setEntry (x : State codec.paths.length) (e : Entry) : State codec.paths.length :=
  {x with entry := fun p => if p = page then e else x.entry p}

private def renamed := setEntry loaded {live := .destination, command := true}
private def synced := setEntry renamed {live := .destination, stable := .destination, command := true}
private def published := setEntry synced {live := .destination, stable := .destination, observation := some true}

private theorem loaded_prefix : Prefix plan.move cfg origin loaded :=
  .advance (.advance .here (storage_step origin _ (by decide +kernel)))
    (storage_step atPage _ (by decide +kernel))

private theorem guard : EntryGuard loaded page move := by
  refine ⟨by decide +kernel, by decide +kernel, by decide +kernel, by decide +kernel,
    by decide +kernel, by decide +kernel, by decide +kernel⟩

private theorem found : plan.move page = some move := Option.eq_some_of_isSome (by decide +kernel)

private theorem guard_after (x : State codec.paths.length) (p : codec.Slot) (m : Move)
    (outside : (x.entry p).live ≠ .source) : (x.entry p).live = .source → EntryGuard x p m :=
  fun eq => False.elim (outside eq)

private theorem renamed_prefix : Prefix plan.move cfg origin renamed := by
  have firstStep : Step plan.move cfg loaded renamed :=
    .entry found (fun _ => guard) (fun _ _ => by decide +kernel)
      (.direct rfl (by decide +kernel) (.inl (by decide +kernel)))
  exact .advance loaded_prefix firstStep

private theorem renamed_here : renamed.entry page = {live := .destination, command := true} := by decide +kernel

private theorem published_prefix : Prefix plan.move cfg origin published := by
  have syncStep : Step plan.move cfg renamed synced :=
    .entry found (guard_after renamed page move (by decide +kernel)) (fun _ _ => by decide +kernel)
      (by rw [renamed_here]; exact .sync)
  have syncedHere : synced.entry page = {live := .destination, stable := .destination, command := true} :=
    by decide +kernel
  have observeStep : Step plan.move cfg synced published :=
    .entry found (guard_after synced page move (by decide +kernel)) (fun _ _ => by decide +kernel)
      (by rw [syncedHere]; exact .observe true rfl rfl rfl (fun _ => rfl))
  exact .advance (.advance renamed_prefix syncStep) observeStep

private def uncertain := setEntry renamed {live := .destination, observation := some false}

theorem observed_sync_failure_cannot_admit_body :
    Prefix plan.move cfg origin uncertain ∧ uncertain.listed move page = some (new.path, disk page) ∧
    (uncertain.entry page).receipted move true = [old.path, new.path] ∧ ¬ EntriesPublished plan uncertain := by
  have step : Step plan.move cfg renamed uncertain :=
    .entry found (guard_after renamed page move (by decide +kernel)) (fun _ _ => by decide +kernel)
      (by rw [renamed_here]; exact .observe false rfl rfl rfl (by simp))
  refine ⟨.advance renamed_prefix step, by decide +kernel, by decide +kernel, ?_⟩
  intro h
  exact (by decide +kernel : (uncertain.entry page).observation ≠ some true) (h page move found).2.2.1

private theorem initial_valid : Stage6.InitialValid cfg disk := by
  refine ⟨?_, codec.texts_reserve_sentinels⟩
  intro p
  exact Codec.encode_real (Codec.image_supported (codec.path p))

private theorem success : decideRequest observations asciiLower (fun _ => false) dates engine before request = .success done := by
  decide +kernel

private theorem fitting : Fits plan (listedImage before) o.foldedPath o.foldedPath o.foldedPath := by
  constructor <;> decide +kernel

theorem case_and_referrer_admitted :
    decideRequest observations asciiLower (fun _ => false) dates engine before request = .success done ∧
    o.foldedPath old.path = o.foldedPath new.path ∧
    (encodeBatch o.foldedPath (listedImage before) patch).isNone = true ∧
    DecisionAdmission plan (listedImage done.candidate) cfg disk origin published (some batch) ∧
    batch.moves = [] ∧ batch.keys.length = 2 ∧
    published.listed move page = some (new.path, disk page) ∧
    codec.decode ((afterBody plan cfg published (some batch)).core.core.s.drafts referrer).bytes =
      some (some (bytes "- [[old]]\n")) := by
  have ready : BodyReady plan published.core cfg (some batch) := by
    refine ⟨by decide +kernel, ?_⟩
    intro p _
    change published.core.core.s.disk p = disk p
    rfl
  have entries : EntriesPublished plan published := by
    intro p m h
    have only : ∀ p, plan.move p ≠ none → p = page := by decide +kernel
    have eq : p = page := only p (by simp [h])
    subst p
    rw [found] at h
    cases Option.some.inj h
    exact ⟨by decide +kernel, by decide +kernel, by decide +kernel, by decide +kernel⟩
  have preparedPlan : prepared = some plan := Option.eq_some_of_isSome (by decide +kernel)
  have encodedBatch : encoded = some batch := Option.eq_some_of_isSome (by decide +kernel)
  have admission := (admit_spelling_decision success preparedPlan
    fitting cfg disk start published rfl (.initial initial_valid) published_prefix entries
    (.batch encodedBatch) ready).2.2
  exact ⟨success, by decide +kernel, by decide +kernel, admission, by decide +kernel, by decide +kernel,
    by decide +kernel, by decide +kernel⟩

private def firstHalf := setEntry loaded {live := .temporary, command := true}
private def stableHalf := setEntry firstHalf {live := .temporary, stable := .temporary, command := true}
private def stopped := advanceStorage stableHalf (.prior (.old .crash))

private theorem stopped_prefix : Prefix plan.move cfg origin stopped := by
  have firstStep : Step plan.move cfg loaded firstHalf :=
    .entry found (fun _ => guard) (fun _ _ => by decide +kernel)
      (.first rfl (by decide +kernel) (.inl (by decide +kernel)))
  have here : firstHalf.entry page = {live := .temporary, command := true} := by decide +kernel
  have syncStep : Step plan.move cfg firstHalf stableHalf :=
    .entry found (guard_after firstHalf page move (by decide +kernel)) (fun _ _ => by decide +kernel)
      (by rw [here]; exact .sync)
  exact .advance (.advance (.advance loaded_prefix firstStep) syncStep)
    (storage_step stableHalf _ (by decide +kernel))

private def recovered (blocked : Bool) := setEntry stopped
  {live := if blocked then .trash else .destination, stable := .temporary}

private theorem recovery_prefix (blocked : Bool) : Prefix plan.move cfg stopped (recovered blocked) := by
  have here : stopped.entry page = {live := .temporary, stable := .temporary} := by decide +kernel
  have running : stopped.core.core.s.alive = false := by decide +kernel
  exact .advance .here (.entry found (guard_after stopped page move (by decide +kernel))
    (fun _ _ => by decide +kernel) (by rw [here, running]; exact .recover blocked rfl rfl))

theorem case_cut_has_private_bytes_and_no_listed_entry :
    Prefix plan.move cfg origin stopped ∧ stopped.core.core.s.alive = false ∧
    stopped.listed move page = none ∧ (stopped.entry page).privateBytes move = some (disk page) ∧
    (stopped.core.core.s.drafts referrer).bytes = noneBytes ∧ ¬ EntriesPublished plan stopped ∧
    Prefix plan.move cfg stopped (recovered false) ∧
    (recovered false).listed move page = some (new.path, disk page) ∧
    ((recovered false).entry page).observation = none := by
  refine ⟨stopped_prefix, by decide +kernel, by decide +kernel, by decide +kernel,
    by decide +kernel, ?_, recovery_prefix false, by decide +kernel, by decide +kernel⟩
  intro h
  exact (by decide +kernel : (stopped.entry page).live ≠ .destination) (h page move found).1

theorem case_cut_collision_keeps_bytes_in_trash :
    Prefix plan.move cfg stopped (recovered true) ∧
    ((recovered true).entry page).live = .trash ∧
    (recovered true).listed move page = none ∧
    ((recovered true).entry page).privateBytes move = some (disk page) :=
  ⟨recovery_prefix true, by decide +kernel, by decide +kernel, by decide +kernel⟩

theorem separate_case_entries_use_the_ordinary_batch :
    (encodeBatch id (listedImage before) patch).map (fun b => (b.moves.length, b.keys.length)) =
      some (1, 3) := by decide +kernel

private def twinImage := ((listedImage before).insert "pages/r.md" (bytes "- another page\n"))
private def sensitivePlan := (prepare entryFirst false twinImage patch).get (by decide +kernel)

theorem unrelated_folded_twins_need_distinct_host_keys :
    (encodeBatch o.foldedPath sensitivePlan.image sensitivePlan.patch).isNone = true ∧
    (encodeBatch id sensitivePlan.image sensitivePlan.patch).isSome = true := by decide +kernel

private def mixedImage := (listedImage before).insert "pages/A.md" (bytes "- another page\n")
private def mixedPatch : ListedDisk.Patch :=
  ⟨(patch.effects.insert "pages/A.md" ⟨some (bytes "- another page\n"), none⟩).insert
    "pages/B.md" ⟨none, some (bytes "- another page\n")⟩,
    patch.bindings ++ [⟨"pages/A.md", "pages/B.md"⟩]⟩
private def mixedPlan := (prepare entryFirst false mixedImage mixedPatch).get (by decide +kernel)

theorem mixed_entry_move_and_batch_pair :
    mixedPlan.entries.length = 1 ∧
    (encodeBatch o.foldedPath mixedPlan.image mixedPlan.patch).map (fun b => (b.moves.length, b.keys.length)) =
      some (1, 4) := by decide +kernel

private def entryOnly := (prepare entryFirst true (listedImage [old])
  (patchOf [old] ⟨⟨"old", some old, none⟩, [new], []⟩)).get (by decide +kernel)

theorem receipted_entry_only_has_empty_body :
    Residual entryOnly o.foldedPath none ∧
    (published.entry page).receipted move true = [old.path, new.path] ∧
    (published.entry page).receipted move false = [] :=
  ⟨.empty (by decide +kernel), by decide +kernel, by decide +kernel⟩

#print axioms case_and_referrer_admitted
#print axioms observed_sync_failure_cannot_admit_body
#print axioms case_cut_has_private_bytes_and_no_listed_entry
#print axioms case_cut_collision_keeps_bytes_in_trash
#print axioms separate_case_entries_use_the_ordinary_batch
#print axioms unrelated_folded_twins_need_distinct_host_keys
#print axioms mixed_entry_move_and_batch_pair
#print axioms receipted_entry_only_has_empty_body

end ModelComposition.Spelling.Witnesses
