import Stage6Witnesses
import Stage6Agreement
import Stage6CoreSafety

namespace Stage6
namespace Witnesses
open Storage21

private def singletonEff : WriteMap 5 :=
  fun p => if p = s₂ then some absent else if p = d₂ then some 20 else none

private def singletonAction : Storage35.Action 5 :=
  .opApply singletonEff (fun p => some (disk p)) (some d₂) (some s₂)

private def legacyRun (x : Storage35.State 5) (a : Storage35.Action 5) :=
  Storage35.execute config x a

private theorem legacyAdvance {x : Storage35.State 5} (h : Storage35.Reachable config disk x)
    (a : Storage35.Action 5) (en : Storage35.enabled config x a = true) :
    Storage35.Reachable config disk (legacyRun x a) :=
  .advance h a (by simp [Storage35.step, en, legacyRun])

private theorem advance {x : Stage6.State 5} (h : Stage6.Reachable config disk x)
    (a : Stage6.Action 5) (en : Stage6.enabled config x a = true) :
    Stage6.Reachable config disk (run x a) :=
  .advance h a (by simp [Stage6.step, en, run])

private def legacyInitial := Storage35.initWith config disk
private def legacySource := legacyRun legacyInitial (.prior (.load s₂))
private def legacyLoaded := legacyRun legacySource (.prior (.load d₂))
private def legacyAdmitted := legacyRun legacyLoaded singletonAction
private def legacyCrashed := legacyRun legacyAdmitted (.prior (.old .crash))
private def legacyLaunched := legacyRun legacyCrashed (.prior (.old .launch))

private theorem legacy_launched : Storage35.Reachable config disk legacyLaunched := by
  have source := legacyAdvance .initial (.prior (.load s₂)) (by decide)
  have loaded := legacyAdvance source (.prior (.load d₂)) (by decide)
  have admitted := legacyAdvance loaded singletonAction (by decide)
  have crashed := legacyAdvance admitted (.prior (.old .crash)) (by decide)
  exact legacyAdvance crashed (.prior (.old .launch)) (by decide)

private def foreign : Storage32.Action 5 := .old (.extWriteD d₂ 31 true)
private def startReceiver : Storage32.Action 5 := .old (.flush d₂)
private def guard : Storage32.Action 5 := .old .check
private def startDeletion : Storage32.Action 5 := .flushDel s₂
private def moveToTrash : Storage32.Action 5 := .old .rename
private def witnessSync : Storage32.Action 5 := .old (.dirSync true)
private def queueOpen : Storage32.Action 5 := .old (.wOpen d₂)
private def deliverOk : Storage32.Action 5 := .old (.deliverUp true)
private def receive : Storage32.Action 5 := .old (.wRecv d₂)
private def queueTakeDisk : Storage32.Action 5 := .old (.wDiscard d₂)
private def retireDraft : Storage32.Action 5 := .old (.draftSync d₂)

private def legacyDelete := legacyRun legacyLaunched (.prior startDeletion)
private def legacyChecked := legacyRun legacyDelete (.prior guard)
private def legacyRemoved := legacyRun legacyChecked (.prior moveToTrash)
private def legacySynced := legacyRun legacyRemoved (.prior witnessSync)
private def legacyForeign := legacyRun legacySynced (.prior foreign)
private def legacyReceiver := legacyRun legacyForeign (.prior startReceiver)
private def legacyConflict := legacyRun legacyReceiver (.prior guard)
private def legacyOpening := legacyRun legacyConflict (.prior queueOpen)
private def legacyOpened := legacyRun legacyOpening (.prior deliverOk)
private def legacyShowing := legacyRun legacyOpened (.prior receive)
private def legacyQueued := legacyRun legacyShowing (.prior queueTakeDisk)
private def legacyDiscarded := legacyRun legacyQueued (.prior deliverOk)
def singletonFinal := legacyRun legacyDiscarded (.prior retireDraft)

private def batchDelete := run relaunched (.prior startDeletion)
private def batchChecked := run batchDelete (.prior guard)
def batchRemoved := run batchChecked (.prior moveToTrash)
private def batchSynced := run batchRemoved (.prior witnessSync)
private def batchForeign := run batchSynced (.prior foreign)
private def batchReceiver := run batchForeign (.prior startReceiver)
def batchConflict := run batchReceiver (.prior guard)
private def batchOpening := run batchConflict (.prior queueOpen)
private def batchOpened := run batchOpening (.prior deliverOk)
private def batchShowing := run batchOpened (.prior receive)
private def batchQueued := run batchShowing (.prior queueTakeDisk)
private def batchDiscarded := run batchQueued (.prior deliverOk)
def batchFinal := run batchDiscarded (.prior retireDraft)

set_option maxRecDepth 4096 in
theorem singleton_relaunch_discard :
    Storage35.Reachable config disk legacyConflict ∧ (legacyConflict.s.pages d₂).conflict = true ∧
    Storage35.Reachable config disk singletonFinal ∧
    Storage35.guarantee singletonFinal = true ∧ Storage35.orderHolds singletonFinal = true ∧
    Storage35.renameResolves singletonFinal = true ∧ singletonFinal.g.renamed = [] ∧
    singletonFinal.s.disk s₂ = absent ∧ singletonFinal.s.disk d₂ = 31 ∧
    (∀ p, singletonFinal.s.disk p ≠ 20) ∧
    (singletonFinal.s.drafts d₂).bytes = noneBytes ∧
    20 ∈ singletonFinal.g.removed s₂ ∧ 20 ∈ singletonFinal.s.trashStable s₂ ∧
    Storage35.Reachable config disk legacyRemoved ∧
    legacyRemoved.s.disk s₂ = absent ∧ legacyRemoved.s.disk d₂ = absent ∧
    Storage35.renameResolves legacyRemoved = true := by
  have deletion := legacyAdvance legacy_launched (.prior startDeletion) (by decide)
  have checked := legacyAdvance deletion (.prior guard) (by decide)
  have removed := legacyAdvance checked (.prior moveToTrash) (by decide)
  have synced := legacyAdvance removed (.prior witnessSync) (by decide)
  have foreign := legacyAdvance synced (.prior foreign) (by decide)
  have receiver := legacyAdvance foreign (.prior startReceiver) (by decide)
  have conflict := legacyAdvance receiver (.prior guard) (by decide)
  have opening := legacyAdvance conflict (.prior queueOpen) (by decide)
  have opened := legacyAdvance opening (.prior deliverOk) (by decide)
  have showing := legacyAdvance opened (.prior receive) (by decide)
  have queued := legacyAdvance showing (.prior queueTakeDisk) (by decide)
  have discarded := legacyAdvance queued (.prior deliverOk) (by decide)
  have final := legacyAdvance discarded (.prior retireDraft) (by decide)
  have targets := Storage35.all_targets_arbitrary_initial_disk config disk singletonFinal rfl
    valid.1 valid.2.1 valid.2.2.1 valid.2.2.2 final
  exact ⟨conflict, rfl, final, targets.1, targets.2.1, targets.2.2, rfl, rfl, rfl,
    by decide, rfl, by decide, by decide, removed, rfl, rfl, rfl⟩

theorem singleton_relaunch_discard_embeds :
    Stage6.Reachable config disk (ofLegacy singletonFinal) ∧
    guarantee (ofLegacy singletonFinal) = true ∧ orderHolds (ofLegacy singletonFinal) = true ∧
    renameResolves (ofLegacy singletonFinal) = true :=
  ⟨reachable_ofLegacy config disk singletonFinal rfl valid singleton_relaunch_discard.2.2.1,
    legacy_targets config disk singletonFinal rfl valid singleton_relaunch_discard.2.2.1⟩

set_option maxRecDepth 4096 in
theorem batch_relaunch_discard :
    Stage6.Reachable config disk batchConflict ∧ (batchConflict.core.s.pages d₂).conflict = true ∧
    Stage6.Reachable config disk batchRemoved ∧
    batchRemoved.core.s.disk s₂ = absent ∧ batchRemoved.core.s.disk d₂ = absent ∧
    (batchRemoved.core.s.drafts d₂).bytes = 20 ∧ 20 ∈ batchRemoved.core.s.trash s₂ ∧
    renameResolves batchRemoved = true ∧
    Stage6.Reachable config disk batchFinal ∧
    guarantee batchFinal = true ∧ orderHolds batchFinal = true ∧ renameResolves batchFinal = true ∧
    batchFinal.renamed = [] ∧ batchFinal.core.s.disk s₂ = absent ∧ batchFinal.core.s.disk d₂ = 31 ∧
    (∀ p, batchFinal.core.s.disk p ≠ 20) ∧
    (batchFinal.core.s.drafts d₂).bytes = noneBytes ∧
    20 ∈ batchFinal.core.g.removed s₂ ∧ 20 ∈ batchFinal.core.s.trashStable s₂ := by
  have deletion := advance relaunch_between_receivers.1 (.prior startDeletion) (by decide)
  have checked := advance deletion (.prior guard) (by decide)
  have removed := advance checked (.prior moveToTrash) (by decide)
  have synced := advance removed (.prior witnessSync) (by decide)
  have foreign := advance synced (.prior foreign) (by decide)
  have receiver := advance foreign (.prior startReceiver) (by decide)
  have conflict := advance receiver (.prior guard) (by decide)
  have opening := advance conflict (.prior queueOpen) (by decide)
  have opened := advance opening (.prior deliverOk) (by decide)
  have showing := advance opened (.prior receive) (by decide)
  have queued := advance showing (.prior queueTakeDisk) (by decide)
  have discarded := advance queued (.prior deliverOk) (by decide)
  have final := advance discarded (.prior retireDraft) (by decide)
  have all := guarantee_reachable config disk batchFinal rfl final
  exact ⟨conflict, rfl, removed, rfl, rfl, rfl, by decide, rfl, final, all, rfl, rfl, rfl, rfl, rfl,
    by decide, rfl, by decide, by decide⟩

#print axioms singleton_relaunch_discard
#print axioms singleton_relaunch_discard_embeds
#print axioms batch_relaunch_discard

end Witnesses
end Stage6
