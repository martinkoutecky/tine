import CompositionSpellingCodec
import CompositionBatchAdmission

namespace ModelComposition.Spelling
open ListedDisk Storage21 CM CM.Cut

def EntriesPublished (s : Prepared) (x : Stage6Spelling.State s.codec.paths.length) : Prop :=
  ∀ p m, s.move p = some m → (x.entry p).live = .destination ∧ (x.entry p).stable = .destination ∧
    (x.entry p).observation = some true ∧ x.core.core.s.disk p = m.expected

def BodyReady (s : Prepared) (x : Stage6.State s.codec.paths.length) (c : Config s.codec.paths.length) :
    Option (Stage6.Batch s.codec.paths.length) → Prop
  | none => True
  | some b => Stage6.enabledApply c x b = true ∧ BatchDiskAgreement s.image s.patch x

def afterBody (s : Prepared) (c : Config s.codec.paths.length) (x : Stage6Spelling.State s.codec.paths.length) :
    Option (Stage6.Batch s.codec.paths.length) → Stage6Spelling.State s.codec.paths.length
  | none => x
  | some b => {x with core := Stage6.opApply c x.core b}

structure PrefixSafety (s : Prepared) (c : Config s.codec.paths.length) (initial : s.codec.Slot → Int)
    (start z : Stage6Spelling.State s.codec.paths.length) : Prop where
  reached : Stage6.Reachable c initial z.core
  guarantee : Stage6.guarantee z.core = true
  order : Stage6.orderHolds z.core = true
  resolves : Stage6.renameResolves z.core = true
  currentCopies : ∀ p, LivePromise (Storage3.project z.core.core) p →
    Covers (Storage3.project z.core.core) p (z.core.core.s.disk p) ∨
      Covers (Storage3.project z.core.core) p (z.core.core.s.drafts p).bytes
  currentDraft : ∀ p, NeedsDraft c (Storage3.project z.core.core) p →
    (z.core.core.s.drafts p).bytes = (z.core.core.g.promise p).bytes
  liveTrash : ∀ p label, label ∈ z.core.core.g.removed p → label ∈ z.core.core.s.trash p
  stableTrash : ∀ p label, label ∈ z.core.core.g.delDurable p → label ∈ z.core.core.s.trashStable p
  oldCopies : ∀ p, (start.entry p).oldCopy = true → (z.entry p).oldCopy = true
  privateEntries : ∀ p m, s.move p = some m →
    ((z.entry p).live = .temporary ∨ (z.entry p).live = .trash) →
    z.listed m p = none ∧ (z.entry p).privateBytes m = some m.expected

theorem prefix_safety {s : Prepared} {c : Config s.codec.paths.length} {initial : s.codec.Slot → Int}
    {start z : Stage6Spelling.State s.codec.paths.length} (normal : c.mutant = "none")
    (base : Stage6.Reachable c initial start.core) (run : Stage6Spelling.Prefix s.move c start z) :
    PrefixSafety s c initial start z := by
  have reached := Stage6Spelling.project_prefix base run
  obtain ⟨safe, order, resolves⟩ := Stage6.all_targets_arbitrary_initial_disk c initial z.core normal reached
  obtain ⟨live, stable⟩ := Stage6.tracked_removals_in_trash c initial z.core normal reached
  have inv := (Stage6.core_safety_reachable c initial z.core normal reached).1
  exact ⟨reached, safe, order, resolves, custody_recoverable c _ inv.custody,
    fun p need => (inv.custody p need).1, live, stable, Stage6Spelling.old_copy_survives run,
    fun p m _ => Stage6Spelling.private_entry_custody z m p⟩

private theorem written_after (s : Prepared) (path : Path) (e : Effect)
    (effect : physicalWrite s.image s.patch path = some e) :
    (s.patch.replay s.image)[path]? = e.after := by
  rw [Patch.replay_lookup]
  cases semantic : s.patch.effects[path]? with
  | some actual =>
    simp only [physicalWrite, semantic] at effect
    simp [Option.some.inj effect]
  | none =>
    simp only [physicalWrite, semantic] at effect
    split at effect
    · cases old : s.image[path]? with
      | none => simp [old] at effect
      | some raw =>
        simp only [old, Option.map_some] at effect
        cases Option.some.inj effect
        rfl
    · contradiction

private theorem body_files {s : Prepared} {candidate : Image} {c : Config s.codec.paths.length}
    {x : Stage6.State s.codec.paths.length} {b : Stage6.Batch s.codec.paths.length}
    (normal : c.mutant = "none") (encoded : encodeBatch postKey s.image s.patch = some b)
    (replay : s.patch.replay s.image = candidate) (ready : BodyReady s x c (some b)) :
    ∀ p ∈ b.keys,
      s.codec.decode ((Stage6.opApply c x b).core.s.disk p) = some s.image[s.codec.path p]? ∧
      s.codec.decode ((Stage6.opApply c x b).core.s.drafts p).bytes = some candidate[s.codec.path p]? ∧
      ((Stage6.opApply c x b).core.s.drafts p).ver = Stage6.version x p := by
  intro p mem
  let path := s.codec.path p
  have written := (encodeBatch_keys encoded p).mp mem
  have disk := ready.2 p written
  have frame := (Stage6.admission_frame c x b).1
  have version := (Stage6.admission_versions c x b normal p mem).2.2.2.2
  obtain ⟨e, effect⟩ := Option.isSome_iff_exists.mp written
  have payload := ((encodeBatch_spec encoded).2.2.2 p).2
  change physicalWrite s.image s.patch path = some e at effect
  change b.eff p = (physicalWrite s.image s.patch path).map _ at payload
  rw [effect] at payload
  have target : candidate[path]? = e.after := by
    rw [← replay]
    exact written_after s path e effect
  refine ⟨?_, ?_, version.2⟩
  · rw [frame, disk]
    exact Codec.decode_image s.image s.patch p
  · rw [version.1,
      show Storage33.get b.eff p = s.codec.encode e.after by simp [Storage33.get, payload, Prepared.codec], target]
    exact Codec.decode_encode (physicalWrite_supported effect).2

private theorem admitted_copy {s : Prepared} {c : Config s.codec.paths.length}
    {initial : s.codec.Slot → Int} {start z : Stage6Spelling.State s.codec.paths.length}
    {x : Stage6.State s.codec.paths.length} {candidate : Image} {b : Stage6.Batch s.codec.paths.length}
    (safe : PrefixSafety s c initial start z) (encoded : encodeBatch postKey s.image s.patch = some b)
    (replay : s.patch.replay s.image = candidate) (p : s.codec.Slot) (mem : p ∈ b.keys)
    (ver : (z.core.core.g.promise p).ver = Stage6.version x p)
    (payload : (z.core.core.g.promise p).bytes = s.codec.encodeImage candidate p)
    (live : LivePromise (Storage3.project z.core.core) p) :
    s.codec.decode (z.core.core.s.disk p) = some candidate[s.codec.path p]? ∨
    s.codec.decode (z.core.core.s.drafts p).bytes = some candidate[s.codec.path p]? ∨
    ∃ w ∈ z.core.core.g.wrote p, w.1 = z.core.core.s.disk p ∧ Stage6.version x p < w.2 := by
  obtain ⟨e, effect⟩ := Option.isSome_iff_exists.mp ((encodeBatch_keys encoded p).mp mem)
  have target := written_after s (s.codec.path p) e effect
  rw [replay] at target
  have supported : s.codec.Supports candidate[s.codec.path p]? := by
    rw [target]
    exact (physicalWrite_supported effect).2
  by_cases disk : Covers (Storage3.project z.core.core) p (z.core.core.s.disk p)
  · rcases disk with same | ⟨w, hw, label, later⟩
    · left
      change z.core.core.s.disk p = (z.core.core.g.promise p).bytes at same
      rw [same, payload]
      exact Codec.decode_encode supported
    · exact Or.inr (Or.inr ⟨w, hw, label, ver ▸ later⟩)
  · have draft := safe.currentDraft p ⟨live, Or.inr (Or.inl disk)⟩
    right; left
    rw [draft, payload]
    exact Codec.decode_encode supported

structure DecisionAdmission (s : Prepared) (candidate : Image) (c : Config s.codec.paths.length)
    (initial : s.codec.Slot → Int) (start atBody : Stage6Spelling.State s.codec.paths.length)
    (body : Option (Stage6.Batch s.codec.paths.length)) : Prop where
  replay : s.patch.replay s.image = candidate
  listed : ∀ e ∈ s.entries, ∃ p, s.codec.slot? e.binding.destination = some p ∧
    atBody.listed ⟨e.binding.origin, e.binding.destination, s.codec.encode (some e.oldBytes),
      s.codec.encode (some e.newBytes)⟩ p = some (e.binding.destination, s.codec.encode (some e.oldBytes)) ∧
    s.codec.decode (s.codec.encode (some e.oldBytes)) = some (some e.oldBytes)
  retainedForRewrite : ∀ p m, s.move p = some m → m.expected ≠ m.offered → (atBody.entry p).oldCopy = true
  entryPrefix : PrefixSafety s c initial start atBody
  admission : match body with
    | none => afterBody s c atBody body = atBody
    | some b => Stage6.step c atBody.core (.opApply b) = some (afterBody s c atBody body).core ∧
      (∀ p ∈ b.keys,
        s.codec.decode ((afterBody s c atBody body).core.core.s.disk p) = some s.image[s.codec.path p]? ∧
        s.codec.decode ((afterBody s c atBody body).core.core.s.drafts p).bytes = some candidate[s.codec.path p]? ∧
        ((afterBody s c atBody body).core.core.s.drafts p).ver = Stage6.version atBody.core p) ∧
      (b.moves ≠ [] → ∃ gt, (afterBody s c atBody body).core.gates = atBody.core.gates ++ [gt] ∧
        gt.moves = b.moves ∧ gt.pending = b.keys)
  prefixes : ∀ z, Stage6Spelling.Prefix s.move c (afterBody s c atBody body) z →
    PrefixSafety s c initial start z ∧
    (∀ p m, s.move p = some m → (z.entry p).live = .destination ∧ (z.entry p).stable = .destination ∧
      (m.expected ≠ m.offered → (z.entry p).oldCopy = true)) ∧
    match body with
    | none => True
    | some b => ∀ p ∈ b.keys, (z.core.core.g.promise p).ver = Stage6.version atBody.core p →
      (z.core.core.g.promise p).bytes = s.codec.encodeImage candidate p →
      LivePromise (Storage3.project z.core.core) p →
      s.codec.decode (z.core.core.s.disk p) = some candidate[s.codec.path p]? ∨
      s.codec.decode (z.core.core.s.drafts p).bytes = some candidate[s.codec.path p]? ∨
      ∃ w ∈ z.core.core.g.wrote p, w.1 = z.core.core.s.disk p ∧ Stage6.version atBody.core p < w.2

theorem admit_spelling_decision {K : Type} [DecidableEq K]
    {O : Observations} {key : String → K} {url : String → Bool} {D : Date13.Dates K}
    {E : Engine} {G : Graph} {r : Request} {d : Done}
    (success : decideRequest O key url D E G r = .success d)
    {s : Prepared} (prepared : prepare entryFirst receipts (listedImage G) (patchOf G d) = some s)
    (fits : Fits s (listedImage G) preKey postKey O.foldedPath)
    (c : Config s.codec.paths.length) (initial : s.codec.Slot → Int)
    (start : Stage6.State s.codec.paths.length) (atBody : Stage6Spelling.State s.codec.paths.length)
    (normal : c.mutant = "none") (base : Stage6.Reachable c initial start)
    (run : Stage6Spelling.Prefix s.move c (Stage6Spelling.State.start start) atBody)
    (entries : EntriesPublished s atBody)
    {body : Option (Stage6.Batch s.codec.paths.length)} (residual : Residual s postKey body)
    (ready : BodyReady s atBody.core c body) :
    ListedLayout G (listedImage G) ∧ ListedLayout d.candidate (listedImage d.candidate) ∧
      DecisionAdmission s (listedImage d.candidate) c initial (Stage6Spelling.State.start start) atBody body := by
  have replay := (prepare_spec prepared).1.trans (patchOf_replay G d)
  have names := done_name_safety (readingInputs O key url) D success
  have listed : ∀ p m, s.move p = some m → atBody.listed m p = some (m.destination, m.expected) := by
    intro p m move
    obtain ⟨live, _, _, disk⟩ := entries p m move
    have nonempty : m.expected ≠ absent := by
      unfold Prepared.move at move
      cases found : s.entries.find? (fun e => e.binding.destination == s.codec.path p) with
      | none => simp [found] at move
      | some e =>
        simp only [found, Option.map_some] at move
        cases Option.some.inj move
        simp [Codec.encode_absent_iff]
    simp [Stage6Spelling.State.listed, Stage6Spelling.Entry.listed, live, disk, nonempty]
  have entrySafety := prefix_safety normal base run
  have next : Stage6Spelling.Prefix s.move c (Stage6Spelling.State.start start) (afterBody s c atBody body) := by
    cases body with
    | none => exact run
    | some b =>
      have step : Stage6.step c atBody.core (.opApply b) = some (Stage6.opApply c atBody.core b) := by
        simp [Stage6.step, Stage6.enabled, Stage6.execute, ready.1]
      exact .advance run (.storage (.opApply b) (fun _ => false) step)
  refine ⟨listedImage_layout names.1.wellFormed, listedImage_layout names.2.wellFormed,
    ⟨replay, ?_, ?_, entrySafety, ?_, ?_⟩⟩
  · intro e he
    obtain ⟨p, slot, move, old⟩ := fits.entries e he
    have supported : s.codec.Supports (some e.oldBytes) := by
      rw [← old]
      exact Codec.image_supported e.binding.destination
    exact ⟨p, slot, listed p _ move, Codec.decode_encode supported⟩
  · intro p m chosen rewrite
    have live := (entries p m chosen).1
    rcases Stage6Spelling.retain_before_rewrite run p m chosen rewrite with copy | ⟨source, _⟩
    · exact copy
    · rw [live] at source
      contradiction
  · cases residual with
    | empty => rfl
    | @batch b encoded =>
      refine ⟨by simp [afterBody, Stage6.step, Stage6.enabled, Stage6.execute, ready.1],
        body_files normal encoded replay ready, ?_⟩
      intro nonempty
      obtain ⟨gt, installed, moves, pending, _⟩ := Stage6.admission_gate c atBody.core b nonempty
      exact ⟨gt, installed, moves, pending⟩
  · intro z suffix
    have safe := prefix_safety normal base (next.append suffix)
    refine ⟨safe, ?_, ?_⟩
    · intro p m move
      obtain ⟨live, stable, _, _⟩ := entries p m move
      have settled : ((afterBody s c atBody body).entry p).live = .destination ∧
          ((afterBody s c atBody body).entry p).stable = .destination := by
        cases body <;> exact ⟨live, stable⟩
      have kept := Stage6Spelling.destination_survives suffix p settled
      refine ⟨kept.1, kept.2, ?_⟩
      intro rewrite
      rcases Stage6Spelling.retain_before_rewrite (next.append suffix) p m move rewrite with copy | ⟨source, _⟩
      · exact copy
      · rw [kept.1] at source
        contradiction
    · cases residual with
      | empty => trivial
      | batch encoded => exact admitted_copy safe encoded replay

#print axioms prefix_safety
#print axioms admit_spelling_decision

end ModelComposition.Spelling
