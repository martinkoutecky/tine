import Stage6PairSafety

namespace Stage6Spelling
open Storage21

/-- The slot index supplies host identity. The two paths are directory listings. -/
structure Move where
  source : String
  destination : String
  expected : Int
  offered : Int
  deriving DecidableEq

inductive Place where
  | source | temporary | destination | trash
  deriving DecidableEq, Repr

structure Entry where
  live : Place := .source
  stable : Place := .source
  observation : Option Bool := none
  oldCopy : Bool := false
  command : Bool := false
  deriving DecidableEq, Repr

def Entry.listed (e : Entry) (m : Move) : Option String :=
  match e.live with
  | .source => some m.source
  | .destination => some m.destination
  | .temporary | .trash => none

def Entry.receipted (e : Entry) (m : Move) (sink : Bool) : List String :=
  if !(sink && e.observation.isSome) then [] else
    if m.expected = m.offered then [m.source, m.destination] else [m.source]

def Entry.privateBytes (e : Entry) (m : Move) : Option Int :=
  match e.live with
  | .temporary | .trash => some m.expected
  | _ => none

def Entry.cut (e : Entry) (keep : Bool) : Entry :=
  let selected := if keep then e.live else e.stable
  {e with live := selected, stable := selected, observation := none, command := false}

inductive EntryStep (m : Move) (running : Bool) : Entry → Entry → Prop where
  | retain {e} : running = true → e.live = .source → m.expected ≠ m.offered →
      EntryStep m running e {e with oldCopy := true}
  | direct {e} : running = true → e.live = .source → (m.expected = m.offered ∨ e.oldCopy = true) →
      EntryStep m running e {e with live := .destination, command := true}
  | first {e} : running = true → e.live = .source → (m.expected = m.offered ∨ e.oldCopy = true) →
      EntryStep m running e {e with live := .temporary, command := true}
  | second {e} : running = true → e.command = true → e.live = .temporary → e.stable = .temporary →
      EntryStep m running e {e with live := .destination}
  | rollback {e} : running = true → e.command = true → e.live = .temporary → e.stable = .temporary →
      EntryStep m running e {e with live := .source, command := false}
  | sync {e} : EntryStep m running e {e with stable := e.live}
  | observe {e} (confirmed : Bool) : running = true → e.command = true → e.live = .destination →
      (confirmed = true → e.stable = .destination) →
      EntryStep m running e {e with observation := some confirmed, command := false}
  | recover {e} (blocked : Bool) : running = false → e.live = .temporary →
      EntryStep m running e {e with live := if blocked then .trash else .destination}

structure State (P : Nat) where
  core : Stage6.State P
  entry : Fin P → Entry

def State.start (x : Stage6.State P) : State P := ⟨x, fun _ => {}⟩

/-- Ordinary storage retains responsibility for byte writes. During a temporary
or trash placement, its label at that slot does not describe a listed entry. -/
def State.listed (x : State P) (m : Move) (p : Fin P) : Option (String × Int) :=
  ((x.entry p).listed m).bind fun path =>
    let label := x.core.core.s.disk p
    if label = absent then none else some (path, label)

def afterAction (x : State P) (a : Stage6.Action P) (keepEntry : Fin P → Bool) : Fin P → Entry :=
  match a with
  | .prior (.old .crash) => fun p => {x.entry p with observation := none, command := false}
  | .prior (.old (.power _)) => fun p => (x.entry p).cut (keepEntry p)
  | _ => x.entry

def EntryGuard (x : State P) (p : Fin P) (m : Move) : Prop :=
  m.source ≠ m.destination ∧ m.expected ≠ absent ∧
  x.core.core.s.disk p = m.expected ∧ x.core.core.s.stable p = m.expected ∧
  Storage3.opFree x.core.core.s [p] = true ∧ Stage6.touches x.core.gates [p] = false ∧
  ((x.core.core.s.pages p).held = true → clean (x.core.core.s.pages p) = true)

/-- Each entry step represents a successful primitive or a fresh listing
observation. A refused primitive with no effect leaves the prefix as it was. -/
inductive Step (moves : Fin P → Option Move) (c : Config P) : State P → State P → Prop where
  | storage {x y} (a : Stage6.Action P) (keepEntry : Fin P → Bool) :
      Stage6.step c x.core a = some y → Step moves c x ⟨y, afterAction x a keepEntry⟩
  | entry {x p m e} : moves p = some m →
      ((x.entry p).live = .source → EntryGuard x p m) →
      ((x.entry p).live = .temporary → (e.listed m).isSome = true →
        x.core.core.s.disk p = m.expected) →
      EntryStep m x.core.core.s.alive (x.entry p) e →
      Step moves c x {x with entry := fun q => if q = p then e else x.entry q}

inductive Prefix (moves : Fin P → Option Move) (c : Config P) (start : State P) : State P → Prop where
  | here : Prefix moves c start start
  | advance {x y} : Prefix moves c start x → Step moves c x y → Prefix moves c start y

theorem Prefix.append {moves : Fin P → Option Move} {c : Config P} {x y z : State P}
    (first : Prefix moves c x y) (later : Prefix moves c y z) : Prefix moves c x z := by
  induction later with
  | here => exact first
  | advance _ step ih => exact .advance ih step

theorem project_prefix {moves : Fin P → Option Move} {c : Config P} {start z : State P}
    {initial : Fin P → Int} (base : Stage6.Reachable c initial start.core)
    (run : Prefix moves c start z) : Stage6.Reachable c initial z.core := by
  induction run with
  | here => exact base
  | advance _ step ih =>
    cases step with
    | storage a _ step => exact .advance ih a step
    | entry => exact ih

theorem lift_reachable {c : Config P} {initial : Fin P → Int} {z : Stage6.State P}
    (run : Stage6.Reachable c initial z) :
    Prefix (fun _ => none) c (State.start (Stage6.initWith c initial)) (State.start z) := by
  induction run with
  | initial => exact .here
  | advance past a step ih =>
    rename_i x y
    have frame : afterAction (State.start x) a (fun _ => false) = (State.start y).entry := by
      cases a with
      | prior a => cases a with
        | old a => cases a <;> rfl
        | _ => rfl
      | _ => rfl
    have next := Step.storage (moves := fun _ => none) (x := State.start x) a (fun _ => false) step
    rw [frame] at next
    exact .advance ih next

theorem old_copy_survives {moves : Fin P → Option Move} {c : Config P} {start z : State P}
    (run : Prefix moves c start z) (p : Fin P) (retained : (start.entry p).oldCopy = true) :
    (z.entry p).oldCopy = true := by
  induction run with
  | here => exact retained
  | advance _ step ih =>
    cases step with
    | storage a keep _ =>
      cases a with
      | prior a => cases a with
        | old a => cases a <;> exact ih
        | _ => exact ih
      | _ => exact ih
    | @entry q m e _ _ _ step =>
      by_cases same : p = q
      · subst q
        cases step <;> simp [ih]
      · simpa [same] using ih

theorem destination_survives {moves : Fin P → Option Move} {c : Config P} {start z : State P}
    (run : Prefix moves c start z) (p : Fin P)
    (settled : (start.entry p).live = .destination ∧ (start.entry p).stable = .destination) :
    (z.entry p).live = .destination ∧ (z.entry p).stable = .destination := by
  induction run with
  | here => exact settled
  | advance _ step ih =>
    cases step with
    | storage a keep _ =>
      cases a with
      | prior a => cases a with
        | old a => cases a <;> first | exact ih | simp [afterAction, Entry.cut, ih.1, ih.2]
        | _ => exact ih
      | _ => exact ih
    | @entry q m e _ _ _ step =>
      by_cases same : p = q
      · subst q
        cases step <;> simp_all
      · simpa [same] using ih

theorem retain_before_rewrite {moves : Fin P → Option Move} {c : Config P}
    {start : Stage6.State P} {z : State P} (run : Prefix moves c (State.start start) z)
    (p : Fin P) (m : Move) (chosen : moves p = some m) (rewrite : m.expected ≠ m.offered) :
    (z.entry p).oldCopy = true ∨ ((z.entry p).live = .source ∧ (z.entry p).stable = .source) := by
  induction run with
  | here => exact Or.inr ⟨rfl, rfl⟩
  | advance _ step ih =>
    cases step with
    | storage a keep _ =>
      cases a with
      | prior a => cases a with
        | old a =>
          cases a <;> try exact ih
          rcases ih with copied | ⟨live, stable⟩
          · exact Or.inl copied
          · exact Or.inr (by simp [afterAction, Entry.cut, live, stable])
        | _ => exact ih
      | _ => exact ih
    | @entry q other e planned _ _ step =>
      by_cases same : p = q
      · subst q
        rw [chosen] at planned
        cases Option.some.inj planned
        rcases ih with copied | ⟨live, stable⟩ <;> cases step <;> simp_all
      · simpa [same] using ih

theorem private_entry_custody (x : State P) (m : Move) (p : Fin P)
    (privateEntry : (x.entry p).live = .temporary ∨ (x.entry p).live = .trash) :
    x.listed m p = none ∧ (x.entry p).privateBytes m = some m.expected := by
  rcases privateEntry with h | h <;> simp [State.listed, Entry.listed, Entry.privateBytes, h]

#print axioms project_prefix
#print axioms lift_reachable
#print axioms old_copy_survives
#print axioms destination_survives
#print axioms retain_before_rewrite
#print axioms private_entry_custody

end Stage6Spelling
