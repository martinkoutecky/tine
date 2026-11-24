import Stage6Admission
import Stage6GateSafety

namespace PairedRecovery
open Storage21 Stage6

inductive Choice where
  | disk
  | review (base text : Int)
  deriving DecidableEq

inductive Disposition where
  | applied | cancelled | conflict
  deriving DecidableEq

inductive Phase where
  | pending
  | retainedConflict
  | retired (result : Disposition)
  deriving DecidableEq

structure Observation where
  exact : Bool
  activeCopy : Bool := false
  deriving DecidableEq

/-- One durable operation ID, fixed by admission. Trash is the custody
evidence for this ID. The retired disposition is a trace observation. -/
structure State (P : Nat) where
  live : Slot P → Int
  stable : Slot P → Int
  preimage : WriteMap P
  trash : Slot P → List Int := fun _ => []
  stableTrash : Slot P → List Int := fun _ => []
  choice : Slot P → Option Choice := fun _ => none
  disarmed : Slot P → Bool := fun _ => false
  tookDisk : Slot P → Bool := fun _ => false
  deleted : Slot P → Bool := fun _ => false
  seen : Slot P → Option Observation := fun _ => none
  permit : Option (Move P) := none
  up : Bool := true
  phase : Phase := .pending

def State.result (x : State P) : Option Disposition :=
  match x.phase with
  | .pending => none
  | .retainedConflict => some .conflict
  | .retired d => some d

def State.hasDraft (x : State P) : Bool :=
  match x.phase with
  | .retired _ => false
  | _ => true

abbrev before (b : Batch P) := Storage33.get b.plan
abbrev after (b : Batch P) := Storage33.get b.eff

def init (b : Batch P) (live stable : Slot P → Int) : State P :=
  {live, stable, preimage := b.plan}

def source (b : Batch P) (p : Slot P) : Prop := ∃ m ∈ b.moves, m.src = p
def receiver (b : Batch P) (p : Slot P) : Prop := ∃ m ∈ b.moves, m.dst = p

def durableAt (x : State P) (p : Slot P) (bytes : Int) : Prop :=
  x.live p = bytes ∧ x.stable p = bytes

def certified (b : Batch P) (x : State P) (m : Move P) : Prop :=
  durableAt x m.dst (after b m.dst)

def disarmed (x : State P) (m : Move P) : Prop :=
  x.disarmed m.dst = true ∨ x.disarmed m.src = true

def fileTurn (b : Batch P) (x : State P) (p : Slot P) : Prop :=
  p ∈ b.keys ∧ ¬source b p ∧
    (receiver b p ∨ ∀ m ∈ b.moves, (x.seen m.dst).isSome)

def sourceTurn (b : Batch P) (x : State P) : Prop :=
  ∀ p ∈ b.keys, ¬source b p → (x.seen p).isSome

/-- The normal save guard compares the chosen base. An exact output can
instead be re-witnessed in place. Use Disk issues no write. -/
def writeGoal (b : Batch P) (x : State P) (p : Slot P) (bytes : Int) : Prop :=
  match x.choice p with
  | none => bytes = after b p ∧ (x.live p = before b p ∨ x.live p = bytes)
  | some .disk => False
  | some (.review base text) => bytes = text ∧ x.live p = base

def observedGoal (b : Batch P) (x : State P) (p : Slot P) : Prop :=
  match x.choice p with
  | none => x.live p = after b p
  | some .disk => True
  | some (.review _ text) => x.live p = text

def State.ready (x : State P) : Prop := x.up = true ∧ x.phase = .pending

def reset (x : State P) (up : Bool) : State P :=
  {x with up, permit := none, seen := fun _ => none}

def powerCut (x : State P) (keepFile keepTrash : Slot P → Bool) : State P :=
  let live := fun p => if keepFile p then x.live p else x.stable p
  let trash := fun p => if keepTrash p then x.trash p else x.stableTrash p
  {reset x false with live, stable := live, trash, stableTrash := trash}

def invalidate (b : Batch P) (x : State P) (p q : Slot P) : Option Observation :=
  if q == p || b.moves.any (fun m => m.dst == p && m.src == q) then none else x.seen q

def choose (b : Batch P) (x : State P) (p : Slot P) (choice : Choice) : State P :=
  {x with
    choice := put x.choice p (some choice)
    disarmed := put x.disarmed p
      (x.disarmed p || match choice with | .disk => true | .review _ text => text != after b p)
    tookDisk := put x.tookDisk p (choice == .disk)
    seen := invalidate b x p
    permit := none}

def trashFile (x : State P) (p : Slot P) : State P :=
  {x with
    live := put x.live p absent
    trash := put x.trash p (x.live p :: x.trash p)
    seen := put x.seen p none, permit := none}

def mayRestore (b : Batch P) (x : State P) (m : Move P) : Prop :=
  disarmed x m ∧ x.deleted m.src = false ∧ x.trash m.src ≠ [] ∧
    x.live m.src = absent ∧ ¬certified b x m ∧
    (∀ base text, x.choice m.src = some (.review base text) → text = absent)

def Standing (x : State P) (m : Move P) : Prop :=
  ∃ o, x.seen m.src = some o ∧ o.activeCopy = true

def Anchored (b : Batch P) (x : State P) (m : Move P) : Prop :=
  durableAt x m.src (before b m.src) ∨ certified b x m ∨ x.deleted m.src = true

def AllSeen (b : Batch P) (x : State P) : Prop := ∀ p ∈ b.keys, (x.seen p).isSome

def Applied (b : Batch P) (x : State P) : Prop :=
  (∀ p ∈ b.keys, ∃ o, x.seen p = some o ∧ o.exact = true) ∧
    (∀ m ∈ b.moves, ¬disarmed x m ∧ Standing x m)

def Cancelled (b : Batch P) (x : State P) : Prop :=
  (∀ m ∈ b.moves, x.tookDisk m.dst = true ∧ x.stableTrash m.src = [] ∧ x.deleted m.src = false ∧
    Standing x m) ∧
    ∀ p ∈ b.keys, ¬source b p → ¬receiver b p → ∃ o, x.seen p = some o ∧ o.exact = true

inductive Decides (b : Batch P) (x : State P) : Disposition → Prop where
  | applied : Applied b x → Decides b x .applied
  | cancelled : ¬Applied b x → Cancelled b x → Decides b x .cancelled
  | conflict : ¬Applied b x → ¬Cancelled b x → Decides b x .conflict

/-- The preflight records a receiver witness and a source base guard in memory.
The trash step changes one source entry. The post-check reads the receiver
again after the source and trash directory barriers. -/
inductive Step (b : Batch P) (racySource : Bool) : State P → State P → Prop where
  | pause {x} : Step b racySource x (reset x false)
  | launch {x} : x.up = false → Step b racySource x (reset x true)
  | cut {x} (keepFile keepTrash : Slot P → Bool) :
      Step b racySource x (powerCut x keepFile keepTrash)
  | outside {x} (p : Slot P) (bytes : Int) (synced : Bool) :
      Step b racySource x {x with
        live := put x.live p bytes
        stable := if synced then put x.stable p bytes else x.stable}
  | decision {x p c} : x.ready → p ∈ b.keys →
      (∀ base text, c = .review base text → x.live p = base ∧ (source b p → text ≠ absent)) →
      Step b racySource x (choose b x p c)
  | write {x p bytes} : x.ready → fileTurn b x p → writeGoal b x p bytes →
      Step b racySource x {x with
        live := put x.live p bytes
        seen := invalidate b x p
        permit := none}
  | syncFile {x p} : x.ready → fileTurn b x p → observedGoal b x p →
      Step b racySource x {x with stable := put x.stable p (x.live p)}
  | checkFile {x p} : x.ready → fileTurn b x p → observedGoal b x p → x.live p = x.stable p →
      Step b racySource x {x with seen := put x.seen p (some ⟨x.live p == after b p, false⟩)}
  | check {x m} : x.ready → m ∈ b.moves → sourceTurn b x → ¬disarmed x m →
      certified b x m → x.live m.src = before b m.src → x.live m.src ≠ absent →
      Step b racySource x {x with permit := some m}
  | trash {x m} : x.ready → m ∈ b.moves → ¬disarmed x m → x.permit = some m →
      (racySource = true ∨ x.live m.src = before b m.src) → x.live m.src ≠ absent →
      Step b racySource x (trashFile x m.src)
  | syncTrash {x m} : x.ready → m ∈ b.moves →
      Step b racySource x {x with stableTrash := put x.stableTrash m.src (x.trash m.src)}
  | syncSource {x m} : x.ready → m ∈ b.moves → x.live m.src = absent →
      (¬disarmed x m ∨ x.deleted m.src = true ∨ ¬mayRestore b x m) →
      Step b racySource x {x with stable := put x.stable m.src absent}
  | checkMoved {x m} : x.ready → m ∈ b.moves → sourceTurn b x → ¬disarmed x m →
      certified b x m → durableAt x m.src absent → x.trash m.src = x.stableTrash m.src →
      Step b racySource x {x with seen := put x.seen m.src (some ⟨true, true⟩), permit := none}
  | restore {x m bytes} : x.up = true → x.hasDraft = true → m ∈ b.moves → mayRestore b x m →
      x.preimage m.src = some bytes →
      Step b racySource x {x with
        live := put x.live m.src bytes
        seen := put x.seen m.src none, permit := none}
  | writeSourceChoice {x m base text} : x.ready → m ∈ b.moves → disarmed x m →
      x.choice m.src = some (.review base text) → x.live m.src = base → text ≠ absent →
      Step b racySource x {x with
        live := put x.live m.src text
        seen := put x.seen m.src none, permit := none}
  | syncKeptSource {x m} : x.up = true → x.hasDraft = true → m ∈ b.moves → disarmed x m →
      Step b racySource x {x with stable := put x.stable m.src (x.live m.src)}
  | checkKeptSource {x m} : x.ready → m ∈ b.moves → disarmed x m →
      ¬mayRestore b x m → x.live m.src = x.stable m.src →
      (∀ base text, x.choice m.src = some (.review base text) → x.live m.src = text) →
      x.trash m.src = x.stableTrash m.src →
      Step b racySource x {x with
        seen := put x.seen m.src (some ⟨false, decide (x.live m.src = before b m.src ∨
          (x.live m.dst = after b m.dst ∧ x.stable m.dst = after b m.dst) ∨
          x.deleted m.src = true)⟩)
        permit := none}
  | explicitDelete {x m} : x.ready → m ∈ b.moves → x.live m.src ≠ absent →
      Step b racySource x {trashFile x m.src with
        deleted := put x.deleted m.src true, disarmed := put x.disarmed m.src true,
        choice := put x.choice m.src (some .disk)}
  | retire {x d} : x.ready → AllSeen b x → Decides b x d →
      (∀ m ∈ b.moves, Standing x m) →
      Step b racySource x {x with phase := .retired d, permit := none, preimage := fun _ => none}
  | retainConflict {x} : x.ready → AllSeen b x → Decides b x .conflict →
      (∃ m ∈ b.moves, ¬Standing x m) →
      Step b racySource x {x with phase := .retainedConflict, permit := none}
  | checkRetained {x m} : x.up = true → x.phase = .retainedConflict →
      m ∈ b.moves → Anchored b x m →
      Step b racySource x {x with seen := put x.seen m.src (some ⟨false, true⟩)}
  | releaseRetained {x} : x.up = true → x.phase = .retainedConflict →
      (∀ m ∈ b.moves, Standing x m) →
      Step b racySource x {x with phase := .retired .conflict, preimage := fun _ => none}

inductive Reaches (b : Batch P) (race : Bool) (first : State P) : State P → Prop where
  | here : Reaches b race first first
  | next {x y} : Reaches b race first x → Step b race x y → Reaches b race first y

variable {b : Batch P}

theorem Reaches.trans (first : Reaches b race x y) (rest : Reaches b race y z) :
    Reaches b race x z := by
  induction rest with
  | here => exact first
  | next _ step ih => exact .next ih step

theorem disarmed_persists (step : Step b race x y) (off : x.disarmed p = true) :
    y.disarmed p = true := by
  cases step
  case decision q _ _ _ _ =>
    by_cases eq : p = q
    · subst q; simp [choose, put, off]
    · simp [choose, put, off, eq]
  case explicitDelete => simp [put, off]
  all_goals exact off

theorem cancellation_survives (trace : Reaches b race x y) (off : disarmed x m) :
    disarmed y m := by
  induction trace with
  | here => exact off
  | next _ step ih => exact ih.imp (disarmed_persists step) (disarmed_persists step)

theorem receiver_take_disk (x : State P) (b : Batch P) (m : Move P) :
    disarmed (choose b x m.dst .disk) m := by
  exact Or.inl (by simp [choose, put])

theorem receiver_review (x : State P) (b : Batch P) (m : Move P) (base text : Int) :
    (choose b x m.dst (.review base text)).disarmed m.dst = true ↔
      x.disarmed m.dst = true ∨ text ≠ after b m.dst := by
  simp [choose, put, Bool.or_eq_true]

theorem cancelled_cannot_trash (off : disarmed x m) :
    ¬ (x.ready ∧ m ∈ b.moves ∧ ¬disarmed x m ∧ x.permit = some m) :=
  fun enabled => enabled.2.2.1 off

def orderHolds (b : Batch P) (race : Bool) (x : State P) : Prop :=
  ∀ m, x.permit = some m →
    ∃ checked, Reaches b race checked x ∧ sourceTurn b checked ∧
      certified b checked m ∧ checked.live m.src = before b m.src ∧ ¬disarmed checked m

theorem orderHolds_reachable (trace : Reaches b race (init b live stable) x) :
    orderHolds b race x := by
  induction trace with
  | here => intro m permit; cases permit
  | next _ step ih =>
    intro m permit
    cases step
    case check _ member turn armed exactBytes guard _ =>
      cases Option.some.inj permit
      exact ⟨_, .next .here (.check (by assumption) member turn armed exactBytes guard (by assumption)),
        turn, exactBytes, guard, armed⟩
    all_goals
      first
      | cases permit
      | obtain ⟨checked, prior, turn, exactBytes, guard, armed⟩ := ih m permit
        refine ⟨checked, .next prior ?_, turn, exactBytes, guard, armed⟩
        first
        | exact .syncKeptSource (by assumption) (by assumption) (by assumption) (by assumption)
        | constructor <;> assumption

theorem source_not_receiver (shape : b.Shape) (src : source b p) : ¬receiver b p := by
  rintro ⟨m, mm, dst⟩
  obtain ⟨n, nm, origin⟩ := src
  have same := endpoint_unique shape.2.1 mm nm (Or.inl dst.symm) (Or.inr origin.symm)
  subst n
  exact endpoint_distinct shape.2.1 mm (origin.trans dst.symm)

theorem source_unique (shape : b.Shape) (mm : m ∈ b.moves) (nm : n ∈ b.moves)
    (eq : m.src = n.src) : m = n :=
  endpoint_unique shape.2.1 mm nm (Or.inr rfl) (Or.inr eq)

theorem unrelated_decision_keeps_pair (shape : b.Shape) (mm : m ∈ b.moves) (nm : n ∈ b.moves)
    (different : m ≠ n) (endpoint : p = n.src ∨ p = n.dst) :
    disarmed (choose b x p c) m ↔ disarmed x m := by
  have nope (hit : p = m.src ∨ p = m.dst) : False :=
    different (endpoint_unique shape.2.1 mm nm (hit.symm) (endpoint.symm))
  have src : m.src ≠ p := fun h => nope (Or.inl h.symm)
  have dst : m.dst ≠ p := fun h => nope (Or.inr h.symm)
  simp [disarmed, choose, put, src, dst]

private theorem standing_step (shape : b.Shape) (member : m ∈ b.moves)
    (step : Step b race x y) (h : Standing y m) : Standing x m ∨ Anchored b x m := by
  obtain ⟨o, seen, active⟩ := h
  cases step
  all_goals try exact Or.inl ⟨o, seen, active⟩
  all_goals try solve
    | simp [reset, powerCut] at seen
    | left
      refine ⟨o, ?_, active⟩
      change invalidate b _ _ m.src = some o at seen
      unfold invalidate at seen
      split at seen <;> simp_all
    | left
      refine ⟨o, ?_, active⟩
      change put _ _ none m.src = some o at seen
      unfold put at seen
      split at seen <;> simp_all
  case checkFile p _ _ _ _ =>
    by_cases eq : m.src = p
    · simp [put, eq] at seen
      simp [← seen] at active
    · exact Or.inl ⟨o, by simpa [put, eq] using seen, active⟩
  case checkMoved n _ nm _ _ exactBytes _ _ =>
    by_cases eq : m.src = n.src
    · cases source_unique shape member nm eq
      exact Or.inr (Or.inr (Or.inl exactBytes))
    · exact Or.inl ⟨o, by simpa [put, eq] using seen, active⟩
  case checkKeptSource n _ nm _ _ sync _ _ =>
    by_cases eq : m.src = n.src
    · cases source_unique shape member nm eq
      simp [put] at seen
      subst o
      simp [Bool.or_eq_true, Bool.and_eq_true] at active
      exact Or.inr (active.imp (fun same => ⟨same, sync.symm.trans same⟩) id)
    · exact Or.inl ⟨o, by simpa [put, eq] using seen, active⟩
  case checkRetained n _ _ nm witness =>
    by_cases eq : m.src = n.src
    · cases source_unique shape member nm eq
      exact Or.inr witness
    · exact Or.inl ⟨o, by simpa [put, eq] using seen, active⟩

theorem standing_witness (shape : b.Shape) (member : m ∈ b.moves)
    (trace : Reaches b race (init b live stable) x) (h : Standing x m) :
    ∃ checked, Reaches b race (init b live stable) checked ∧
      Reaches b race checked x ∧ Anchored b checked m := by
  induction trace with
  | here => obtain ⟨o, seen, _⟩ := h; cases seen
  | next priorTrace step ih =>
    rcases standing_step shape member step h with old | witnessed
    · obtain ⟨checked, prior, after, witnessed⟩ := ih old
      exact ⟨checked, prior, .next after step, witnessed⟩
    · exact ⟨_, priorTrace, .next .here step, witnessed⟩

theorem retirement_witness (shape : b.Shape) (trace : Reaches b race (init b live stable) x)
    (step : Step b race x y) (kept : x.hasDraft = true) (unlinked : y.hasDraft = false)
    (member : m ∈ b.moves) :
    ∃ checked, Reaches b race (init b live stable) checked ∧
      Reaches b race checked x ∧ Anchored b checked m := by
  cases step
  case retire _ _ _ standing => exact standing_witness shape member trace (standing m member)
  case releaseRetained _ _ standing => exact standing_witness shape member trace (standing m member)
  case retainConflict => cases unlinked
  all_goals exact False.elim (Bool.noConfusion (unlinked.symm.trans kept))

def Covered (old new : Int) (live : Slot P → Int) (saved : Option Int) (m : Move P) : Prop :=
  live m.src = old ∨ live m.dst = new ∨ saved = some old

def renameResolves (b : Batch P) (x : State P) : Prop :=
  ∀ m ∈ b.moves, x.hasDraft = true → x.deleted m.src = false →
    Covered (before b m.src) (after b m.dst) x.live (x.preimage m.src) m

theorem preimage_retained (trace : Reaches b race (init b live stable) x) :
    x.hasDraft = true → x.preimage = b.plan := by
  induction trace with
  | here => intro _; rfl
  | next _ step ih =>
    cases step
    case retire => simp [State.hasDraft]
    case releaseRetained => simp [State.hasDraft]
    case retainConflict ready _ _ _ => exact fun _ => ih (by simp [State.hasDraft, ready.2])
    all_goals exact ih

theorem renameResolves_reachable (shape : b.Shape)
    (trace : Reaches b race (init b live stable) x) : renameResolves b x := by
  intro m hm on _
  have present := (shape.2.2 m hm).2.2
  right; right
  cases h : b.plan m.src with
  | none => simp [Storage33.get, h] at present
  | some old => simp [preimage_retained trace on, Storage33.get, h]

def TrashCustody (x : State P) : Prop :=
  ∀ p bytes, bytes ∈ x.stableTrash p → bytes ∈ x.trash p

private theorem trash_file_custody (held : TrashCustody x) (p : Slot P) :
    TrashCustody (trashFile x p) := by
  intro q bytes mem
  have old := held q bytes mem
  by_cases hit : q = p
  · subst q; simp [trashFile, put, old]
  · simpa [trashFile, put, hit] using old

private theorem trash_custody_step (step : Step b race x y) (held : TrashCustody x) :
    TrashCustody y ∧ ∀ p bytes, bytes ∈ x.stableTrash p → bytes ∈ y.stableTrash p := by
  cases step <;> try exact ⟨held, fun _ _ h => h⟩
  case cut _ keepTrash =>
    refine ⟨fun _ _ h => h, fun p bytes h => ?_⟩
    cases pick : keepTrash p <;> simp [powerCut, pick, h, held p bytes h]
  case syncTrash m _ _ =>
    constructor
    · intro p bytes mem
      by_cases hit : p = m.src
      · simpa [put, hit] using mem
      · exact held p bytes (by simpa [put, hit] using mem)
    · intro p bytes mem
      by_cases hit : p = m.src
      · subst p; simpa [put] using held m.src bytes mem
      · simpa [put, hit] using mem
  case trash m _ _ _ _ _ _ => exact ⟨trash_file_custody held m.src, fun _ _ h => h⟩
  case explicitDelete m _ _ _ => exact ⟨trash_file_custody held m.src, fun _ _ h => h⟩

theorem trash_custody_reachable (trace : Reaches b race (init b live stable) x) :
    TrashCustody x := by
  induction trace with
  | here => exact fun _ _ h => h
  | next _ step ih => exact (trash_custody_step step ih).1

theorem stable_trash_survives (trace : Reaches b race x y) (held : TrashCustody x)
    (mem : bytes ∈ x.stableTrash p) : bytes ∈ y.stableTrash p := by
  have keeps : TrashCustody y ∧ ∀ p bytes, bytes ∈ x.stableTrash p → bytes ∈ y.stableTrash p := by
    induction trace with
    | here => exact ⟨held, fun _ _ h => h⟩
    | next _ step ih =>
      obtain ⟨kept, stable⟩ := trash_custody_step step ih.1
      exact ⟨kept, fun p bytes h => stable p bytes (ih.2 p bytes h)⟩
  exact keeps.2 p bytes mem

theorem result_persists (step : Step b race x y) (h : x.result = some d) :
    y.result = some d := by
  cases step
  case retire ready _ _ _ => simp [State.result, ready.2] at h
  case retainConflict ready _ _ _ => simp [State.result, ready.2] at h
  case releaseRetained _ held _ => simpa [State.result, held] using h
  all_goals exact h

theorem disposition_unique (trace : Reaches b race x y) (h : x.result = some d) :
    y.result = some d := by
  induction trace with
  | here => exact h
  | next _ step ih => exact result_persists step ih

theorem terminal_when_observed (ready : x.ready) (seen : AllSeen b x) :
    ∃ y d, Step b race x y ∧ y.result = some d := by
  classical
  by_cases applied : Applied b x
  · exact ⟨_, .applied, .retire ready seen (.applied applied) (fun m hm => (applied.2 m hm).2), rfl⟩
  by_cases cancelled : Cancelled b x
  · exact ⟨_, .cancelled,
      .retire ready seen (.cancelled applied cancelled) (fun m hm => (cancelled.1 m hm).2.2.2), rfl⟩
  by_cases copies : ∀ m ∈ b.moves, Standing x m
  · exact ⟨_, .conflict, .retire ready seen (.conflict applied cancelled) copies, rfl⟩
  · have missing : ∃ m ∈ b.moves, ¬Standing x m := by
      apply Classical.byContradiction
      intro none
      apply copies
      intro m hm
      exact Classical.byContradiction (fun no => none ⟨m, hm, no⟩)
    exact ⟨_, .conflict, .retainConflict ready seen (.conflict applied cancelled) missing, rfl⟩

/-- An outside file replacement after the final observation can beat the
private unlink. A disposition certifies observations in their checked order. -/
theorem outside_may_win_after_observation (ready : x.ready) (seen : AllSeen b x)
    (applied : Applied b x) (p : Slot P) (bytes : Int) :
    ∃ u y, Step b race x u ∧ Step b race u y ∧ y.result = some .applied ∧
      durableAt y p bytes := by
  let u := {x with live := put x.live p bytes, stable := put x.stable p bytes}
  exact ⟨u, _, .outside p bytes true,
    .retire ready seen (.applied applied) (fun m hm => (applied.2 m hm).2), rfl,
    by simp [durableAt, u, put]⟩

#print axioms renameResolves_reachable
#print axioms orderHolds_reachable
#print axioms retirement_witness
#print axioms cancellation_survives
#print axioms unrelated_decision_keeps_pair
#print axioms trash_custody_reachable
#print axioms stable_trash_survives
#print axioms disposition_unique
#print axioms terminal_when_observed
#print axioms outside_may_win_after_observation

end PairedRecovery
