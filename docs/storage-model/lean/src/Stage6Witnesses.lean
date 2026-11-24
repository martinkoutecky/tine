import Stage6Admission

namespace Stage6.Witnesses
open Storage21

def config : Config 5 := { positive := by decide, texts := [10, 20, 30, 31], nonempty := by decide }
def s₁ : Slot 5 := 0
def d₁ : Slot 5 := 1
def s₂ : Slot 5 := 2
def d₂ : Slot 5 := 3
def referrer : Slot 5 := 4

def disk (p : Slot 5) : Int :=
  if p = s₁ then 10 else if p = s₂ then 20 else if p = referrer then 30 else absent

theorem valid : InitialValid config disk := by
  refine ⟨?_, by decide, by decide, by decide⟩
  intro p
  unfold disk
  by_cases h₁ : p = s₁
  · rw [if_pos h₁]
    simp [RealByte, config]
  rw [if_neg h₁]
  by_cases h₂ : p = s₂
  · rw [if_pos h₂]
    simp [RealByte, config]
  rw [if_neg h₂]
  split <;> simp [RealByte, config]

def load (p : Slot 5) : Action 5 := .prior (.load p)
def start (p : Slot 5) : Action 5 := .prior (.old (.flush p))
def deleteStart (p : Slot 5) : Action 5 := .prior (.flushDel p)
def run (x : State 5) (a : Action 5) : State 5 := execute config x a

private theorem advances {x : State 5} (reached : Reachable config disk x) (a : Action 5)
    (en : enabled config x a = true) : Reachable config disk (run x a) :=
  .advance reached a (by simp [step, en, run])

def initial := initWith config disk
def loaded₁ := run initial (load s₁)
def loaded₂ := run loaded₁ (load d₁)
def loaded₃ := run loaded₂ (load s₂)
def loaded₄ := run loaded₃ (load d₂)
def loaded := run loaded₄ (load referrer)

theorem loaded_reachable : Reachable config disk loaded := by
  have h₁ := advances (.initial valid) (load s₁) (by decide)
  have h₂ := advances h₁ (load d₁) (by decide)
  have h₃ := advances h₂ (load s₂) (by decide)
  have h₄ := advances h₃ (load d₂) (by decide)
  exact advances h₄ (load referrer) (by decide)

def namespaceBatch : Batch 5 :=
  ⟨fun p => some (if p = d₁ then 10 else if p = d₂ then 20 else if p = referrer then 31 else absent),
   fun p => some (disk p), [⟨d₁, s₁⟩, ⟨d₂, s₂⟩]⟩

def admitted := run loaded (.opApply namespaceBatch)

theorem two_move_admission :
    step config loaded (.opApply namespaceBatch) = some admitted ∧
    Reachable config disk admitted ∧
    (∃ gt, admitted.gates = [gt] ∧ gt.moves = namespaceBatch.moves ∧
      gt.pending = [s₁, d₁, s₂, d₂, referrer] ∧
      ∀ p, gt.vers p = some (version loaded p)) ∧
    admitted.renamed = [(s₁, d₁), (s₂, d₂)] ∧
    (admitted.core.s.drafts referrer).bytes = 31 ∧
    enabled config admitted (start d₁) = true ∧ enabled config admitted (start d₂) = true ∧
    enabled config admitted (start referrer) = false ∧
    enabled config admitted (deleteStart s₁) = false ∧ enabled config admitted (deleteStart s₂) = false := by
  have en : enabled config loaded (.opApply namespaceBatch) = true := by decide
  have versions := admission_gate config loaded namespaceBatch (by decide)
  obtain ⟨gt, installed, moves, pending, owned⟩ := versions
  have empty : loaded.gates = [] := rfl
  rw [empty, List.nil_append] at installed
  refine ⟨by simp [step, en, admitted, run], advances loaded_reachable _ en,
    ⟨gt, installed, moves, pending.trans rfl, fun p => owned p⟩, rfl, rfl,
    by decide, by decide, by decide, by decide, by decide⟩

def repeatedEndpoint : Batch 5 :=
  {namespaceBatch with moves := [⟨d₁, s₁⟩, ⟨d₂, s₁⟩]}

theorem repeated_endpoint_refused :
    Reachable config disk loaded ∧ ¬repeatedEndpoint.Shape ∧
    step config loaded (.opApply repeatedEndpoint) = none :=
  ⟨loaded_reachable, by decide, rfl⟩

def mergeBatch : Batch 5 :=
  ⟨fun p => if p = s₁ then some absent else if p = referrer then some 30 else none,
   fun p => if p = s₁ || p = referrer then some (disk p) else none,
   [⟨referrer, s₁⟩]⟩

def merged := run loaded (.opApply mergeBatch)
def check : Action 5 := .prior (.old .check)
def rename : Action 5 := .prior (.old .rename)
def sync : Action 5 := .prior (.old (.dirSync true))
def receiverJob := run merged (start referrer)
def receiverChecked := run receiverJob check
def receiverRenamed := run receiverChecked rename
def receiverPublished := run receiverRenamed sync
def uncertainSync : Action 5 := .prior (.old (.dirSync false))
def receiverUncertain := run receiverRenamed uncertainSync

theorem byte_identical_survivor :
    Reachable config disk merged ∧ Reachable config disk receiverRenamed ∧
    Reachable config disk receiverPublished ∧
    Reachable config disk receiverUncertain ∧
    mergeBatch.eff referrer = mergeBatch.plan referrer ∧
    (merged.core.s.drafts referrer).bytes = disk referrer ∧
    (merged.core.s.drafts referrer).ver = version loaded referrer ∧
    merged.core.s.disk referrer = receiverRenamed.core.s.disk referrer ∧
    receiverPublished.core.s.stable referrer = disk referrer ∧
    receiverRenamed.gates = merged.gates ∧ receiverUncertain.gates = merged.gates ∧
    receiverPublished.gates.map Gate.pending = [[s₁]] ∧
    enabled config merged (deleteStart s₁) = false ∧
    enabled config receiverUncertain (deleteStart s₁) = false ∧
    enabled config receiverPublished (deleteStart s₁) = true := by
  have admitted := advances loaded_reachable (.opApply mergeBatch) (by decide)
  have started := advances admitted (start referrer) (by decide)
  have checked := advances started check (by decide)
  have renamed := advances checked rename (by decide)
  have published := advances renamed sync (by decide)
  have uncertain := advances renamed uncertainSync (by decide)
  exact ⟨admitted, renamed, published, uncertain, rfl, rfl, rfl, rfl, rfl, rfl, rfl, rfl,
    by decide, by decide, by decide⟩

def firstJob := run admitted (start d₁)
def firstChecked := run firstJob check
def firstRenamed := run firstChecked rename
def firstPublished := run firstRenamed sync

theorem first_published_reachable : Reachable config disk firstPublished :=
  advances (advances (advances (advances two_move_admission.2.1 (start d₁) (by decide))
    check (by decide)) rename (by decide)) sync (by decide)

def queueDiscard : Action 5 := .prior (.old (.wDiscard d₂))
def deliver (ok : Bool) : Action 5 := .prior (.old (.deliverUp ok))
def openSecond : Action 5 := .prior (.old (.wOpen d₂))
def receiveSecond : Action 5 := .prior (.old (.wRecv d₂))
def openingSecond := run firstPublished openSecond
def openedSecond := run openingSecond (deliver true)
def showingSecond := run openedSecond receiveSecond
def discardQueued := run showingSecond queueDiscard
def discardFailed := run discardQueued (deliver false)
def secondCancelled := run discardQueued (deliver true)

theorem partial_discard :
    Reachable config disk discardFailed ∧ Reachable config disk secondCancelled ∧
    discardFailed.gates = firstPublished.gates ∧ discardFailed.obl = firstPublished.obl ∧
    discardFailed.core.s.pages = discardQueued.core.s.pages ∧
    secondCancelled.gates.map Gate.moves = [[⟨d₁, s₁⟩]] ∧
    secondCancelled.gates.map Gate.pending = [[s₁, referrer]] ∧
    touches secondCancelled.gates [d₁] = true ∧ touches secondCancelled.gates [s₂] = true ∧
    (secondCancelled.core.s.pages s₁).buf = absent ∧ (secondCancelled.core.s.pages s₂).buf = disk s₂ ∧
    secondCancelled.obl = [(s₁, referrer, version loaded referrer, d₁)] ∧
    enabled config firstPublished (start referrer) = false ∧
    enabled config secondCancelled (start referrer) = true ∧
    enabled config secondCancelled (deleteStart s₁) = false := by
  have opening := advances first_published_reachable openSecond (by decide)
  have opened := advances opening (deliver true) (by decide)
  have showing := advances opened receiveSecond (by decide)
  have queued := advances showing queueDiscard (by decide)
  exact ⟨advances queued (deliver false) (by decide),
    advances queued (deliver true) (by decide), rfl, rfl, rfl, rfl, rfl,
    rfl, rfl, rfl, rfl, rfl, by decide, by decide, by decide⟩

def crash : Action 5 := .prior (.old .crash)
def launch : Action 5 := .prior (.old .launch)
def crashed := run firstPublished crash
def relaunched := run crashed launch

theorem relaunch_between_receivers :
    Reachable config disk relaunched ∧ relaunched.gates = [] ∧ relaunched.obl = [] ∧
    relaunched.core.s.drafts = firstPublished.core.s.drafts ∧
    relaunched.core.s.disk d₂ = absent ∧ (relaunched.core.s.pages d₂).buf = 20 ∧
    enabled config relaunched (deleteStart s₂) = true := by
  have down := advances first_published_reachable crash (by decide)
  exact ⟨advances down launch (by decide), rfl, rfl, rfl, rfl, rfl, by decide⟩

def outsideRemoval : Action 5 := .prior (.old (.extWriteD referrer absent true))
def replacedAfterRename := run receiverRenamed outsideRemoval
def oldPublication := run replacedAfterRename sync

/-- The inherited publication signal abstracts the production stamp check. -/
theorem legacy_publication_signal :
    Reachable config disk oldPublication ∧ oldPublication.core.s.disk referrer = absent ∧
    enabled config oldPublication (deleteStart s₁) = true := by
  have outside := advances byte_identical_survivor.2.1 outsideRemoval (by decide)
  exact ⟨advances outside sync (by decide), rfl, by decide⟩

def sourceJob := run receiverPublished (deleteStart s₁)
def sourceChecked := run sourceJob check
def sourceRemoved := run sourceChecked rename
def power (keep : Bool) : Action 5 := .prior (.old (.power fun _ => keep))
def keptRemoval := run sourceRemoved (power true)
def revertedToKept := run keptRemoval (power false)

/-- The surviving trash payload and the legacy removal ghost can diverge
across consecutive cuts, even with a strong directory profile. -/
theorem legacy_removal_ghost_after_two_cuts :
    Reachable config disk revertedToKept ∧ keptRemoval.core.g.removed s₁ = [10] ∧
    revertedToKept.core.s.disk s₁ = absent ∧
    revertedToKept.core.s.trash s₁ = [10] ∧ revertedToKept.core.g.removed s₁ = [] := by
  have started := advances byte_identical_survivor.2.2.1 (deleteStart s₁) (by decide)
  have checked := advances started check (by decide)
  have moved := advances checked rename (by decide)
  have kept := advances moved (power true) (by decide)
  exact ⟨advances kept (power false) (by decide), rfl, rfl, rfl, rfl⟩

#print axioms two_move_admission
#print axioms repeated_endpoint_refused
#print axioms byte_identical_survivor
#print axioms partial_discard
#print axioms relaunch_between_receivers
#print axioms legacy_publication_signal
#print axioms legacy_removal_ghost_after_two_cuts

end Stage6.Witnesses
