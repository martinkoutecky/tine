import PairedRecovery

namespace PairedRecovery
open Stage6 Storage21

variable {P : Nat} {b : Batch P} {race : Bool}

structure Clean (b : Batch P) (x : State P) : Prop where
  ready : x.ready
  untouched : ∀ p ∈ b.keys, x.choice p = none ∧ x.disarmed p = false
  compatible : ∀ p ∈ b.keys, x.live p = before b p ∨ x.live p = after b p

def Verified (b : Batch P) (x : State P) (p : Slot P) : Prop :=
  durableAt x p (after b p) ∧
    ∃ o, x.seen p = some o ∧ o.exact = true ∧ (source b p → o.activeCopy = true)

def Frame (b : Batch P) (x y : State P) : Prop :=
  ∀ p, p ∉ b.keys → y.live p = x.live p ∧ y.stable p = x.stable p

theorem Frame.trans (xy : Frame b x y) (yz : Frame b y z) : Frame b x z :=
  fun p outside => ⟨(yz p outside).1.trans (xy p outside).1,
    (yz p outside).2.trans (xy p outside).2⟩

private theorem invalidate_other_file (different : q ≠ p) (file : ¬source b q) :
    invalidate b x p q = x.seen q := by
  have noPartner : (b.moves.any fun m => m.dst == p && m.src == q) = false := by
    apply Bool.eq_false_iff.mpr
    intro hit
    obtain ⟨m, mm, both⟩ := List.any_eq_true.mp hit
    simp only [Bool.and_eq_true] at both
    exact file ⟨m, mm, beq_iff_eq.mp both.2⟩
  simp [invalidate, different, noPartner]

private theorem file_turn {x : State P} (key : p ∈ b.keys) (file : ¬source b p)
    (turn : receiver b p ∨ ∀ q, receiver b q → Verified b x q) : fileTurn b x p :=
  ⟨key, file, turn.imp_right fun all m hm => by
    obtain ⟨_, o, seen, _, _⟩ := all m.dst ⟨m, hm, rfl⟩
    simp [seen]⟩

private theorem finish_file (shape : b.Shape) (good : Clean b x) (turn : fileTurn b x p) :
    ∃ y, Reaches b race x y ∧ Frame b x y ∧ Clean b y ∧ Verified b y p ∧
      ∀ q, q ≠ p → ¬source b q → Verified b x q → Verified b y q := by
  let w := {x with live := put x.live p (after b p), seen := invalidate b x p, permit := none}
  let s := {w with stable := put w.stable p (w.live p)}
  let y := {s with seen := put s.seen p (some ⟨s.live p == after b p, false⟩)}
  have chosen := (good.untouched p turn.1).1
  have write : Step b race x w :=
    .write good.ready turn (by simp [writeGoal, chosen, good.compatible p turn.1])
  have turn_w : fileTurn b w p := by
    refine ⟨turn.1, turn.2.1, ?_⟩
    by_cases recv : receiver b p
    · exact Or.inl recv
    · right
      intro m hm
      have dst : receiver b m.dst := ⟨m, hm, rfl⟩
      have ne : m.dst ≠ p := fun eq => recv (eq ▸ dst)
      change (invalidate b x p m.dst).isSome
      rw [invalidate_other_file ne (fun src => source_not_receiver shape src dst)]
      exact turn.2.2.resolve_left recv m hm
  have sync : Step b race w s := .syncFile good.ready turn_w (by simp [observedGoal, w, chosen, put])
  have check : Step b race s y :=
    .checkFile good.ready turn_w (by simp [observedGoal, s, w, chosen, put]) (by simp [s, put])
  have done : Verified b y p := by
    refine ⟨?_, ⟨true, false⟩, ?_, rfl, fun src => False.elim (turn.2.1 src)⟩
    · simp [durableAt, y, s, w, put]
    · simp [y, s, w, put]
  refine ⟨y, .next (.next (.next .here write) sync) check, ?_,
    ⟨good.ready, good.untouched, ?_⟩, done, ?_⟩
  · intro q outside
    have ne : q ≠ p := fun eq => outside (eq ▸ turn.1)
    simp [y, s, w, put, ne]
  · intro q key
    by_cases eq : q = p
    · subst q; exact Or.inr done.1.1
    · simpa [y, s, w, put, eq] using good.compatible q key
  · intro q ne file old
    simpa [Verified, durableAt, y, s, w, put, ne, invalidate_other_file ne file] using old

private theorem finish_source (shape : b.Shape) (good : Clean b x) (turn : sourceTurn b x)
    (m : Move P) (member : m ∈ b.moves) (receiver : certified b x m) :
    ∃ y, Reaches b race x y ∧ Frame b x y ∧ Clean b y ∧ Verified b y m.src ∧
      ∀ q, q ≠ m.src → Verified b x q → Verified b y q := by
  have ⟨src, dst⟩ := move_endpoints_in_keys b shape m member
  have absentAfter : after b m.src = absent := by
    simp [Storage33.get, (shape.2.2 m member).1]
  have armed : ¬disarmed x m := by
    simp [disarmed, (good.untouched m.dst dst).2, (good.untouched m.src src).2]
  let w := if x.live m.src = absent then x else trashFile x m.src
  have write : Reaches b race x w := by
    by_cases empty : x.live m.src = absent
    · simpa [w, empty] using (Reaches.here : Reaches b race x x)
    · have same := (good.compatible m.src src).resolve_right (fun h => empty (h.trans absentAfter))
      have pre := Step.check (racySource := race) good.ready member turn armed receiver same empty
      have remove := Step.trash (racySource := race) (x := {x with permit := some m})
        good.ready member armed rfl (Or.inr same) empty
      exact (by simpa [w, empty, trashFile] using (Reaches.next (.next .here pre) remove))
  have keptReady : w.ready := by
    by_cases h : x.live m.src = absent <;> simpa [w, h, trashFile, State.ready] using good.ready
  have keptArmed : ¬disarmed w m := by
    by_cases h : x.live m.src = absent <;> simpa [w, h, trashFile, disarmed] using armed
  have empty : w.live m.src = absent := by
    by_cases h : x.live m.src = absent <;> simp [w, h, trashFile, put]
  let s := {w with stable := put w.stable m.src absent}
  let t := {s with stableTrash := put s.stableTrash m.src (s.trash m.src)}
  let y := {t with seen := put t.seen m.src (some ⟨true, true⟩), permit := none}
  have syncFile : Step b race w s := .syncSource keptReady member empty (Or.inl keptArmed)
  have syncTrash : Step b race s t := .syncTrash keptReady member
  have check : Step b race t y := by
    apply Step.checkMoved (x := t) keptReady member _ keptArmed _ _ _
    · intro q key file
      have ne : q ≠ m.src := fun eq => file ⟨m, member, eq.symm⟩
      by_cases h : x.live m.src = absent <;>
        simpa [t, s, w, h, trashFile, put, ne] using turn q key file
    · have ne : m.dst ≠ m.src := (endpoint_distinct shape.2.1 member).symm
      by_cases h : x.live m.src = absent <;>
        simpa [certified, durableAt, t, s, w, h, trashFile, put, ne] using receiver
    · simp [durableAt, t, s, empty, put]
    · simp [t, s, put]
  have done : Verified b y m.src := by
    refine ⟨?_, ⟨true, true⟩, ?_, rfl, fun _ => rfl⟩
    · simp [durableAt, y, t, s, empty, absentAfter, put]
    · simp [y, put]
  refine ⟨y, .next (.next (.next write syncFile) syncTrash) check, ?_, ⟨keptReady, ?_, ?_⟩, done, ?_⟩
  · intro q outside
    have ne : q ≠ m.src := fun eq => outside (eq ▸ src)
    by_cases h : x.live m.src = absent <;> simp [y, t, s, w, h, trashFile, put, ne]
  · intro q key
    by_cases h : x.live m.src = absent <;> simpa [y, t, s, w, h, trashFile] using good.untouched q key
  · intro q key
    by_cases eq : q = m.src
    · subst q; exact Or.inr done.1.1
    · by_cases h : x.live m.src = absent <;>
        simpa [y, t, s, w, h, trashFile, put, eq] using good.compatible q key
  · intro q ne old
    by_cases h : x.live m.src = absent <;>
      simpa [Verified, durableAt, y, t, s, w, h, trashFile, put, ne] using old

private theorem drain (eligible track : Slot P → Prop) (condition : State P → Prop)
    (tracked : ∀ p, eligible p → track p)
    (serve : ∀ x p, condition x → eligible p →
      ∃ y, Reaches b race x y ∧ Frame b x y ∧ condition y ∧ Verified b y p ∧
        ∀ q, q ≠ p → track q → Verified b x q → Verified b y q)
    (pending : List (Slot P)) (members : ∀ p ∈ pending, eligible p)
    (x : State P) (initial : condition x) :
    ∃ y, Reaches b race x y ∧ Frame b x y ∧ condition y ∧ (∀ p ∈ pending, Verified b y p) ∧
      ∀ p, track p → Verified b x p → Verified b y p := by
  induction pending generalizing x with
  | nil => exact ⟨x, .here, fun _ _ => ⟨rfl, rfl⟩, initial, by simp, fun _ _ h => h⟩
  | cons p rest ih =>
    obtain ⟨y, xy, fx, cond, done, keep⟩ := serve x p initial (members p (by simp))
    obtain ⟨z, yz, fy, last, tail, frame⟩ := ih
      (fun q h => members q (List.mem_cons_of_mem _ h)) y cond
    refine ⟨z, xy.trans yz, fx.trans fy, last, ?_, ?_⟩
    · intro q h
      rcases List.mem_cons.mp h with rfl | h
      · exact frame q (tracked q (members q h)) done
      · exact tail q h
    · intro q watch verified
      apply frame q watch
      by_cases eq : q = p
      · subst q; exact done
      · exact keep q eq watch verified

private theorem file_keys (shape : b.Shape) (m : Move P) (mem : m ∈ b.moves) :
    m.dst ∈ b.keys ∧ ¬source b m.dst :=
  ⟨(move_endpoints_in_keys b shape m mem).2,
    fun src => source_not_receiver shape src ⟨m, mem, rfl⟩⟩

/-- The driver can drain any clean recovered prefix in receiver, remaining
non-source, source, then retirement order. Live exact outputs are re-witnessed. -/
theorem clean_completion (shape : b.Shape) (good : Clean b x) :
    ∃ y, Reaches b race x y ∧ Frame b x y ∧ y.result = some .applied ∧
      ∀ p ∈ b.keys, durableAt y p (after b p) := by
  classical
  let receivers := b.moves.map Move.dst
  let files := b.keys.filter (fun p => decide (¬source b p ∧ ¬receiver b p))
  let sources := b.moves.map Move.src
  let seenReceivers := fun x => ∀ p, receiver b p → Verified b x p
  let seenFiles := fun x => ∀ p ∈ b.keys, ¬source b p → Verified b x p
  have servesReceiver (x : State P) (p : Slot P) (clean : Clean b x) (recv : receiver b p) :
      ∃ y, Reaches b race x y ∧ Frame b x y ∧ Clean b y ∧ Verified b y p ∧
        ∀ q, q ≠ p → ¬source b q → Verified b x q → Verified b y q := by
    obtain ⟨m, mem, rfl⟩ := recv
    exact finish_file (b := b) (race := race) shape clean
      (file_turn (file_keys shape m mem).1 (file_keys shape m mem).2 (Or.inl ⟨m, mem, rfl⟩))
  obtain ⟨v, xv, fx, goodV, seenV, _⟩ := drain (b := b) (race := race) (receiver b)
    (fun p => ¬source b p) (Clean b) (fun _ recv src => source_not_receiver shape src recv)
    servesReceiver receivers (by simp [receivers, receiver]) x good
  have recvV : seenReceivers v := fun p hp => seenV p (by simpa [receiver, receivers] using hp)
  have servesFile (x : State P) (p : Slot P) (clean : Clean b x ∧ seenReceivers x)
      (file : p ∈ files) :
      ∃ y, Reaches b race x y ∧ Frame b x y ∧ (Clean b y ∧ seenReceivers y) ∧ Verified b y p ∧
        ∀ q, q ≠ p → ¬source b q → Verified b x q → Verified b y q := by
    have ⟨key, ns, _⟩ : p ∈ b.keys ∧ ¬source b p ∧ ¬receiver b p := by simpa [files] using file
    obtain ⟨y, xy, frame, goodY, done, keep⟩ := finish_file (b := b) (race := race) shape clean.1
      (file_turn key ns (Or.inr clean.2))
    refine ⟨y, xy, frame, ⟨goodY, ?_⟩, done, keep⟩
    intro q hq
    by_cases eq : q = p
    · subst q; exact done
    · exact keep q eq (fun src => source_not_receiver shape src hq) (clean.2 q hq)
  obtain ⟨w, vw, fv, goodW, seenW, _⟩ := drain (b := b) (race := race) (· ∈ files)
    (fun p => ¬source b p) (fun x => Clean b x ∧ seenReceivers x)
    (fun p h => ((List.mem_filter.mp h).2 |> of_decide_eq_true).1)
    servesFile files (fun _ h => h) v ⟨goodV, recvV⟩
  have filesW : seenFiles w := by
    intro p key ns
    by_cases recv : receiver b p
    · exact goodW.2 p recv
    · exact seenW p (by simp [files, key, ns, recv])
  have servesSource (x : State P) (p : Slot P) (clean : Clean b x ∧ seenFiles x) (src : source b p) :
      ∃ y, Reaches b race x y ∧ Frame b x y ∧ (Clean b y ∧ seenFiles y) ∧ Verified b y p ∧
        ∀ q, q ≠ p → True → Verified b x q → Verified b y q := by
    obtain ⟨m, mm, rfl⟩ := src
    have turn : sourceTurn b x := by
      intro q key ns
      obtain ⟨_, o, seen, _, _⟩ := clean.2 q key ns
      simp [seen]
    obtain ⟨y, xy, frame, goodY, done, keep⟩ := finish_source (b := b) (race := race) shape clean.1 turn m mm
      (clean.2 m.dst (file_keys shape m mm).1 (file_keys shape m mm).2).1
    refine ⟨y, xy, frame, ⟨goodY, ?_⟩, done, fun _ ne _ h => keep _ ne h⟩
    intro q key ns
    exact keep q (fun eq => ns ⟨m, mm, eq.symm⟩) (clean.2 q key ns)
  obtain ⟨z, wz, fw, goodZ, seenZ, _⟩ := drain (b := b) (race := race) (source b)
    (fun _ => True) (fun x => Clean b x ∧ seenFiles x) (fun _ _ => trivial)
    servesSource sources (by simp [sources, source]) w ⟨goodW.1, filesW⟩
  have verified (p : Slot P) (key : p ∈ b.keys) : Verified b z p := by
    by_cases src : source b p
    · exact seenZ p (by simpa [sources, source] using src)
    · exact goodZ.2 p key src
  have seen : AllSeen b z := by
    intro p key
    obtain ⟨_, o, obs, _, _⟩ := verified p key
    simp [obs]
  have applies : Applied b z := by
    refine ⟨fun p key => ?_, fun m mem => ?_⟩
    · obtain ⟨_, o, obs, exactBytes, _⟩ := verified p key
      exact ⟨o, obs, exactBytes⟩
    · have ⟨src, dst⟩ := move_endpoints_in_keys b shape m mem
      obtain ⟨_, o, obs, _, copy⟩ := verified m.src src
      refine ⟨?_, o, obs, copy ⟨m, mem, rfl⟩⟩
      simp [disarmed, (goodZ.1.untouched m.dst dst).2, (goodZ.1.untouched m.src src).2]
  exact ⟨_, .next (xv.trans (vw.trans wz)) (.retire goodZ.1.ready seen (.applied applies)
    (fun m mem => (applies.2 m mem).2)), fx.trans (fv.trans fw), rfl, fun p hp => (verified p hp).1⟩

theorem clean_relaunch_completion (shape : b.Shape) (pending : x.phase = .pending)
    (noChoices : ∀ p ∈ b.keys, x.choice p = none ∧ x.disarmed p = false)
    (compatible : ∀ p ∈ b.keys, x.live p = before b p ∨ x.live p = after b p) :
    ∃ y, Reaches b race x y ∧ Frame b x y ∧ y.result = some .applied ∧
      ∀ p ∈ b.keys, durableAt y p (after b p) := by
  cases h : x.up
  · obtain ⟨y, trace, frame, result, exactFiles⟩ := clean_completion (race := race) shape
      (x := reset x true) ⟨⟨rfl, pending⟩, noChoices, compatible⟩
    exact ⟨y, (Reaches.next Reaches.here (.launch h)).trans trace, frame, result, exactFiles⟩
  · exact clean_completion shape ⟨⟨h, pending⟩, noChoices, compatible⟩

inductive DriverQueue (b : Batch P) (race : Bool) (start : State P) : Nat → State P → Prop where
  | empty : DriverQueue b race start 0 start
  | append {n x y} : DriverQueue b race start n x → Step b race x y →
      DriverQueue b race start (n + 1) y

private theorem queue_of_reaches (reached : Reaches b race x y) : ∃ n, DriverQueue b race x n y := by
  induction reached with
  | here => exact ⟨0, .empty⟩
  | next _ step ih =>
    obtain ⟨n, queued⟩ := ih
    exact ⟨n + 1, .append queued step⟩

/-- Fair service of the finite queue advances its cursor whenever work remains.
A failed or unscheduled turn can leave it at the same index. -/
def FairService (size : Nat) (cursor : Nat → Nat) : Prop :=
  (∀ t, cursor t ≤ size) ∧
    (∀ t, cursor t < size → ∃ later, t < later ∧ cursor t < cursor later)

theorem finite_queue_serviced (fair : FairService size cursor) : ∃ t, cursor t = size := by
  have reaches (n : Nat) (limit : n ≤ size) : ∃ t, n ≤ cursor t := by
    induction n with
    | zero => exact ⟨0, Nat.zero_le _⟩
    | succ n ih =>
      obtain ⟨t, h⟩ := ih (by omega)
      by_cases done : n + 1 ≤ cursor t
      · exact ⟨t, done⟩
      · obtain ⟨later, _, advances⟩ := fair.2 t (by omega)
        exact ⟨later, by omega⟩
  obtain ⟨t, h⟩ := reaches size (Nat.le_refl _)
  exact ⟨t, Nat.le_antisymm (fair.1 t) h⟩

theorem clean_driver_liveness (shape : b.Shape) (pending : x.phase = .pending)
    (noChoices : ∀ p ∈ b.keys, x.choice p = none ∧ x.disarmed p = false)
    (compatible : ∀ p ∈ b.keys, x.live p = before b p ∨ x.live p = after b p) :
    ∃ size y, DriverQueue b race x size y ∧ y.result = some .applied ∧
      Frame b x y ∧ (∀ p ∈ b.keys, durableAt y p (after b p)) ∧
      ∀ cursor, FairService size cursor → ∃ t, cursor t = size := by
  obtain ⟨y, trace, frame, result, files⟩ := clean_relaunch_completion (race := race)
    shape pending noChoices compatible
  obtain ⟨size, queued⟩ := queue_of_reaches trace
  exact ⟨size, y, queued, result, frame, files, fun _ => finite_queue_serviced⟩

#print axioms clean_completion
#print axioms clean_relaunch_completion
#print axioms clean_driver_liveness

end PairedRecovery
