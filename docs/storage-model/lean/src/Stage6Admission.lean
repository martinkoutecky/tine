import Stage6
import Guarantee35Apply

namespace Stage6
open Storage21

theorem keys_iff (b : Batch P) (p : Slot P) : p ∈ b.keys ↔ (b.eff p).isSome = true :=
  Storage33.keys_mem8 b.eff p

theorem move_endpoints_in_keys (b : Batch P) (shape : b.Shape) (m : Move P)
    (member : m ∈ b.moves) : m.src ∈ b.keys ∧ m.dst ∈ b.keys := by
  obtain ⟨source, receiver, _⟩ := shape.2.2 m member
  simp only [keys_iff, source, Option.isSome_some, true_and]
  cases value : b.eff m.dst
  · simp [Storage33.get, value] at receiver
  · rfl

private theorem core_enabled (c : Config P) (x : State P) (b : Batch P)
    (en : enabledApply c x b = true) :
    Storage34.enabledApply c x.admissionView b.eff b.plan none none = true := by
  simp only [enabledApply, Bool.and_eq_true] at en
  exact en.2

theorem enabled_shape (c : Config P) (x : State P) (b : Batch P)
    (en : enabledApply c x b = true) : b.Shape := by
  simp only [enabledApply, Bool.and_eq_true, decide_eq_true_eq] at en
  exact en.1.1

theorem admission_expected (c : Config P) (x : State P) (b : Batch P)
    (normal : c.mutant = "none") (en : enabledApply c x b = true) :
    ∀ p ∈ b.keys, (x.core.s.pages p).held = true ∧ clean (x.core.s.pages p) = true ∧
      b.plan p = some (x.core.s.pages p).buf := by
  have raw := core_enabled c x b en
  simp only [Storage34.enabledApply, Bool.and_eq_true] at raw
  intro p mem
  obtain ⟨held, cleaned, expected⟩ :=
    Storage33.apply_admission8 c (Storage35.project x.admissionView) b.eff b.plan
      none none normal raw.1 p mem
  refine ⟨held, cleaned, ?_⟩
  change b.plan p = some (Storage3.opCur x.core.s p) at expected
  simpa [Storage3.opCur, show (x.core.s.pages p).held = true from held] using expected

theorem admission_projects_batch (c : Config P) (x : State P) (b : Batch P)
    (normal : c.mutant = "none") (inv : Storage31.Inv c x.core) (en : enabledApply c x b = true) :
    (opApply c x b).core = Storage31.batch10 c x.core b.keys (Storage33.get b.eff) := by
  let z := x.admissionView
  have zEn := core_enabled c x b en
  have raw := zEn
  simp only [Storage34.enabledApply, Bool.and_eq_true] at raw
  change Storage32.project (Storage34.project (Storage34.opApply c z b.eff b.plan none none)) = _
  rw [Storage34.apply_refines33_9 c z b.eff b.plan none none
    (Storage34.apply_slot_free9 c z b.eff b.plan none none normal zEn)]
  exact Storage33.apply_eq_batch10 c (Storage35.project z) b.eff b.plan none none normal inv raw.1

theorem admission_preserves_core (c : Config P) (x : State P) (b : Batch P)
    (normal : c.mutant = "none") (inv : Storage31.Inv c x.core) (en : enabledApply c x b = true) :
    Storage31.Inv c (opApply c x b).core :=
  Storage35.apply_inv35_10 c x.admissionView b.eff b.plan none none normal inv (core_enabled c x b en)

theorem admission_frame (c : Config P) (x : State P) (b : Batch P) :
    let y := opApply c x b
    y.core.s.disk = x.core.s.disk ∧ y.core.s.stable = x.core.s.stable ∧
      ∀ p, p ∉ b.keys → y.core.s.pages p = x.core.s.pages p ∧ y.core.s.drafts p = x.core.s.drafts p := by
  refine ⟨rfl, rfl, ?_⟩
  intro p outside
  simp [opApply, State.fromCore, Storage35.opApply, Storage34.opApply, Storage33.opApply,
    Storage34.project, Storage32.commit, Storage32.project, Storage3.commit, Storage21.commit,
    Storage3.fold_put_if, Batch.keys, show p ∉ Storage33.keys b.eff from outside]
  split <;> (try simp only [Storage3.fold_put_if, show p ∉ Storage33.keys b.eff from outside, if_false]) <;>
    exact ⟨rfl, rfl⟩

theorem admission_versions (c : Config P) (x : State P) (b : Batch P)
    (normal : c.mutant = "none") (p : Slot P) (mem : p ∈ b.keys) :
    let y := opApply c x b
    (y.core.s.pages p).held = true ∧ (y.core.s.pages p).buf = Storage33.get b.eff p ∧
      (y.core.s.pages p).ver = version x p ∧ (y.core.s.pages p).risk = true ∧
      (y.core.s.drafts p).bytes = Storage33.get b.eff p ∧ (y.core.s.drafts p).ver = version x p :=
  Storage33.apply_effects_drafted8 c (Storage35.project x.legacyView) b.eff b.plan none none normal p mem

theorem admission_changed_iff (c : Config P) (x : State P) (b : Batch P)
    (normal : c.mutant = "none") (bounds : VersionBounds x.core.s.toSys x.core.g.vc) (p : Slot P) :
    let y := opApply c x b
    (y.core.s.pages p ≠ x.core.s.pages p ∨ y.core.s.drafts p ≠ x.core.s.drafts p) ↔ p ∈ b.keys := by
  constructor
  · intro changed
    by_cases mem : p ∈ b.keys
    · exact mem
    · obtain ⟨page, draft⟩ := (admission_frame c x b).2.2 p mem
      exact False.elim (changed.elim (fun ne => ne page) (fun ne => ne draft))
  · intro mem
    left
    intro same
    have new := (admission_versions c x b normal p mem).2.2.1
    rw [same] at new
    have old := bounds.pages p
    change (x.core.s.pages p).ver ≤ x.core.g.vc at old
    unfold version at new
    omega

theorem admission_gate (c : Config P) (x : State P) (b : Batch P) (moves : b.moves ≠ []) :
    ∃ gt, (opApply c x b).gates = x.gates ++ [gt] ∧ gt.moves = b.moves ∧ gt.pending = b.keys ∧
      ∀ p, gt.vers p = (b.eff p).map (fun _ => version x p) := by
  have nonempty : b.moves.isEmpty = false := List.isEmpty_eq_false_iff.mpr moves
  refine ⟨⟨b.moves, fun p => if b.keys.contains p then some (version x p) else none, b.keys⟩,
    ?_, rfl, rfl, ?_⟩
  · change install x b.moves b.keys = _
    simp only [install, nonempty, Bool.false_eq_true, if_false]
  · intro p
    cases value : b.eff p <;> simp [keys_iff, value]

#print axioms enabled_shape
#print axioms move_endpoints_in_keys
#print axioms admission_expected
#print axioms admission_projects_batch
#print axioms admission_preserves_core
#print axioms admission_frame
#print axioms admission_versions
#print axioms admission_changed_iff
#print axioms admission_gate

end Stage6
