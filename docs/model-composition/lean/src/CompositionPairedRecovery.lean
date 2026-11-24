import CompositionBatchAdmission
import PairedRecoveryProgress

namespace ModelComposition
open ListedDisk Storage21 CM CM.Cut PairedRecovery

private theorem offered_bytes {G : Graph} {d : Done}
    {b : Stage6.Batch (decisionCodec G d).paths.length}
    (encoded : encodeBatch slotKey (listedImage G) (patchOf G d) = some b)
    (p : (decisionCodec G d).Slot) (written : p ∈ b.keys) :
    before b p = (decisionCodec G d).encodeImage (listedImage G) p ∧
      after b p = (decisionCodec G d).encodeImage (listedImage d.candidate) p ∧
      (decisionCodec G d).Supports (listedImage d.candidate)[(decisionCodec G d).path p]? := by
  obtain ⟨e, effect⟩ := Option.isSome_iff_exists.mp ((encodeBatch_keys encoded p).mp written)
  obtain ⟨old, new⟩ := (encodeBatch_spec encoded).2.2.2 p
  have support := (physicalWrite_supported effect).2
  rw [physicalWrite_decision G d effect] at support
  rw [effect, physicalWrite_decision G d effect] at old new
  exact ⟨by simp [Storage33.get, Codec.encodeImage, old],
    by simp [Storage33.get, Codec.encodeImage, new], support⟩

structure PairedAdmission (G : Graph) (d : Done)
    (b : Stage6.Batch (decisionCodec G d).paths.length)
    (start : PairedRecovery.State (decisionCodec G d).paths.length) : Prop where
  binding : ∀ m ∈ b.moves, (⟨(decisionCodec G d).path m.src, (decisionCodec G d).path m.dst⟩
    : Binding) ∈ crossBindings (patchOf G d)
  prefixes : ∀ race y, PairedRecovery.Reaches b race start y →
    PairedRecovery.renameResolves b y ∧ PairedRecovery.orderHolds b race y
  recovery : ∀ race x, PairedRecovery.Reaches b race start x → x.phase = .pending →
    (∀ p ∈ b.keys, x.choice p = none ∧ x.disarmed p = false) →
    (∀ p ∈ b.keys, x.live p = before b p ∨ x.live p = after b p) →
    ∃ y, PairedRecovery.Reaches b race x y ∧ Frame b x y ∧ y.result = some .applied ∧
      ∀ p ∈ b.keys, (decisionCodec G d).decode (y.stable p) =
        some (listedImage d.candidate)[(decisionCodec G d).path p]? ∧ y.live p = y.stable p

variable {K : Type} [DecidableEq K]
variable {O : Observations} {key : String → K} {url : String → Bool} {D : Date13.Dates K}
variable {E : Engine} {G : Graph} {r : Request} {d : Done}

theorem admit_paired_decision (success : decideRequest O key url D E G r = .success d)
    {b : Stage6.Batch (decisionCodec G d).paths.length}
    (encoded : encodeBatch O.foldedPath (listedImage G) (patchOf G d) = some b)
    (c : Config (decisionCodec G d).paths.length) (initial : (decisionCodec G d).Slot → Int)
    (x : Stage6.State (decisionCodec G d).paths.length) (normal : c.mutant = "none")
    (palette : c.texts = (decisionCodec G d).texts) (reached : Stage6.Reachable c initial x)
    (disk : BatchDiskAgreement (listedImage G) (patchOf G d) x) (ready : BatchReady x b) :
    let start := PairedRecovery.init b x.core.s.disk x.core.s.stable
    DecisionBatchAdmission G d c initial x b ∧ PairedAdmission G d b start ∧ Clean b start := by
  have admitted := (admit_batch_decision success encoded c initial x normal palette reached disk ready).2.2
  refine ⟨admitted, ⟨?_, ?_, ?_⟩, ⟨⟨rfl, rfl⟩, fun _ _ => ⟨rfl, rfl⟩, ?_⟩⟩
  · intro m mm
    rw [← (encodeBatch_spec encoded).2.2.1]
    exact List.mem_map_of_mem mm
  · intro race y trace
    exact ⟨PairedRecovery.renameResolves_reachable admitted.shape trace,
      PairedRecovery.orderHolds_reachable trace⟩
  · intro race u _ pending noChoices compatible
    obtain ⟨v, trace, frame, result, exactFiles⟩ := clean_relaunch_completion (race := race)
      admitted.shape pending noChoices compatible
    refine ⟨v, trace, frame, result, fun p hp => ⟨?_, (exactFiles p hp).1.trans (exactFiles p hp).2.symm⟩⟩
    rw [(exactFiles p hp).2, (offered_bytes encoded p hp).2.1]
    exact Codec.decode_encode (offered_bytes encoded p hp).2.2
  · intro p hp
    exact Or.inl ((disk p ((encodeBatch_keys encoded p).mp hp)).trans (offered_bytes encoded p hp).1.symm)

private theorem unwritten_same {b : Stage6.Batch (decisionCodec G d).paths.length}
    (encoded : encodeBatch slotKey (listedImage G) (patchOf G d) = some b)
    (p : (decisionCodec G d).Slot) (unwritten : p ∉ b.keys) :
    (listedImage G)[(decisionCodec G d).path p]? = (listedImage d.candidate)[(decisionCodec G d).path p]? := by
  let path := (decisionCodec G d).path p
  by_cases same : (listedImage G)[path]? = (listedImage d.candidate)[path]?
  · exact same
  · have effect : (patchOf G d).effects[path]? =
        some ⟨(listedImage G)[path]?, (listedImage d.candidate)[path]?⟩ := by
      simp [patchOf_effect, same]
    have written : (physicalWrite (listedImage G) (patchOf G d) path).isSome = true := by
      simp [physicalWrite, effect]
    exact False.elim (unwritten ((encodeBatch_keys encoded p).mpr written))

/-- The entire encoded image agrees when the untouched entries already
agreed with the conceptual input in both disk views. -/
theorem paired_recovery_exact_image {b : Stage6.Batch (decisionCodec G d).paths.length}
    (encoded : encodeBatch slotKey (listedImage G) (patchOf G d) = some b)
    {start x : PairedRecovery.State (decisionCodec G d).paths.length}
    (admitted : PairedAdmission G d b start) (prefixTrace : PairedRecovery.Reaches b race start x)
    (pending : x.phase = .pending)
    (choices : ∀ p ∈ b.keys, x.choice p = none ∧ x.disarmed p = false)
    (compatible : ∀ p ∈ b.keys, x.live p = before b p ∨ x.live p = after b p)
    (base : ∀ p, p ∉ b.keys → durableAt x p ((decisionCodec G d).encodeImage (listedImage G) p)) :
    ∃ y, PairedRecovery.Reaches b race x y ∧ y.result = some .applied ∧
      ∀ p, (decisionCodec G d).decode (y.stable p) =
        some (listedImage d.candidate)[(decisionCodec G d).path p]? ∧ y.live p = y.stable p := by
  obtain ⟨y, trace, frame, result, exactWrites⟩ := admitted.recovery race x prefixTrace pending choices compatible
  refine ⟨y, trace, result, fun p => ?_⟩
  by_cases written : p ∈ b.keys
  · exact exactWrites p written
  · obtain ⟨live, stable⟩ := base p written
    obtain ⟨keepLive, keepStable⟩ := frame p written
    refine ⟨?_, (keepLive.trans live).trans (keepStable.trans stable).symm⟩
    rw [keepStable, stable, ← unwritten_same encoded p written]
    exact Codec.decode_encode (Codec.image_supported _)

#print axioms admit_paired_decision
#print axioms paired_recovery_exact_image

end ModelComposition
