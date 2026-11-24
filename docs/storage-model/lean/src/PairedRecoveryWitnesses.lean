import PairedRecoveryProgress
import Stage6RecoveryWitness

namespace PairedRecovery.Witnesses
open Storage21
open Stage6.Witnesses (disk s₂ d₂)

private def pair : Stage6.Move 5 := ⟨d₂, s₂⟩

private def batch : Stage6.Batch 5 :=
  ⟨fun p => if p = s₂ then some absent else if p = d₂ then some 20 else none,
    fun p => some (disk p), [pair]⟩

private theorem shape : batch.Shape := by decide
private theorem member : pair ∈ batch.moves := by decide
private theorem either_key {p : Fin 5} (key : p ∈ batch.keys) : p = s₂ ∨ p = d₂ := by
  rw [Stage6.keys_iff] at key
  by_cases src : p = s₂
  · exact Or.inl src
  · right
    by_cases dst : p = d₂
    · exact dst
    · simp [batch, src, dst] at key

private theorem dst_turn (x : State 5) : fileTurn batch x d₂ :=
  ⟨(Stage6.move_endpoints_in_keys batch shape pair member).2,
    fun src => source_not_receiver shape src ⟨pair, member, rfl⟩, Or.inl ⟨pair, member, rfl⟩⟩

private theorem source_turn (x : State 5) (seen : (x.seen d₂).isSome) : sourceTurn batch x := by
  intro p key notSource
  rcases either_key key with rfl | rfl
  · exact False.elim (notSource ⟨pair, member, rfl⟩)
  · exact seen

private theorem all_seen (x : State 5) (src : (x.seen s₂).isSome)
    (dst : (x.seen d₂).isSome) : AllSeen batch x := by
  intro p key
  rcases either_key key with rfl | rfl
  · exact src
  · exact dst

private def start := init batch disk disk
private def down := powerCut start (fun _ => false) (fun _ => false)
private def launched := reset down true
private def chosen := choose batch launched d₂ .disk
private def saved := reset (reset chosen false) true

private theorem choice_at_relaunch : Reaches batch false start saved := by
  have cut : Reaches batch false start down := .next .here (.cut _ _)
  have launch : Reaches batch false start launched := .next cut (.launch rfl)
  have chooseStep : Step batch false launched chosen := by
    apply Step.decision (by constructor <;> rfl) (dst_turn launched).1
    intro _ _ eq
    cases eq
  exact .next (.next (.next launch chooseStep) .pause) (.launch rfl)

private def receiverSeen (x : State 5) :=
  {x with seen := put x.seen d₂ (some ⟨x.live d₂ == after batch d₂, false⟩)}

private def keptSeen (x : State 5) :=
  {x with
    seen := put x.seen s₂ (some ⟨false, decide (x.live s₂ = before batch s₂ ∨
      (x.live d₂ = after batch d₂ ∧ x.stable d₂ = after batch d₂) ∨ x.deleted s₂ = true)⟩)
    permit := none}

private theorem checked_take_disk : Step batch false saved (receiverSeen saved) :=
  .checkFile ⟨rfl, rfl⟩ (dst_turn saved) trivial rfl

private theorem checked_source_kept : Step batch false (receiverSeen saved) (keptSeen (receiverSeen saved)) :=
  .checkKeptSource ⟨rfl, rfl⟩ member (Or.inl rfl) (fun h => h.2.2.1 rfl) rfl
    (fun _ _ h => by cases h) rfl

theorem direct_discard_survives_relaunch :
    ∃ x, Reaches batch false start x ∧ x.result = some .cancelled ∧ x.live s₂ = 20 ∧
      x.live d₂ = absent ∧ x.stableTrash s₂ = [] := by
  let x := keptSeen (receiverSeen saved)
  have reached : Reaches batch false start x := .next (.next choice_at_relaunch checked_take_disk) checked_source_kept
  have standing : Standing x pair := ⟨⟨false, true⟩, rfl, rfl⟩
  have applied : ¬Applied batch x := fun h => (h.2 pair member).1 (Or.inl rfl)
  have cancelled : Cancelled batch x := by
    refine ⟨?_, ?_⟩
    · intro m hm
      have eq : m = pair := by simpa [batch] using hm
      subst m
      exact ⟨rfl, rfl, rfl, standing⟩
    · intro p key ns nr
      rcases either_key key with rfl | rfl
      · exact False.elim (ns ⟨pair, member, rfl⟩)
      · exact False.elim (nr ⟨pair, member, rfl⟩)
  have retire : Step batch false x
      {x with phase := .retired .cancelled, permit := none, preimage := fun _ => none} :=
    .retire ⟨rfl, rfl⟩ (all_seen x rfl rfl) (.cancelled applied cancelled) (fun m hm => (cancelled.1 m hm).2.2.2)
  exact ⟨_, .next (.next (.next reached retire) .pause) (.launch rfl), rfl, rfl, rfl, rfl⟩

/-- This mutant forgets only the durable choice at launch. With the order
intact it can republish the cancelled receiver and remove its source. -/
theorem forgetting_cancellation_revives_removal :
    ∃ u v w, Reaches batch false start u ∧ disarmed u pair ∧
      ¬Reaches batch false u v ∧
      Reaches batch false v w ∧ w.result = some .applied ∧ w.live s₂ = absent := by
  let forgotten := {saved with
    choice := fun _ => none
    disarmed := fun _ => false
    tookDisk := fun _ => false}
  have on : disarmed saved pair := Or.inl rfl
  have resetIsIllegal (trace : Reaches batch false saved forgotten) : False := by
    cases cancellation_survives trace on with
    | inl h => cases h
    | inr h => cases h
  have clean : Clean batch forgotten := by
    refine ⟨⟨rfl, rfl⟩, fun _ _ => ⟨rfl, rfl⟩, fun p _ => Or.inl ?_⟩
    rfl
  obtain ⟨y, replay, _, result, done⟩ := clean_completion (race := false) shape clean
  exact ⟨saved, forgotten, y, choice_at_relaunch, on, resetIsIllegal, replay, result,
    (done s₂ (Stage6.move_endpoints_in_keys batch shape pair member).1).1⟩

set_option maxRecDepth 4096 in
theorem legacy_singleton_breaks_pair_coverage :
    let x := Stage6.Witnesses.singletonFinal
    Storage35.Reachable Stage6.Witnesses.config disk x ∧
      20 ∈ x.s.trashStable s₂ ∧
      ¬Covered 20 20 x.s.disk none pair ∧
      (∀ p, (x.s.drafts p).bytes ≠ 20) := by
  exact ⟨Stage6.Witnesses.singleton_relaunch_discard.2.2.1, by decide,
    by simp [Covered, pair, Stage6.Witnesses.singletonFinal] <;> decide, by decide⟩

set_option maxRecDepth 4096 in
theorem legacy_batch_breaks_pair_coverage :
    let x := Stage6.Witnesses.batchFinal
    Stage6.Reachable Stage6.Witnesses.config disk x ∧
      20 ∈ x.core.s.trashStable s₂ ∧
      ¬Covered 20 20 x.core.s.disk none pair ∧
      (∀ p, (x.core.s.drafts p).bytes ≠ 20) := by
  exact ⟨Stage6.Witnesses.batch_relaunch_discard.2.2.2.2.2.2.2.2.1, by decide,
    by simp [Covered, pair, Stage6.Witnesses.batchFinal] <;> decide, by decide⟩

private def written :=
  {start with live := put start.live d₂ 20, seen := invalidate batch start d₂}
private def synced := {written with stable := put written.stable d₂ 20}
private def preflight := {receiverSeen synced with permit := some pair}
private def replaced (x : State 5) (p : Fin 5) (bytes : Int) :=
  {x with live := put x.live p bytes, stable := put x.stable p bytes}
private def moved := trashFile (replaced preflight d₂ 31) s₂
private def sourceSynced := {moved with stable := put moved.stable s₂ absent}
private def trashSynced := {sourceSynced with stableTrash := put moved.stableTrash s₂ [20]}
private def afterRace := reset (powerCut trashSynced (fun _ => false) (fun _ => false)) true

private theorem raced : Reaches batch false start afterRace := by
  have write := Step.write (b := batch) (racySource := false) (x := start) (bytes := 20)
    ⟨rfl, rfl⟩ (dst_turn start) ⟨rfl, Or.inl rfl⟩
  have fileSynced := Step.syncFile (racySource := false) ⟨rfl, rfl⟩ (dst_turn written) rfl
  have checked := Step.checkFile (racySource := false) ⟨rfl, rfl⟩ (dst_turn synced) rfl rfl
  have checkedPair : Step batch false (receiverSeen synced) preflight :=
    .check ⟨rfl, rfl⟩ member (source_turn _ rfl) (fun h => h.elim Bool.noConfusion Bool.noConfusion)
      ⟨rfl, rfl⟩ rfl (by decide)
  have replacedDst : Step batch false preflight (replaced preflight d₂ 31) := .outside d₂ 31 true
  have movedSource : Step batch false (replaced preflight d₂ 31) moved :=
    .trash ⟨rfl, rfl⟩ member (fun h => h.elim Bool.noConfusion Bool.noConfusion) rfl (Or.inr rfl) (by decide)
  have fileBarrier : Step batch false moved sourceSynced :=
    .syncSource ⟨rfl, rfl⟩ member rfl (Or.inl (fun h => h.elim Bool.noConfusion Bool.noConfusion))
  have trashBarrier : Step batch false sourceSynced trashSynced := .syncTrash ⟨rfl, rfl⟩ member
  exact .next (.next (.next (.next (.next (.next (.next (.next (.next (.next .here write)
    fileSynced) checked) checkedPair) replacedDst) movedSource) fileBarrier) trashBarrier) (.cut _ _)) (.launch rfl)

private theorem take_disk (x : State 5) (ready : x.ready) : Step batch false x (choose batch x d₂ .disk) :=
  .decision ready (dst_turn x).1 (fun _ _ h => by cases h)

theorem lost_receiver_restores_free_source :
    ∃ x, Reaches batch false start x ∧ durableAt x s₂ 20 ∧ x.live d₂ = 31 ∧ disarmed x pair := by
  let chosen := choose batch afterRace d₂ .disk
  let restored := {chosen with live := put chosen.live s₂ 20, seen := put chosen.seen s₂ none, permit := none}
  have restore : Step batch false chosen restored := by
    apply Step.restore rfl rfl member _ rfl
    refine ⟨Or.inl rfl, rfl, by decide, rfl, ?_, ?_⟩
    · intro h
      have bad : (31 : Int) = 20 := h.1
      omega
    · intro _ _ h
      cases h
  have sync : Step batch false restored {restored with stable := put restored.stable s₂ 20} :=
    .syncKeptSource rfl rfl member (Or.inl rfl)
  exact ⟨_, .next (.next (.next raced (take_disk afterRace ⟨rfl, rfl⟩)) restore) sync,
    ⟨rfl, rfl⟩, rfl, Or.inl rfl⟩

theorem lost_receiver_occupied_source_retains_conflict :
    ∃ x, Reaches batch false start x ∧ x.result = some .conflict ∧ x.live s₂ = 10 ∧
      x.live d₂ = 31 ∧ x.preimage s₂ = some 20 ∧ 20 ∈ x.stableTrash s₂ := by
  let claim := replaced afterRace s₂ 10
  let chosen := choose batch claim d₂ .disk
  let y := keptSeen (receiverSeen chosen)
  have occupied : Reaches batch false start claim := .next raced (.outside s₂ 10 true)
  have dest : Step batch false chosen (receiverSeen chosen) :=
    .checkFile ⟨rfl, rfl⟩ (dst_turn chosen) trivial rfl
  have source : Step batch false (receiverSeen chosen) y :=
    .checkKeptSource ⟨rfl, rfl⟩ member (Or.inl rfl) (fun h => by cases h.2.2.2.1) rfl
      (fun _ _ h => by cases h) rfl
  have applied : ¬Applied batch y := fun h => (h.2 pair member).1 (Or.inl rfl)
  have cancelled : ¬Cancelled batch y := fun h => by cases (h.1 pair member).2.1
  have missing : ¬Standing y pair := by
    rintro ⟨o, h, active⟩
    have e : some (⟨false, false⟩ : Observation) = some o := h
    cases Option.some.inj e
    cases active
  have retain : Step batch false y {y with phase := .retainedConflict, permit := none} :=
    .retainConflict ⟨rfl, rfl⟩ (all_seen y rfl rfl) (.conflict applied cancelled) ⟨pair, member, missing⟩
  exact ⟨_, .next (.next (.next (.next (.next (.next occupied (take_disk claim ⟨rfl, rfl⟩))
    dest) source) retain) .pause) (.launch rfl), rfl, rfl, rfl, rfl, by decide⟩

open Stage6.Witnesses (namespaceBatch s₁ d₁ referrer)

theorem two_moves_resume_after_first_receiver :
    let first := init namespaceBatch disk disk
    ∃ u y, Reaches namespaceBatch false first u ∧ u.up = false ∧
      u.live d₁ = 10 ∧ u.live d₂ = absent ∧ u.live referrer = 30 ∧
      Reaches namespaceBatch false u y ∧ y.result = some .applied ∧
      y.stable s₁ = absent ∧ y.stable s₂ = absent ∧ y.stable d₁ = 10 ∧ y.stable d₂ = 20 ∧
      y.stable referrer = 31 := by
  let first := init namespaceBatch disk disk
  let m : Stage6.Move 5 := ⟨d₁, s₁⟩
  have shape : namespaceBatch.Shape := by decide
  have mem : m ∈ namespaceBatch.moves := by decide
  have dst : receiver namespaceBatch d₁ := ⟨m, mem, rfl⟩
  let published :=
    {first with live := put first.live d₁ 10, seen := invalidate namespaceBatch first d₁}
  have write : Step namespaceBatch false first published :=
    .write ⟨rfl, rfl⟩ ⟨(Stage6.move_endpoints_in_keys _ shape m mem).2,
      fun src => source_not_receiver shape src dst, Or.inl dst⟩ ⟨rfl, Or.inl rfl⟩
  let u := powerCut published (fun _ => true) (fun _ => false)
  have cut : Reaches namespaceBatch false first u := .next (.next .here write) (.cut _ _)
  have compatible : ∀ p ∈ namespaceBatch.keys, u.live p = before namespaceBatch p ∨
      u.live p = after namespaceBatch p := by
    intro p _
    by_cases selected : p = d₁
    · subst p; exact Or.inr rfl
    · left; simp [u, published, first, powerCut, init, put, selected, namespaceBatch, Storage33.get]
  obtain ⟨y, finished, _, result, files⟩ := clean_relaunch_completion (race := false) shape
    (x := u) rfl (fun _ _ => ⟨rfl, rfl⟩) compatible
  exact ⟨u, y, cut, rfl, rfl, rfl, rfl, finished, result,
    (files s₁ (by decide)).2, (files s₂ (by decide)).2,
    (files d₁ (by decide)).2, (files d₂ (by decide)).2, (files referrer (by decide)).2⟩

#print axioms direct_discard_survives_relaunch
#print axioms forgetting_cancellation_revives_removal
#print axioms legacy_singleton_breaks_pair_coverage
#print axioms legacy_batch_breaks_pair_coverage
#print axioms lost_receiver_restores_free_source
#print axioms lost_receiver_occupied_source_retains_conflict
#print axioms two_moves_resume_after_first_receiver

end PairedRecovery.Witnesses
