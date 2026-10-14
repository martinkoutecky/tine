import ACustody6
namespace Storage3
open Storage21
set_option maxRecDepth 8192
set_option maxHeartbeats 4000000
set_option linter.unusedSimpArgs false

def NoNone6 (c : Config P) : Prop := ∀ b, RealByte c b → b ≠ noneBytes

theorem positiveTexts_noNone6 (c : Config P) (T : Nat) (texts : c.texts = positiveTexts4 T) : NoNone6 c := by
  intro b hb
  rcases hb with rfl | hb
  · decide
  · rw [texts] at hb
    obtain ⟨n,hn,rfl⟩ := List.mem_map.mp hb
    simp [noneBytes]; omega

theorem custody_launch6 (c : Config P) (x : State P) (h : Custody c (project x))
    (inactive : InactivePages (project x)) (pb : PromiseBytes c x.g.toGhost)
    (noNone : NoNone6 c) (hen : enabled c x (.old .launch) = true) :
    Custody c (project (execute c x (.old .launch))) := by
  have dead : x.s.alive = false := by simpa [enabled,Storage21.enabled,project] using hen
  simp only [execute,executeOld,lift,project,Storage21.execute,Storage21.launch]
  apply custody_commit
  · intro p need
    have oldneed : NeedsDraft c (project x) p := need
    have eq := (h p oldneed).1
    have nonempty : (x.s.drafts p).bytes ≠ noneBytes := by change ((project x).s.drafts p).bytes ≠ noneBytes; rw [eq]; exact noNone _ (pb p oldneed.1.1)
    refine ⟨eq,?_⟩
    intro _; simp [restored,nonempty]
  · exact postPromises_unheld_old (project x) _ _ (inactive dead)

theorem custody_old6 (c : Config P) (x : State P) (a : Storage21.Action P)
    (mutant : c.mutant = "none") (h : Custody c (project x))
    (shape : Shapes x.s.toSys) (epochs : EpochBounds x.s.toSys x.g.toGhost)
    (lineage : PromiseLineage (project x)) (snap : JobSnapshot x.s.toSys)
    (agreement : ActiveAgreement x.s.toSys x.g.toGhost)
    (distinct : QueueDistinct x.s.toSys) (fence : PublicationFence x.s.toSys x.g.toGhost)
    (inactive : InactivePages (project x)) (pb : PromiseBytes c x.g.toGhost)
    (bytes : ByteDomain c x.s.toSys) (noNone : NoNone6 c)
    (hen : enabled c x (.old a) = true) : Custody c (project (execute c x (.old a))) := by
  by_cases closed : custodyPending a = false
  · have core := custody_execute_closed c mutant (project x) a h shape epochs closed
    simpa only [Custody,NeedsDraft,LivePromise,Covers,project,old_sys6 c x a mutant,
      old_pages6 c x a mutant,old_promise4 c x a mutant,old_ext6 c x a mutant,old_wrote6 c x a mutant] using core
  · cases a <;> simp only [custodyPending] at closed
    case launch => exact custody_launch6 c x h inactive pb noNone hen
    case deliverUp ok =>
      simpa [execute,executeOld,mutant,lift,project] using custody_delivery c mutant (project x) ok lineage.1 distinct h
    case rename =>
      have core := custody_rename c mutant (project x) snap lineage agreement h hen
      simpa only [Custody,NeedsDraft,LivePromise,Covers,project,old_sys6 c x .rename mutant,
        old_pages6 c x .rename mutant,old_promise4 c x .rename mutant,
        old_ext6 c x .rename mutant,old_wrote6 c x .rename mutant,Storage21.execute] using core
    case dirSync ok =>
      have core := custody_dirSync c mutant (project x) ok snap lineage agreement fence h hen
      simpa only [Custody,NeedsDraft,LivePromise,Covers,project,old_sys6 c x (.dirSync ok) mutant,
        old_pages6 c x (.dirSync ok) mutant,old_promise4 c x (.dirSync ok) mutant,
        old_ext6 c x (.dirSync ok) mutant,old_wrote6 c x (.dirSync ok) mutant,Storage21.execute] using core
    case draftSync p =>
      simpa [execute,executeOld,lift,project,Storage21.execute] using custody_draftSync c mutant (project x) lineage agreement
        (fun p => noNone _ (bytes.pages p)) h hen
    all_goals contradiction

theorem custody_execute6 (c : Config P) (x : State P) (a : Action P)
    (mutant : c.mutant = "none") (h : Custody c (project x))
    (shape : Shapes x.s.toSys) (epochs : EpochBounds x.s.toSys x.g.toGhost)
    (lineage : PromiseLineage (project x)) (snap : JobSnapshot x.s.toSys)
    (agreement : ActiveAgreement x.s.toSys x.g.toGhost)
    (distinct : QueueDistinct x.s.toSys) (fence : PublicationFence x.s.toSys x.g.toGhost)
    (inactive : InactivePages (project x)) (pb : PromiseBytes c x.g.toGhost)
    (bytes : ByteDomain c x.s.toSys) (noNone : NoNone6 c)
    (hen : enabled c x a = true) : Custody c (project (execute c x a)) := by
  cases a with
  | old a => exact custody_old6 c x a mutant h shape epochs lineage snap agreement distinct fence inactive pb bytes noNone hen
  | opDelete p => exact custody_delete6 c x p mutant h lineage
  | opRename src dst refs rt => exact custody_opRename6 c x src dst refs rt mutant h lineage hen
  | flushDel p =>
    simp only [execute,flushDel,lift,project,Storage21.flush]
    apply custody_kept_commit c (project x) _ _ _ h
    all_goals first | rfl | (intro _ hp; exact hp) | exact postPromises_same_pages (project x) _ _ rfl

theorem custody_reachable6 (c : Config P) (disk : Fin P → Int) (x : State P)
    (mutant : c.mutant = "none") (realDisk : ∀ p, RealByte c (disk p)) (noNone : NoNone6 c)
    (hr : Reachable c disk x) : Custody c (project x) := by
  induction hr with
  | initial => simp [Custody,NeedsDraft,LivePromise,initWith,embed,project,Storage21.init]
  | @advance x y hr a hs ih =>
    unfold step at hs; split at hs
    · cases hs; exact custody_execute6 c x a mutant ih (shapes_reachable4 c disk x mutant hr)
        (epochs_reachable6 c disk x mutant hr) (lineage_reachable4 c disk x mutant hr)
        (snapshot_reachable4 c disk x mutant hr) (active_reachable6 c disk x mutant hr)
        (distinct_reachable6 c disk x mutant hr) (fence_reachable6 c disk x mutant hr)
        (inactive_reachable3 c disk x mutant hr) (promiseBytes_reachable6 c disk x mutant realDisk hr)
        (bytes_reachable4 c disk x mutant realDisk hr) noNone (by assumption)
    · contradiction

theorem noLoss_reachable6 (c : Config P) (disk : Fin P → Int) (x : State P)
    (mutant : c.mutant = "none") (realDisk : ∀ p, RealByte c (disk p)) (noNone : NoNone6 c)
    (hr : Reachable c disk x) : noLoss x = true :=
  noLoss_of_custody c (project x) (custody_reachable6 c disk x mutant realDisk noNone hr)

theorem A_positiveTexts6 (c : Config P) (T : Nat)
    (texts : c.texts = positiveTexts4 T) (two : 2 ≤ T) (mutant : c.mutant = "none")
    (x : State P) (hr : Reachable c (initDisk c) x) : noLoss x = true :=
  noLoss_reachable6 c (initDisk c) x mutant (positiveTexts_initial_real4 c T texts two)
    (positiveTexts_noNone6 c T texts) hr

theorem guarantee_remaining6 (c : Config P) (T : Nat)
    (texts : c.texts = positiveTexts4 T) (two : 2 ≤ T) (mutant : c.mutant = "none")
    (x : State P) (hr : Reachable c (initDisk c) x) :
    guarantee x = true ↔ "G-overwrote-unseen" ∉ x.g.bad := by
  rw [guarantee_remaining5 c (initDisk c) x mutant (positiveTexts_initial_real4 c T texts two) hr]
  simp [A_positiveTexts6 c T texts two mutant x hr]

end Storage3
