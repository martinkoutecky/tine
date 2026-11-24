import CompositionAdmission
import CutPatchWitnesses

namespace ModelComposition.Witnesses
open ListedDisk Storage21 CM CM.Cut CM.CutExamples

private def markdown (name text : String) : File := ⟨"pages/" ++ name ++ ".md", .pages, .md, text⟩

private def keep : File := markdown "C" "- target\n"
private def before : Graph := plain_rename_before ++ [keep]
private def after : Graph := plain_rename_after ++ [keep]
private def engine : Engine := ⟨fun _ => some after⟩
private def request : Request := ⟨"A", "B", none, true⟩
private def done : Done := ⟨⟨"B", plain_rename_before.head?, none⟩, after, []⟩

private theorem rename_success : decideRequest o asciiLower (fun _ => false) dates engine before request = .success done := by
  decide +kernel

private def patch : Patch := patchOf before done
private def codec : Codec := decisionCodec before done
private def config : Config codec.paths.length :=
  { positive := by decide +kernel, texts := codec.texts, nonempty := by decide +kernel }
private def disk : codec.Slot → Int := codec.encodeImage (listedImage before)
private def move := admission codec patch

private def slot (path : Path) : codec.Slot := (codec.slot? path).getD (zero config)
private def source := slot "pages/A.md"
private def destination := slot "pages/B.md"
private def referrer := slot "pages/R.md"
private def untouched := slot "pages/C.md"

private def load (p : codec.Slot) : Storage35.Action codec.paths.length := .prior (.load p)
private def initial := Storage35.initWith config disk
private def withSource := Storage35.execute config initial (load source)
private def withDestination := Storage35.execute config withSource (load destination)
private def loaded := Storage35.execute config withDestination (load referrer)
private def admitted := Storage35.execute config loaded move

private theorem load_reachable {x : Storage35.State codec.paths.length} (p : codec.Slot)
    (reachable : Storage35.Reachable config disk x) (enabled : Storage35.enabled config x (load p) = true) :
    Storage35.Reachable config disk (Storage35.execute config x (load p)) := by
  exact .advance reachable (load p) (by simp only [Storage35.step, enabled, if_true])

private theorem loaded_reachable : Storage35.Reachable config disk loaded :=
  load_reachable referrer (load_reachable destination (load_reachable source .initial (by decide +kernel))
    (by decide +kernel)) (by decide +kernel)

theorem rename_with_referrer_admitted :
    decideRequest o asciiLower (fun _ => false) dates engine before request = .success done ∧
    representable o.foldedPath (listedImage before) patch = true ∧
    Storage35.Reachable config disk loaded ∧ DiskAgreement (listedImage before) patch loaded ∧
    Storage35.enabled config loaded move = true ∧ Storage35.step config loaded move = some admitted ∧
    Storage35.Reachable config disk admitted ∧ admitted.g.renamed = [(source, destination)] ∧
    codec.decode (admitted.s.drafts source).bytes = some none ∧
    codec.decode (admitted.s.drafts destination).bytes = some (some (bytes "- notes\n")) ∧
    codec.decode (admitted.s.drafts referrer).bytes = some (some (bytes "- [[B]]\n")) ∧
    admitted.s.pages untouched = loaded.s.pages untouched ∧ admitted.s.drafts untouched = loaded.s.drafts untouched ∧
    admitted.s.disk = loaded.s.disk ∧
    Storage35.guarantee admitted = true ∧ Storage35.orderHolds admitted = true ∧ Storage35.renameResolves admitted = true := by
  have fits : representable o.foldedPath (listedImage before) patch = true := by decide +kernel
  have real (p : codec.Slot) : RealByte config (disk p) := Codec.encode_real (Codec.image_supported _)
  have agrees : DiskAgreement (listedImage before) patch loaded := fun _ _ => rfl
  have enabled : Storage35.enabled config loaded move = true := by decide +kernel
  obtain ⟨nondelete, noNone, noUnknown⟩ := codec.texts_reserve_sentinels
  obtain ⟨_, effects, frame, diskFrame, _, _, step, reached, safety⟩ := admit_decision rename_success o.foldedPath
    fits config disk loaded rfl real nondelete noNone noUnknown loaded_reachable agrees enabled
  have decoded {path : Path} {e : Effect} (he : patch.effects[path]? = some e)
      {p : codec.Slot} (hp : codec.slot? path = some p) :
      codec.decode (admitted.s.drafts p).bytes = some (listedImage after)[path]? := by
    obtain ⟨q, hq, custody⟩ := effects path e he
    change codec.slot? path = some q at hq
    have same := Option.some.inj (hq.symm.trans hp)
    subst q
    exact custody.draft
  have deleted := decoded (path := "pages/A.md") (e := ⟨some (bytes "- notes\n"), none⟩)
    (by decide +kernel) (p := source) (by decide +kernel)
  have moved := decoded (path := "pages/B.md") (e := ⟨none, some (bytes "- notes\n")⟩)
    (by decide +kernel) (p := destination) (by decide +kernel)
  have rewritten := decoded (path := "pages/R.md") (e := ⟨some (bytes "- [[A]]\n"), some (bytes "- [[B]]\n")⟩)
    (by decide +kernel) (p := referrer) (by decide +kernel)
  have outside := frame untouched (by decide +kernel)
  exact ⟨rename_success, fits, loaded_reachable, agrees, enabled, step, reached, by decide +kernel,
    deleted, moved, rewritten, outside.1, outside.2, diskFrame, safety⟩

private def externalDelete : Storage35.Action codec.paths.length := .prior (.old (.extWriteD source absent true))
private def stale := Storage35.execute config loaded externalDelete

theorem stale_source_admits :
    Storage35.Reachable config disk stale ∧ Storage35.enabled config stale move = true ∧
    (stale.s.pages source).held = true ∧ clean (stale.s.pages source) = true ∧
    (stale.s.pages source).buf = disk source ∧ stale.s.disk source = absent ∧
    ¬DiskAgreement (listedImage before) patch stale ∧
    (Storage35.execute config stale move).g.renamed = [] := by
  have enabled : Storage35.enabled config loaded externalDelete = true := by decide +kernel
  have reached : Storage35.Reachable config disk stale :=
    .advance loaded_reachable externalDelete (by simp only [Storage35.step, enabled, if_true, stale])
  refine ⟨reached, by decide +kernel, by decide +kernel, by decide +kernel, by decide +kernel,
    by decide +kernel, ?_, by decide +kernel⟩
  intro agreement
  exact (by decide +kernel : stale.s.disk source ≠ disk source) (agreement source (by decide +kernel))

private def classifies (O : Observations) (G : Graph) (E : Engine) (r : Request) (expected : Bool) : Bool :=
  match decideRequest O asciiLower (fun _ => false) dates E G r with
  | .success d => representable O.foldedPath (listedImage G) (patchOf G d) == expected
  | _ => false

private def oldPage := markdown "Old" "- notes\n"
private def targetPage := markdown "B" "- target\n"
private def namespaceObservations : Observations :=
  { o with stem := fun f => if f.path = "pages/A%2Fx.md" then "A/x" else o.stem f }
private def namespaceBefore : Graph :=
  [markdown "A%2Fsub" "- notes\n", markdown "A%2Fx" "- source\n", markdown "R" "- [[A]]\n"]
private def namespaceAfter : Graph :=
  [markdown "B%2Fsub" "- notes\n", markdown "B%2Fx" "- source\n", markdown "R" "- [[B]]\n"]

theorem stage2_same_name :
    classifies o [oldPage] ⟨fun _ => some [oldPage]⟩ ⟨"Old", "Old", none, true⟩ false = true := by
  decide +kernel

theorem stage2_removed_merge_source :
    classifies o plain_merge_before plain_merge_engine ⟨"A", "B", some .confirm, true⟩ true = true := by
  decide +kernel

theorem stage2_respelling :
    classifies o [oldPage] ⟨fun _ => some [markdown "old" "- notes\n"]⟩ ⟨"Old", "old", none, true⟩ false = true ∧
    o.foldedPath "pages/Old.md" = o.foldedPath "pages/old.md" := by
  exact ⟨by decide +kernel, respelling_only_patch.2⟩

theorem stage2_namespace_rename :
    classifies namespaceObservations namespaceBefore ⟨fun _ => some namespaceAfter⟩ ⟨"A", "B", none, true⟩ false = true := by
  decide +kernel

theorem stage2_fileless_reference_merge :
    classifies o fileless_merge_before fileless_merge_engine ⟨"A", "B", some .confirm, true⟩ true = true := by
  decide +kernel

theorem stage2_fileless_namespace_merge :
    classifies namespaceObservations (namespaceBefore ++ [targetPage]) ⟨fun _ => some (namespaceAfter ++ [targetPage])⟩
      ⟨"A", "B", some .confirm, true⟩ false = true := by
  decide +kernel

theorem stage2_unchanged_survivor :
    classifies o [markdown "A" "", targetPage] ⟨fun _ => some [targetPage]⟩ ⟨"A", "B", some .confirm, true⟩ false = true := by
  decide +kernel

theorem same_slot_pair_disabled (c : Config P) (x : Storage35.State P) (eff plan : Storage33.Map P) (p : Fin P) :
    Storage35.enabled c x (.opApply eff plan (some p) (some p)) = false := by
  simp [Storage34.enabled, Storage34.enabledApply, Storage33.enabledApply]

#print axioms rename_with_referrer_admitted
#print axioms stale_source_admits
#print axioms stage2_respelling

end ModelComposition.Witnesses
