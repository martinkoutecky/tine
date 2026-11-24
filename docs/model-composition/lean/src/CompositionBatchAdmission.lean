import CompositionBatchCodec
import Stage6PairSafety

namespace ModelComposition
open ListedDisk Storage21 CM CM.Cut

/-- The live disk agrees with the conceptual preimage at every offered write,
including absence at new paths. Stable disk and paths outside the write set
can differ. The host's independently held buffers also need to match. -/
def BatchDiskAgreement (image : Image) (patch : Patch)
    (x : Stage6.State (Codec.forPatch image patch).paths.length) : Prop :=
  let k := Codec.forPatch image patch
  ∀ p, (physicalWrite image patch (k.path p)).isSome = true → x.core.s.disk p = k.encodeImage image p

/-- The ordinary host guards, apart from shape and the codec's label domain. -/
structure BatchReady (x : Stage6.State P) (b : Stage6.Batch P) : Prop where
  alive : x.core.s.alive = true
  free : Storage3.opFree x.core.s b.keys = true
  ungated : Stage6.touches x.gates b.keys = false
  buffers : ∀ p ∈ b.keys, (x.core.s.pages p).held = true ∧ clean (x.core.s.pages p) = true ∧
    b.plan p = some (x.core.s.pages p).buf
  unclaimed : ∀ p ∈ b.keys, Storage33.get b.plan p = absent → Storage33.get b.eff p ≠ absent →
    p ∉ x.claimed

private theorem batch_enabled {slotKey : Path → String} {image : Image} {patch : Patch}
    {b : Stage6.Batch (Codec.forPatch image patch).paths.length}
    (encoded : encodeBatch slotKey image patch = some b)
    (c : Config (Codec.forPatch image patch).paths.length)
    (palette : c.texts = (Codec.forPatch image patch).texts)
    (x : Stage6.State (Codec.forPatch image patch).paths.length)
    (ready : BatchReady x b) :
    Stage6.enabledApply c x b = true := by
  have shape := (encodeBatch_spec encoded).2.1
  have keys : b.keys.isEmpty = false := List.isEmpty_eq_false_iff.mpr shape.1
  have held : (b.keys.all fun p => (x.core.s.pages p).held) = true :=
    List.all_eq_true.mpr fun p hp => (ready.buffers p hp).1
  have cleaned : b.keys.all (Storage3.opClean x.core.s) = true :=
    List.all_eq_true.mpr fun p hp => by
      obtain ⟨on, clean, _⟩ := ready.buffers p hp
      simp [Storage3.opClean, on, clean]
  have fresh : Storage33.stale (Storage35.project x.admissionView) b.eff b.plan = false := by
    apply List.any_eq_false.mpr
    intro p hp
    obtain ⟨on, _, buf⟩ := ready.buffers p hp
    change ¬((b.plan p).isNone || Storage3.opCur x.core.s p != Storage33.get b.plan p) = true
    simp [Storage3.opCur, Storage33.get, on, buf]
  have valid : (b.keys.all fun p => Storage33.get b.eff p == absent ||
      c.texts.contains (Storage33.get b.eff p)) = true := by
    apply List.all_eq_true.mpr
    intro p hp
    obtain ⟨e, he⟩ := Option.isSome_iff_exists.mp ((encodeBatch_keys encoded p).mp hp)
    have map := ((encodeBatch_spec encoded).2.2.2 p).2
    rw [he] at map
    have bytes := Codec.encode_real (physicalWrite_supported he).2
    simpa [Storage33.get, map, palette] using bytes
  have claim : Storage34.claimedCreates x.admissionView b.eff b.plan = false := by
    apply List.any_eq_false.mpr
    intro p hp
    change ¬(Storage33.get b.plan p == absent && Storage33.get b.eff p != absent &&
      x.claimed.contains p) = true
    by_cases old : Storage33.get b.plan p = absent
    · by_cases new : Storage33.get b.eff p = absent
      · simp [new]
      · simp [ready.unclaimed p hp old new]
    · simp [old]
  have core : Storage33.enabledApply c (Storage35.project x.admissionView) b.eff b.plan none none = true := by
    change (x.core.s.alive && !b.keys.isEmpty && Storage3.opFree x.core.s b.keys &&
      Storage31.opHeld c x.core b.keys && !Storage32.touches (Storage35.project x.admissionView).s b.keys &&
      b.keys.all (fun p => Storage3.opClean x.core.s p || c.mutant == "MAC") &&
      (!Storage33.stale (Storage35.project x.admissionView) b.eff b.plan || c.mutant == "MEX") &&
      (b.keys.all fun p => Storage33.get b.eff p == absent || c.texts.contains (Storage33.get b.eff p)) &&
      true) = true
    have noLegacyGate : Storage32.touches (Storage35.project x.admissionView).s b.keys = false := rfl
    have cleanOr : b.keys.all (fun p => Storage3.opClean x.core.s p || c.mutant == "MAC") = true :=
      List.all_eq_true.mpr fun p hp => by simp [List.all_eq_true.mp cleaned p hp]
    simpa [ready.alive, keys, ready.free, Storage31.opHeld, held, noLegacyGate, cleanOr, fresh] using valid
  simp [Stage6.enabledApply, shape, ready.ungated, Storage34.enabledApply, core, claim]

structure BatchFileAdmission (k : Codec) (c : Config k.paths.length)
    (x y : Stage6.State k.paths.length) (p : k.Slot) (before after : Entry) : Prop where
  buffer : (x.core.s.pages p).buf = k.encode before
  base : (x.core.s.pages p).base = k.encode before
  disk : k.decode (y.core.s.disk p) = some before
  draft : k.decode (y.core.s.drafts p).bytes = some after
  draftVersion : (y.core.s.drafts p).ver = Stage6.version x p
  promiseBytes : (y.core.g.promise p).bytes = k.encode after
  promiseVersion : (y.core.g.promise p).ver = Stage6.version x p
  owed : NeedsDraft c (Storage3.project y.core) p

/-- A finite suffix starting at this particular admission, including the empty suffix. -/
inductive BatchContinuation (c : Config P) (y : Stage6.State P) : Stage6.State P → Prop where
  | here : BatchContinuation c y y
  | advance {u v} : BatchContinuation c y u → (a : Stage6.Action P) →
      Stage6.step c u a = some v → BatchContinuation c y v

structure BatchPrefixSafety (k : Codec) (c : Config k.paths.length) (candidate : Image)
    (b : Stage6.Batch k.paths.length) (x z : Stage6.State k.paths.length) : Prop where
  guarantee : Stage6.guarantee z = true
  order : Stage6.orderHolds z = true
  resolves : Stage6.renameResolves z = true
  pair : ∀ m ∈ b.moves, (m.src, m.dst) ∈ z.renamed →
    z.core.s.disk m.src ≠ absent ∨ z.core.s.disk m.dst ≠ absent
  currentCopies : ∀ p ∈ b.keys, LivePromise (Storage3.project z.core) p →
    Covers (Storage3.project z.core) p (z.core.s.disk p) ∨
      Covers (Storage3.project z.core) p (z.core.s.drafts p).bytes
  currentDraft : ∀ p ∈ b.keys, NeedsDraft c (Storage3.project z.core) p →
    (z.core.s.drafts p).bytes = (z.core.g.promise p).bytes
  admittedCopies : ∀ p ∈ b.keys, (z.core.g.promise p).ver = Stage6.version x p →
    (z.core.g.promise p).bytes = k.encodeImage candidate p → LivePromise (Storage3.project z.core) p →
    k.decode (z.core.s.disk p) = some candidate[k.path p]? ∨
      k.decode (z.core.s.drafts p).bytes = some candidate[k.path p]? ∨
      ∃ w ∈ z.core.g.wrote p, w.1 = z.core.s.disk p ∧ Stage6.version x p < w.2
  liveTrash : ∀ p ∈ b.keys, ∀ label, label ∈ z.core.g.removed p → label ∈ z.core.s.trash p
  stableTrash : ∀ p ∈ b.keys, ∀ label, label ∈ z.core.g.delDurable p → label ∈ z.core.s.trashStable p

structure DecisionBatchAdmission (G : Graph) (d : Done) (c : Config (decisionCodec G d).paths.length)
    (initial : (decisionCodec G d).Slot → Int) (x : Stage6.State (decisionCodec G d).paths.length)
    (b : Stage6.Batch (decisionCodec G d).paths.length) : Prop where
  shape : b.Shape
  step : Stage6.step c x (.opApply b) = some (Stage6.opApply c x b)
  files : ∀ p ∈ b.keys,
    BatchFileAdmission (decisionCodec G d) c x (Stage6.opApply c x b) p
      (listedImage G)[(decisionCodec G d).path p]? (listedImage d.candidate)[(decisionCodec G d).path p]?
  frame : let y := Stage6.opApply c x b
    y.core.s.disk = x.core.s.disk ∧ y.core.s.stable = x.core.s.stable ∧
    y.core.s.trash = x.core.s.trash ∧ y.core.s.trashStable = x.core.s.trashStable ∧
    ∀ p, p ∉ b.keys → y.core.s.pages p = x.core.s.pages p ∧ y.core.s.drafts p = x.core.s.drafts p
  changed : ∀ p, let y := Stage6.opApply c x b
    (y.core.s.pages p ≠ x.core.s.pages p ∨ y.core.s.drafts p ≠ x.core.s.drafts p) ↔ p ∈ b.keys
  marked : ∀ m ∈ b.moves, (m.src, m.dst) ∈ (Stage6.opApply c x b).renamed
  gate : b.moves ≠ [] → ∃ gt, (Stage6.opApply c x b).gates = x.gates ++ [gt] ∧
    gt.moves = b.moves ∧ gt.pending = b.keys ∧
    ∀ p, gt.vers p = (b.eff p).map (fun _ => Stage6.version x p)
  prefixes : ∀ z, BatchContinuation c (Stage6.opApply c x b) z →
    Stage6.Reachable c initial z ∧ BatchPrefixSafety (decisionCodec G d) c (listedImage d.candidate) b x z

private theorem prefix_safety (G : Graph) (d : Done) (c : Config (decisionCodec G d).paths.length)
    (initial : (decisionCodec G d).Slot → Int) (x z : Stage6.State (decisionCodec G d).paths.length)
    (b : Stage6.Batch (decisionCodec G d).paths.length)
    (normal : c.mutant = "none") (reached : Stage6.Reachable c initial z)
    (encoded : encodeBatch slotKey (listedImage G) (patchOf G d) = some b) :
    BatchPrefixSafety (decisionCodec G d) c (listedImage d.candidate) b x z := by
  obtain ⟨safe, order, resolves⟩ := Stage6.all_targets_arbitrary_initial_disk c initial z normal reached
  have inv := (Stage6.core_safety_reachable c initial z normal reached).1
  obtain ⟨live, stable⟩ := Stage6.tracked_removals_in_trash c initial z normal reached
  have copies := custody_recoverable c (Storage3.project z.core) inv.custody
  refine ⟨safe, order, resolves, ?_, fun p _ => copies p, fun p _ need => (inv.custody p need).1,
    ?_, fun p _ => live p, fun p _ => stable p⟩
  · intro m _ member
    simpa using List.all_eq_true.mp resolves (m.src, m.dst) member
  · intro p mem ver bytes on
    obtain ⟨e, he⟩ := Option.isSome_iff_exists.mp ((encodeBatch_keys encoded p).mp mem)
    have supported := (physicalWrite_supported he).2
    rw [physicalWrite_decision G d he] at supported
    have decoded := Codec.decode_encode supported
    by_cases disk : Covers (Storage3.project z.core) p (z.core.s.disk p)
    · rcases disk with same | ⟨w, hw, label, later⟩
      · left
        change z.core.s.disk p = (z.core.g.promise p).bytes at same
        rw [same, bytes]
        exact decoded
      · exact Or.inr (Or.inr ⟨w, hw, label, ver ▸ later⟩)
    · have exactDraft := (inv.custody p ⟨on, Or.inr (Or.inl disk)⟩).1
      right; left
      change (z.core.s.drafts p).bytes = (z.core.g.promise p).bytes at exactDraft
      rw [exactDraft, bytes]
      exact decoded

variable {K : Type} [DecidableEq K]
variable {O : Observations} {key : String → K} {url : String → Bool} {D : Date13.Dates K}
variable {E : Engine} {G : Graph} {r : Request} {d : Done}

set_option maxHeartbeats 800000 in
theorem admit_batch_decision (success : decideRequest O key url D E G r = .success d)
    {b : Stage6.Batch (decisionCodec G d).paths.length}
    (encoded : encodeBatch O.foldedPath (listedImage G) (patchOf G d) = some b)
    (c : Config (decisionCodec G d).paths.length) (initial : (decisionCodec G d).Slot → Int)
    (x : Stage6.State (decisionCodec G d).paths.length) (normal : c.mutant = "none")
    (palette : c.texts = (decisionCodec G d).texts) (reached : Stage6.Reachable c initial x)
    (disk : BatchDiskAgreement (listedImage G) (patchOf G d) x) (ready : BatchReady x b) :
    ListedLayout G (listedImage G) ∧ ListedLayout d.candidate (listedImage d.candidate) ∧
      DecisionBatchAdmission G d c initial x b := by
  let y := Stage6.opApply c x b
  have en := batch_enabled encoded c palette x ready
  have step : Stage6.step c x (.opApply b) = some y := by
    simp [Stage6.step, Stage6.enabled, en, Stage6.execute, y]
  have inv := (Stage6.core_safety_reachable c initial x normal reached).1
  have legacy := Stage6.apply_enabled_legacy en
  have projection := Storage34.apply_refines33_9 c x.admissionView b.eff b.plan none none
    (Storage34.apply_slot_free9 c x.admissionView b.eff b.plan none none normal legacy)
  have frame := Stage6.admission_frame c x b
  have names := done_name_safety (readingInputs O key url) D success
  refine ⟨listedImage_layout names.1.wellFormed, listedImage_layout names.2.wellFormed,
    ⟨(encodeBatch_spec encoded).2.1, step, ?_, ⟨frame.1, frame.2.1, rfl, rfl, frame.2.2⟩,
      Stage6.admission_changed_iff c x b normal inv.bounds, ?_, Stage6.admission_gate c x b, ?_⟩⟩
  · intro p mem
    obtain ⟨e, he⟩ := Option.isSome_iff_exists.mp ((encodeBatch_keys encoded p).mp mem)
    obtain ⟨plan, eff⟩ := (encodeBatch_spec encoded).2.2.2 p
    rw [he] at plan eff
    obtain ⟨_, cleaned, buf⟩ := ready.buffers p mem
    have buffer := Option.some.inj (plan.symm.trans buf)
    have base : (x.core.s.pages p).base = (decisionCodec G d).encode e.before := by
      simp [clean] at cleaned
      exact cleaned.1.2.symm.trans buffer.symm
    obtain ⟨on, bytes, ver, unsaved, draft, draftVer⟩ := Storage33.apply_new_custody8 c
      (Storage35.project x.admissionView) b.eff b.plan none none normal p mem (inv.lineage.1 p)
    rw [← projection] at on bytes ver unsaved draft draftVer
    change (y.core.g.promise p).bytes = Storage33.get b.eff p at bytes
    rw [show Storage33.get b.eff p = (decisionCodec G d).encode e.after by simp [Storage33.get, eff]] at bytes
    change (y.core.s.drafts p).bytes = (y.core.g.promise p).bytes at draft
    have owed : NeedsDraft c (Storage3.project y.core) p :=
      ⟨⟨on, fun expired => Bool.noConfusion (unsaved.symm.trans expired.1)⟩, Or.inl unsaved⟩
    have decoded := physicalWrite_supported he
    have endpoints := physicalWrite_decision G d he
    have old := disk p ((encodeBatch_keys encoded p).mp mem)
    rw [endpoints] at decoded buffer base bytes
    refine ⟨buffer.symm, base, ?_, ?_, draftVer.trans ver, bytes, ver, owed⟩
    · rw [show y.core.s.disk p = x.core.s.disk p by rw [frame.1], old]
      exact Codec.decode_encode decoded.1
    · rw [draft, bytes]
      exact Codec.decode_encode decoded.2
  · intro m member
    have src := (Stage6.move_endpoints_in_keys b (encodeBatch_spec encoded).2.1 m member).1
    obtain ⟨e, he⟩ := Option.isSome_iff_exists.mp ((encodeBatch_keys encoded m.src).mp src)
    have plan := ((encodeBatch_spec encoded).2.2.2 m.src).1
    rw [he] at plan
    have present := (encodeBatch_spec encoded).2.1.2.2 m member |>.2.2
    have diskPresent : x.core.s.disk m.src ≠ absent := by
      rw [disk m.src ((encodeBatch_keys encoded m.src).mp src)]
      rw [show (decisionCodec G d).encodeImage (listedImage G) m.src =
        (decisionCodec G d).encode e.before by rw [physicalWrite_decision G d he]; rfl]
      simpa [Storage33.get, plan] using present
    change (m.src, m.dst) ∈ union (Storage32.unrename x.renamed b.keys) _
    apply (mem_union_general _ _ _).mpr
    exact Or.inr (List.mem_map.mpr ⟨m, List.mem_filter.mpr ⟨member, by simpa using diskPresent⟩, rfl⟩)
  · intro z suffix
    have next : Stage6.Reachable c initial z := by
      induction suffix with
      | here => exact .advance reached (.opApply b) step
      | advance _ action step ih => exact .advance ih action step
    exact ⟨next, prefix_safety G d c initial x z b normal next encoded⟩

/-- A deletion step retains the bytes that were at the source when it ran. -/
theorem removal_step_actual_bytes {c : Config P} {u v : Stage6.State P}
    (normal : c.mutant = "none") (step : Stage6.step c u (.prior (.old .rename)) = some v)
    (deleting : u.core.s.job.bytes = absent) (present : u.core.s.disk u.core.s.job.p ≠ absent) :
    let p := u.core.s.job.p
    v.core.s.disk p = absent ∧ u.core.s.disk p ∈ v.core.s.trash p ∧ u.core.s.disk p ∈ v.core.g.removed p := by
  obtain ⟨_, rfl⟩ := Stage6.step_eq_some_iff.mp step
  simp only [Stage6.execute, Stage6.State.fromCore, Storage35.execute, Storage35.lift,
    Storage35Base.execute, Storage32.execute, Storage32.rename, normal]
  change (Storage3.rename c u.core).s.disk u.core.s.job.p = absent ∧
    u.core.s.disk u.core.s.job.p ∈ (Storage3.rename c u.core).s.trash u.core.s.job.p ∧
    u.core.s.disk u.core.s.job.p ∈ (Storage3.rename c u.core).g.removed u.core.s.job.p
  simp [Storage3.rename, Storage3.commit, put, normal, deleting, present, mem_union_general]

/-- A power cut copies its selected trash image to both trash views. The
payload test uses that image directly, even when the removal ghost forgets it. -/
theorem power_selects_trash {c : Config P} {u v : Stage6.State P} {keep : Fin P → Bool}
    (step : Stage6.step c u (.prior (.old (.power keep))) = some v) (p : Fin P) :
    v.core.s.trash p = (if keep p then u.core.s.trash p else u.core.s.trashStable p) ∧
    v.core.s.trashStable p = v.core.s.trash p := by
  obtain ⟨_, rfl⟩ := Stage6.step_eq_some_iff.mp step
  exact ⟨rfl, rfl⟩

#print axioms admit_batch_decision
#print axioms removal_step_actual_bytes
#print axioms power_selects_trash

end ModelComposition
