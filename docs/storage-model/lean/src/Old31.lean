import Initial31
namespace Storage31
open Storage21 Storage3
set_option maxRecDepth 8192
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false

theorem B_baseline (c : Config P) (x : State P) (a : Storage3.Action P)
    (mutant : c.mutant = "none") (h : Inv c x) (hen : Storage3.enabled c x a = true) :
    BInv (project (Storage3.execute c x a)) := by
  by_cases res : BResidualAction a
  · rcases res with ⟨p,rfl⟩ | ⟨ok,rfl⟩ | rfl | rfl | rfl
    · exact B_observe c mutant (project x) p h.B h.lineage.2.2
    · simp only [Storage3.execute,executeOld,mutant]
      simp only [show ("none" == "MDO") = false from rfl,Bool.false_eq_true,ite_false]
      change BInv (Storage21.execute c (project x) (.deliverUp ok))
      simp only [Storage21.execute]
      cases hq : (project x).s.up with
      | nil => exact h.B
      | cons m tail =>
        have kind := h.protocol.kinds m (by change m ∈ (project x).s.up; simp [hq])
        apply B_delivery_protected4 c mutant (project x) m tail ok h.B h.lineage.2.2 kind
        intro submit held typed _ _
        rcases h.protection m.p held typed with seen | promised | blind
        · exact Or.inl seen
        · exact Or.inr promised
        · exact False.elim (blind.2.1 m (by simp [hq]) submit rfl)
    · exact B_check c mutant (project x) h.B h.lineage.2.2
    · exact B_switchFin c mutant (project x) h.B h.lineage.2.2 hen
    · exact B_launch c (project x) h.B h.inactive hen
  · exact B_nonresidual3 c x a mutant h.B hen res

theorem job_authority (c : Config P) (x : State P) (h : Inv c x)
    (hen : Storage3.enabled c x (.old .rename) = true) : JobAuthority x = true := by
  have inv := h.authority.structural
  have on : x.s.job.on = true := by simp [Storage3.enabled,Storage21.enabled,project] at hen; grind
  have phase : x.s.job.phase = 2 := by simp [Storage3.enabled,Storage21.enabled,project] at hen; grind
  have guarded := h.C.2 on phase
  have base := inv.2.2.2.1 on
  have real := inv.2.2.2.2.2 on phase guarded.1
  rcases base with unknown | equal | auth
  · exact False.elim (real unknown)
  · simp [JobAuthority,beq_iff_eq]; exact Or.inl (Or.inl (Or.inl equal.symm))
  · have actual := (shadow_auth6 x x.s.job.p x.s.job.base).mp auth
    simp [JobAuthority,Auth3_6,List.any_eq_true] at actual ⊢
    grind

theorem inv_baseline (c : Config P) (x : State P) (a : Storage3.Action P)
    (mutant : c.mutant = "none") (noNone : NoNone6 c) (noUnknown : NoUnknown6 c)
    (h : Inv c x) (hen : Storage3.enabled c x a = true) :
    Inv c (Storage3.execute c x a) := by
  refine {
    inactive := inactive_execute3 c x a mutant h.inactive hen
    protocol := protocol_execute3 c x a mutant h.protocol hen
    snapshot := snapshot_execute4 c x a mutant h.snapshot h.inactive hen
    shapes := shapes_execute4 c x a mutant h.shapes h.snapshot hen
    bounds := bounds_execute4 c x a mutant h.bounds
    clear := clear_execute4 c x a mutant h.clear h.snapshot h.inactive hen
    bytes := bytes_execute4 c x a mutant h.bytes hen
    lineage := lineage_execute4 c x a mutant h.lineage h.shapes h.bounds h.snapshot h.clear h.inactive hen
    windowSeen := windowSeen_execute4 c x a mutant h.windowSeen h.bytes hen
    carrierSeen := carrierSeen_execute4 c x a mutant h.carrierSeen h.windowSeen h.bytes hen
    epochs := epochs_execute6 c x a mutant h.epochs
    distinct := distinct_execute6 c x a mutant h.distinct hen
    active := active_execute6 c x a mutant h.active h.lineage h.snapshot h.distinct hen
    fence := fence_execute6 c x a mutant h.fence h.epochs hen
    promiseBytes := promiseBytes_execute6 c x a mutant h.promiseBytes h.bytes hen
    obs := obs_execute6 c x a mutant h.obs h.bytes hen
    protection := protected_execute_s3_5 c x a mutant h.protection h.carrierSeen h.bytes h.protocol.inactive h.lineage hen
    custody := custody_execute6 c x a mutant h.custody h.shapes h.epochs h.lineage h.snapshot h.active h.distinct h.fence h.inactive h.promiseBytes h.bytes noNone hen
    authority := authority_execute6 c x a mutant h.authority h.bounds h.bytes h.windowSeen h.carrierSeen h.obs noUnknown hen
    C := C_execute3 c x a mutant h.C hen
    B := B_baseline c x a mutant h hen
    W := ?_
    taken := taken_execute3 c x a mutant h.taken
    newTags := newTags_execute3 c x a mutant h.newTags hen
    vocabulary := vocabulary_execute3 c x a mutant h.vocabulary
    unheld := unheld_execute3 c x a mutant h.unheld h.protocol
    G := ?_
    trash := trash_execute c x a mutant h.trash
  }
  · by_cases recv : ∃ p, a = .old (.wRecv p)
    · obtain ⟨p,rfl⟩ := recv
      exact W_wRecv_of_fence c mutant (project x) p h.W (lifetime_mailFence (project x) h.protocol.life h.protocol.carriage)
    · exact W_execute3_nonrecv c x a mutant h.W hen (by simpa using recv)
  · by_cases rename : a = .old .rename
    · subst a; exact G_rename_of_authority3 c x h.G (job_authority c x h hen)
    · exact G_nonrename3 c x a mutant h.G rename
end Storage31
