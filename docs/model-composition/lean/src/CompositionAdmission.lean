import CompositionCodec
import CutPatch
import ArbitraryInitialDisk

namespace ModelComposition
open ListedDisk Storage21 CM CM.Cut

def crossBindings (patch : Patch) : List Binding :=
  patch.bindings.filter (fun b => b.origin != b.destination)

/-- The slot key is the host's physical path identity on the listed support. -/
def representable (slotKey : Path → String) (image : Image) (patch : Patch) : Bool :=
  decide (((Codec.forPatch image patch).paths.map slotKey).Nodup) && !patch.effects.isEmpty &&
    match crossBindings patch with
    | [] => true
    | [b] =>
      match patch.effects[b.origin]?, patch.effects[b.destination]? with
      | some s, some d => s.before.isSome && s.after.isNone && d.after.isSome
      | _, _ => false
    | _ => false

theorem physical_slots_injective (slotKey : Path → String) (image : Image) (patch : Patch)
    (fits : representable slotKey image patch = true) :
    let k := Codec.forPatch image patch
    Function.Injective (fun p => slotKey (k.path p)) := by
  let k := Codec.forPatch image patch
  have unique : (k.paths.map slotKey).Nodup := by
    simp only [representable, Bool.and_eq_true, decide_eq_true_eq] at fits
    exact fits.1.1
  change Function.Injective (fun p => slotKey (k.path p))
  intro a b eq
  apply Fin.ext
  have ha : a.val < (k.paths.map slotKey).length := by simp
  have hb : b.val < (k.paths.map slotKey).length := by simp
  exact (List.getElem_inj unique).mp (show (k.paths.map slotKey)[a.val] = (k.paths.map slotKey)[b.val] by
    simpa [Codec.path] using eq)

def effectMap (k : Codec) (patch : Patch) (entry : Effect → Entry) : Storage33.Map k.paths.length :=
  fun p => patch.effects[k.path p]?.map (fun e => k.encode (entry e))

/-- Interpret the action as the full patch when `representable` holds. -/
def admission (k : Codec) (patch : Patch) : Storage35.Action k.paths.length :=
  let move := (crossBindings patch).head?
  .opApply (effectMap k patch Effect.after) (effectMap k patch Effect.before)
    (move.bind (fun b => k.slot? b.destination)) (move.bind (fun b => k.slot? b.origin))

def DiskAgreement (image : Image) (patch : Patch) (x : Storage35.State (Codec.forPatch image patch).paths.length) : Prop :=
  let k := Codec.forPatch image patch
  ∀ p, patch.effects[k.path p]?.isSome = true → x.s.disk p = k.encodeImage image p

private theorem designated_slots {slotKey : Path → String} {image : Image} {patch : Patch} {b : Binding}
    (fits : representable slotKey image patch = true) (binding : crossBindings patch = [b]) :
    let k := Codec.forPatch image patch
    ∃ s d, s ≠ d ∧ k.slot? b.origin = some s ∧ k.slot? b.destination = some d ∧
      admission k patch = .opApply (effectMap k patch Effect.after) (effectMap k patch Effect.before)
        (some d) (some s) := by
  have different : b.origin ≠ b.destination := by
    have member : b ∈ crossBindings patch := by rw [binding]; simp
    simpa [crossBindings] using (List.mem_filter.mp member).2
  simp only [representable, binding, Bool.and_eq_true] at fits
  cases hs : patch.effects[b.origin]? <;> cases hd : patch.effects[b.destination]? <;>
    simp only [hs, hd, Bool.false_eq_true] at fits
  · exact False.elim fits.2
  · exact False.elim fits.2
  · exact False.elim fits.2
  · obtain ⟨s, hs⟩ := Codec.slot_of_effect (image := image) hs
    obtain ⟨d, hd⟩ := Codec.slot_of_effect (image := image) hd
    exact ⟨s, d, fun eq => different ((Codec.slot_spec hs).symm.trans (eq ▸ Codec.slot_spec hd)),
      hs, hd, by simp [admission, binding, hs, hd]⟩

private theorem apply_frame (c : Config P) (x : Storage35.State P) (eff plan : Storage33.Map P)
    (dst src : Option (Fin P)) (p : Fin P) (outside : p ∉ Storage33.keys eff) :
    let y := Storage35.opApply c x eff plan dst src
    y.s.pages p = x.s.pages p ∧ y.s.drafts p = x.s.drafts p := by
  simp [Storage34.opApply, Storage33.opApply, Storage34.project, Storage32.commit, Storage3.commit,
    Storage21.commit, Storage3.fold_put_if, outside]
  split <;> simp [Storage3.fold_put_if, outside]

def coreState (x : Storage35.State P) : Storage21.State P :=
  Storage3.project (Storage32.project (Storage35.project x))

structure EffectAdmission (k : Codec) (c : Config k.paths.length)
    (x y : Storage35.State k.paths.length) (p : k.Slot) (before after : Entry) : Prop where
  held : (x.s.pages p).held = true
  buffer : (x.s.pages p).buf = k.encode before
  base : (x.s.pages p).base = k.encode before
  disk : y.s.disk p = k.encode before
  draft : k.decode (y.s.drafts p).bytes = some after
  owed : NeedsDraft c (coreState y) p
  recoverable : Covers (coreState y) p (y.s.drafts p).bytes
  draftVersion : (y.s.drafts p).ver = x.g.vc + 1 + p.val
  promiseVersion : (y.g.promise p).ver = x.g.vc + 1 + p.val

variable {K : Type} [DecidableEq K]
variable {O : Observations} {key : String → K} {url : String → Bool} {D : Date13.Dates K}
variable {E : Engine} {G : Graph} {r : Request} {d : Done}

abbrev decisionCodec (G : Graph) (d : Done) := Codec.forPatch (listedImage G) (patchOf G d)

theorem admit_decision (success : decideRequest O key url D E G r = .success d)
    (slotKey : Path → String) (fits : representable slotKey (listedImage G) (patchOf G d) = true)
    (c : Config (decisionCodec G d).paths.length) (initial : (decisionCodec G d).Slot → Int)
    (x : Storage35.State (decisionCodec G d).paths.length)
    (mutant : c.mutant = "none") (realDisk : ∀ p, RealByte c (initial p))
    (textNondelete : absent ∉ c.texts) (textNoNone : noneBytes ∉ c.texts)
    (textNoUnknown : unknown ∉ c.texts) (reachable : Storage35.Reachable c initial x)
    (diskMatches : DiskAgreement (listedImage G) (patchOf G d) x)
    (enabled : Storage35.enabled c x (admission (decisionCodec G d) (patchOf G d)) = true) :
    let patch := patchOf G d
    let k := decisionCodec G d
    let a := admission k patch
    let y := Storage35.execute c x a
    ListedLayout d.candidate (listedImage d.candidate) ∧
    (∀ path e, patch.effects[path]? = some e → ∃ p, k.slot? path = some p ∧
      EffectAdmission k c x y p (listedImage G)[path]? (listedImage d.candidate)[path]?) ∧
    (∀ p, patch.effects[k.path p]? = none →
      y.s.pages p = x.s.pages p ∧ y.s.drafts p = x.s.drafts p) ∧
    y.s.disk = x.s.disk ∧ y.s.stable = x.s.stable ∧
    (∀ b, crossBindings patch = [b] →
      ∃ s t, s ≠ t ∧ k.slot? b.origin = some s ∧ k.slot? b.destination = some t ∧
        a = .opApply (effectMap k patch Effect.after) (effectMap k patch Effect.before) (some t) (some s)) ∧
    Storage35.step c x a = some y ∧ Storage35.Reachable c initial y ∧
    Storage35.guarantee y = true ∧ Storage35.orderHolds y = true ∧ Storage35.renameResolves y = true := by
  let image := listedImage G
  let patch := patchOf G d
  let k := decisionCodec G d
  let eff := effectMap k patch Effect.after
  let plan := effectMap k patch Effect.before
  let dst := (crossBindings patch).head?.bind (fun b => k.slot? b.destination)
  let src := (crossBindings patch).head?.bind (fun b => k.slot? b.origin)
  let y := Storage35.opApply c x eff plan dst src
  have en : Storage34.enabledApply c x eff plan dst src = true := enabled
  have en33 : Storage33.enabledApply c (Storage35.project x) eff plan dst src = true := by
    simp only [Storage34.enabledApply, Bool.and_eq_true] at en
    exact en.1
  have step : Storage35.step c x (admission k patch) = some y := by
    change Storage35.enabled c x (admission k patch) = true at enabled
    simp only [Storage35.step, enabled, if_true]
    rfl
  have next : Storage35.Reachable c initial y := .advance reachable (admission k patch) step
  have inv := Storage35.inv_reachable35_10 c initial x mutant realDisk
    (by intro b hb h; rcases hb with eq | mem
        · exact (by decide : absent ≠ noneBytes) (eq.symm.trans h)
        · exact textNoNone (h ▸ mem))
    (by intro b hb h; rcases hb with eq | mem
        · exact (by decide : absent ≠ unknown) (eq.symm.trans h)
        · exact textNoUnknown (h ▸ mem)) reachable
  have names := done_name_safety (readingInputs O key url) D success
  refine ⟨listedImage_layout names.2.wellFormed, ?_, ?_, rfl, rfl,
    fun b hb => designated_slots fits hb, step, next,
    Storage35.all_targets_arbitrary_initial_disk c initial y mutant realDisk
      textNondelete textNoNone textNoUnknown next⟩
  · intro path e he
    obtain ⟨p, hp⟩ := Codec.slot_of_effect (image := image) he
    have pathEq : k.path p = path := Codec.slot_spec hp
    have effect : patch.effects[k.path p]? = some e := by rw [pathEq]; exact he
    have hEff : eff p = some (k.encode e.after) := by simp [eff, effectMap, effect]
    have hPlan : plan p = some (k.encode e.before) := by simp [plan, effectMap, effect]
    have mem := (Storage33.keys_mem8 eff p).mpr (by simp [hEff])
    obtain ⟨held, cleaned, planned⟩ := Storage33.apply_admission8 c (Storage35.project x) eff plan dst src mutant en33 p mem
    change (x.s.pages p).held = true at held
    have buffer : (x.s.pages p).buf = k.encode e.before := by
      rw [hPlan] at planned
      simpa [Storage3.opCur, Storage34.project, held] using (Option.some.inj planned).symm
    have base : (x.s.pages p).base = k.encode e.before := by
      simp [clean] at cleaned
      exact cleaned.1.2.symm.trans buffer
    have beforeAfter : e = ⟨image[path]?, (listedImage d.candidate)[path]?⟩ := by
      rw [patchOf_effect] at he
      split at he
      · contradiction
      · exact (Option.some.inj he).symm
    have refines := Storage34.apply_refines33_9 c x eff plan dst src (Storage34.apply_slot_free9 c x eff plan dst src mutant en)
    obtain ⟨on, bytes, ver, unsaved, draft, draftVer⟩ := Storage33.apply_new_custody8 c
      (Storage35.project x) eff plan dst src mutant p mem (inv.lineage.1 p)
    rw [← refines] at on bytes ver unsaved draft draftVer
    change (y.g.promise p).bytes = Storage33.get eff p at bytes
    change (y.g.promise p).saved = false at unsaved
    change (y.s.drafts p).bytes = (y.g.promise p).bytes at draft
    have decode : k.decode (y.s.drafts p).bytes = some e.after := by
      rw [draft, bytes]
      simpa [Storage33.get, hEff] using Codec.decode_encode (Codec.effect_supported (image := image) he).2
    have owed : NeedsDraft c (coreState y) p :=
      ⟨⟨on, fun expired => Bool.noConfusion (unsaved.symm.trans expired.1)⟩, Or.inl unsaved⟩
    have disk : x.s.disk p = k.encodeImage image p :=
      diskMatches p (show (patch.effects[k.path p]?).isSome = true by rw [effect]; rfl)
    refine ⟨p, hp, held, ?_, ?_, ?_, ?_, owed, Or.inl draft, draftVer.trans ver, ver⟩
    · simpa only [beforeAfter] using buffer
    · simpa only [beforeAfter] using base
    · change x.s.disk p = k.encode image[path]?
      simpa only [Codec.encodeImage, pathEq] using disk
    · change k.decode (y.s.drafts p).bytes = some (listedImage d.candidate)[path]?
      simpa only [beforeAfter] using decode
  · intro p outside
    change patch.effects[k.path p]? = none at outside
    exact apply_frame c x eff plan dst src p (by simp [Storage33.keys_mem8, eff, effectMap, outside])

#print axioms admit_decision

end ModelComposition
