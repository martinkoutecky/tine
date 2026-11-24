import CompositionBatchAdmission
import CutPatchWitnesses

namespace ModelComposition.BatchWitnesses
open ListedDisk Storage21 CM CM.Cut CM.CutExamples

private def markdown (name text : String) : File := ⟨"pages/" ++ name ++ ".md", .pages, .md, text⟩

private def initial_valid (G : Graph) (d : Done) (config : Config (decisionCodec G d).paths.length)
    (palette : config.texts = (decisionCodec G d).texts) :
    Stage6.InitialValid config ((decisionCodec G d).encodeImage (listedImage G)) := by
  refine ⟨?_, ?_⟩
  · intro p
    change _ = absent ∨ _ ∈ config.texts
    rw [palette]
    exact Codec.encode_real (Codec.image_supported _)
  · rw [palette]
    exact (decisionCodec G d).texts_reserve_sentinels

private def loadWrites (c : Config P) (x : Stage6.State P) : List (Fin P) → Option (Stage6.State P)
  | [] => some x
  | p :: ps => (Stage6.step c x (.prior (.load p))).bind (fun y => loadWrites c y ps)

private theorem loads_reachable {c : Config P} {disk : Fin P → Int} {x y : Stage6.State P}
    (ps : List (Fin P)) (reached : Stage6.Reachable c disk x) (loads : loadWrites c x ps = some y) :
    Stage6.Reachable c disk y := by
  induction ps generalizing x with
  | nil => cases Option.some.inj loads; exact reached
  | cons p ps ih =>
    simp only [loadWrites] at loads
    cases step : Stage6.step c x (.prior (.load p)) with
    | none => simp [step] at loads
    | some mid => exact ih (.advance reached (.prior (.load p)) step) (by simpa [step] using loads)

private def namespaceObservations : Observations :=
  { o with stem := fun f => if f.path = "pages/A%2Fx.md" then "A/x" else o.stem f }

private def before : Graph :=
  [markdown "A%2Fsub" "- notes\n", markdown "A%2Fx" "- source\n", markdown "R" "- [[A]]\n",
    markdown "C" "- untouched\n"]
private def after : Graph :=
  [markdown "B%2Fsub" "- notes\n", markdown "B%2Fx" "- source\n", markdown "R" "- [[B]]\n",
    markdown "C" "- untouched\n"]
private def engine : Engine := ⟨fun _ => some after⟩
private def request : Request := ⟨"A", "B", none, true⟩
private def done : Done := ⟨⟨"B", none, none⟩, after, []⟩
private def codec := decisionCodec before done
private def disk := codec.encodeImage (listedImage before)
private def config : Config codec.paths.length :=
  { positive := by decide +kernel, texts := codec.texts, nonempty := by decide +kernel }
private def encoded := encodeBatch namespaceObservations.foldedPath (listedImage before) (patchOf before done)
private def batch := encoded.get (by decide +kernel)
private def loads := loadWrites config (Stage6.initWith config disk) batch.keys
private def loaded := loads.get (by decide +kernel)
private def admitted := Stage6.opApply config loaded batch
private def referrer : codec.Slot := (codec.slot? "pages/R.md").get (by decide +kernel)
private def untouched : codec.Slot := (codec.slot? "pages/C.md").get (by decide +kernel)

theorem namespace_rename_admitted :
    decideRequest namespaceObservations asciiLower (fun _ => false) dates engine before request = .success done ∧
    representable namespaceObservations.foldedPath (listedImage before) (patchOf before done) = false ∧
    encoded = some batch ∧
    batch.moves.map (fun m => (codec.path m.src, codec.path m.dst)) =
      [("pages/A%2Fsub.md", "pages/B%2Fsub.md"), ("pages/A%2Fx.md", "pages/B%2Fx.md")] ∧
    batch.keys.length = 5 ∧ referrer ∈ batch.keys ∧ untouched ∉ batch.keys ∧
    Stage6.Reachable config disk loaded ∧ BatchDiskAgreement (listedImage before) (patchOf before done) loaded ∧
    DecisionBatchAdmission before done config disk loaded batch ∧
    codec.decode (admitted.core.s.drafts referrer).bytes = some (some (bytes "- [[B]]\n")) := by
  have success : decideRequest namespaceObservations asciiLower (fun _ => false) dates engine before request =
      .success done := by decide +kernel
  have compiled : encoded = some batch := Option.eq_some_of_isSome (by decide +kernel)
  have reached : Stage6.Reachable config disk loaded :=
    loads_reachable batch.keys (.initial (initial_valid before done config rfl))
      (Option.eq_some_of_isSome (by decide +kernel))
  have diskAgreement : BatchDiskAgreement (listedImage before) (patchOf before done) loaded := by
    intro p _
    change loaded.core.s.disk p = disk p
    revert p
    decide +kernel
  have ready : BatchReady loaded batch := by constructor <;> decide +kernel
  have admission := (admit_batch_decision success compiled config disk loaded rfl rfl reached diskAgreement ready).2.2
  have refKey : referrer ∈ batch.keys := by decide +kernel
  have entry : (listedImage done.candidate)[codec.path referrer]? = some (bytes "- [[B]]\n") := by
    decide +kernel
  exact ⟨success, by decide +kernel, compiled, by decide +kernel, by decide +kernel, refKey, by decide +kernel,
    reached, diskAgreement, admission, (admission.files referrer refKey).draft.trans (congrArg some entry)⟩

private def survivor := markdown "B" "- target\n"
private def mergeBefore : Graph := [markdown "A" "", survivor]
private def mergeDone : Done := ⟨⟨"B", mergeBefore.head?, some survivor⟩, [survivor], []⟩
private def mergeCodec := decisionCodec mergeBefore mergeDone
private def mergePatch := patchOf mergeBefore mergeDone
private def mergeDisk := mergeCodec.encodeImage (listedImage mergeBefore)
private def mergeConfig : Config mergeCodec.paths.length :=
  { positive := by decide +kernel, texts := mergeCodec.texts, nonempty := by decide +kernel }
private def mergeEncoded := encodeBatch o.foldedPath (listedImage mergeBefore) mergePatch
private def mergeBatch := mergeEncoded.get (by decide +kernel)
private def mergeLoads := loadWrites mergeConfig (Stage6.initWith mergeConfig mergeDisk) mergeBatch.keys
private def mergeLoaded := mergeLoads.get (by decide +kernel)
private def receiver : mergeCodec.Slot := (mergeCodec.slot? survivor.path).get (by decide +kernel)

theorem unchanged_merge_survivor_admitted :
    decideRequest o asciiLower (fun _ => false) dates ⟨fun _ => some [survivor]⟩ mergeBefore
      ⟨"A", "B", some .confirm, true⟩ = .success mergeDone ∧
    mergePatch.effects[survivor.path]? = none ∧
    receiver ∈ mergeBatch.keys ∧ mergeBatch.keys.length = 2 ∧
    mergeBatch.plan receiver = some (mergeDisk receiver) ∧ mergeBatch.eff receiver = mergeBatch.plan receiver ∧
    DecisionBatchAdmission mergeBefore mergeDone mergeConfig mergeDisk mergeLoaded mergeBatch := by
  have success : decideRequest o asciiLower (fun _ => false) dates ⟨fun _ => some [survivor]⟩ mergeBefore
      ⟨"A", "B", some .confirm, true⟩ = .success mergeDone := by decide +kernel
  have compiled : mergeEncoded = some mergeBatch := Option.eq_some_of_isSome (by decide +kernel)
  have reached : Stage6.Reachable mergeConfig mergeDisk mergeLoaded :=
    loads_reachable mergeBatch.keys (.initial (initial_valid mergeBefore mergeDone mergeConfig rfl))
      (Option.eq_some_of_isSome (by decide +kernel))
  have diskAgreement : BatchDiskAgreement (listedImage mergeBefore) mergePatch mergeLoaded := by
    intro p _
    change mergeLoaded.core.s.disk p = mergeDisk p
    revert p
    decide +kernel
  have ready : BatchReady mergeLoaded mergeBatch := by constructor <;> decide +kernel
  have admission := (admit_batch_decision success compiled mergeConfig mergeDisk mergeLoaded rfl rfl reached
    diskAgreement ready).2.2
  exact ⟨success, by decide +kernel, by decide +kernel, by decide +kernel, by decide +kernel,
    by decide +kernel, admission⟩

private def outsideDeletion : Stage6.Action mergeCodec.paths.length := .prior (.old (.extWriteD receiver absent true))
private def stale := Stage6.execute mergeConfig mergeLoaded outsideDeletion

theorem stale_merge_survivor_fails_disk_agreement :
    Stage6.step mergeConfig mergeLoaded outsideDeletion = some stale ∧
    Stage6.enabledApply mergeConfig stale mergeBatch = true ∧
    stale.core.s.disk receiver = absent ∧ mergeDisk receiver ≠ absent ∧
    ¬ BatchDiskAgreement (listedImage mergeBefore) mergePatch stale := by
  have enabled : Stage6.enabled mergeConfig mergeLoaded outsideDeletion = true := by decide +kernel
  have step : Stage6.step mergeConfig mergeLoaded outsideDeletion = some stale := by
    simp [Stage6.step, enabled, stale]
  have missing : stale.core.s.disk receiver = absent := by decide +kernel
  have planned : mergeDisk receiver ≠ absent := by decide +kernel
  refine ⟨step, by decide +kernel, missing, planned, ?_⟩
  intro disk
  have equal : stale.core.s.disk receiver = mergeDisk receiver := disk receiver (by decide +kernel)
  exact planned (equal.symm.trans missing)

private def old := markdown "Old" "- notes\n"
private def respelled := markdown "old" "- notes\n"
private def respelling : Done := ⟨⟨"old", some old, none⟩, [respelled], []⟩

theorem same_slot_respelling_refused :
    decideRequest o asciiLower (fun _ => false) dates ⟨fun _ => some [respelled]⟩ [old]
      ⟨"Old", "old", none, true⟩ = .success respelling ∧
    old.path ≠ respelled.path ∧ o.foldedPath old.path = o.foldedPath respelled.path ∧
    ¬ ((decisionCodec [old] respelling).paths.map o.foldedPath).Nodup ∧
    (encodeBatch o.foldedPath (listedImage [old]) (patchOf [old] respelling)).isNone = true := by
  exact ⟨by decide +kernel, by decide +kernel, by decide +kernel, by decide +kernel, by decide +kernel⟩

#print axioms namespace_rename_admitted
#print axioms unchanged_merge_survivor_admitted
#print axioms stale_merge_survivor_fails_disk_agreement
#print axioms same_slot_respelling_refused

end ModelComposition.BatchWitnesses
