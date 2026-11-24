import CompositionBatchCodec
import Stage6Spelling

namespace ModelComposition.Spelling
open ListedDisk Storage21

structure EntryOffer where
  binding : Binding
  oldBytes : Bytes
  newBytes : Bytes
  deriving DecidableEq

structure Prepared where
  image : Image
  patch : Patch
  entries : List EntryOffer

def splitOne (receipts : Bool) (s : Prepared) (b : Binding) : Option Prepared :=
  match s.image[b.origin]?, s.patch.effects[b.destination]? with
  | some old, some ⟨none, some new⟩ =>
    if b.origin ≠ b.destination ∧ s.image[b.destination]? = none ∧
        s.patch.effects[b.origin]? = some ⟨some old, none⟩ then
      let effects := (s.patch.effects.erase b.origin).erase b.destination
      let effects := if receipts && old == new then effects else
        effects.insert b.destination ⟨some old, some new⟩
      some ⟨(s.image.erase b.origin).insert b.destination old, {s.patch with effects},
        ⟨b, old, new⟩ :: s.entries⟩
    else none
  | _, _ => none

private theorem splitOne_spec (receipts : Bool) (s t : Prepared) (b : Binding)
    (split : splitOne receipts s b = some t) :
    t.patch.replay t.image = s.patch.replay s.image ∧ t.patch.bindings = s.patch.bindings ∧
    t.entries.map EntryOffer.binding = b :: s.entries.map EntryOffer.binding := by
  unfold splitOne at split
  cases old : s.image[b.origin]? with
  | none => simp [old] at split
  | some oldBytes =>
    cases dst : s.patch.effects[b.destination]? with
    | none => simp [old, dst] at split
    | some destination =>
      rcases destination with ⟨before, after⟩
      cases before with
      | some => simp [old, dst] at split
      | none =>
        cases after with
        | none => simp [old, dst] at split
        | some newBytes =>
          simp only [old, dst] at split
          split at split
          · rename_i guards
            obtain ⟨different, _, origin⟩ := guards
            cases Option.some.inj split
            refine ⟨?_, rfl, rfl⟩
            apply Std.ExtTreeMap.ext_getElem?
            intro path
            rw [Patch.replay_lookup, Patch.replay_lookup]
            by_cases atSource : path = b.origin
            · subst path
              split <;> simp [origin, Ne.symm different]
            · by_cases atDestination : path = b.destination
              · subst path
                split
                · rename_i equal
                  simp only [Bool.and_eq_true, beq_iff_eq] at equal
                  have same := equal.2
                  simp [dst, same]
                · simp [dst]
              · split <;> simp [Std.ExtTreeMap.getElem?_erase, Std.ExtTreeMap.getElem?_insert,
                  Ne.symm atSource, Ne.symm atDestination]
          · contradiction

/-- The caller supplies the observed host route. A lexical fold cannot choose
between a separate entry and a guarded entry-first move. -/
def prepare (entryFirst : Binding → Bool) (receipts : Bool) (image : Image) (patch : Patch) : Option Prepared :=
  let chosen := (crossBindings patch).filter entryFirst
  if ¬ ((crossBindings patch).flatMap fun b => [b.origin, b.destination]).Nodup then none else
    let start : Prepared :=
      ⟨image, {patch with bindings := patch.bindings.filter (fun b => b.origin == b.destination || !entryFirst b)}, []⟩
    (chosen.foldlM (splitOne receipts) start).map fun s => {s with entries := s.entries.reverse}

theorem prepare_spec {entryFirst : Binding → Bool} {receipts : Bool} {image : Image} {patch : Patch}
    {s : Prepared} (result : prepare entryFirst receipts image patch = some s) :
    s.patch.replay s.image = patch.replay image ∧
    s.entries.map EntryOffer.binding = (crossBindings patch).filter entryFirst ∧
    crossBindings s.patch = (crossBindings patch).filter (fun b => !entryFirst b) := by
  have run (entries : List Binding) (u v : Prepared)
      (run : entries.foldlM (splitOne receipts) u = some v) :
      v.patch.replay v.image = u.patch.replay u.image ∧ v.patch.bindings = u.patch.bindings ∧
      v.entries.map EntryOffer.binding = entries.reverse ++ u.entries.map EntryOffer.binding := by
    induction entries generalizing u with
    | nil => cases Option.some.inj run; exact ⟨rfl, rfl, rfl⟩
    | cons b rest ih =>
      simp only [List.foldlM_cons] at run
      cases h : splitOne receipts u b with
      | none => simp [h] at run
      | some mid =>
        obtain ⟨earlier, kept, recorded⟩ := splitOne_spec _ _ _ _ h
        obtain ⟨later, unchanged, restRecorded⟩ := ih mid (by simpa [h] using run)
        exact ⟨later.trans earlier, unchanged.trans kept,
          by simp [restRecorded, recorded, List.reverse_cons, List.append_assoc]⟩
  unfold prepare at result
  split at result
  · contradiction
  · let start : Prepared :=
      ⟨image, {patch with bindings := patch.bindings.filter (fun b => b.origin == b.destination || !entryFirst b)}, []⟩
    change (((crossBindings patch).filter entryFirst).foldlM (splitOne receipts) start).map _ = some s at result
    cases h : ((crossBindings patch).filter entryFirst).foldlM (splitOne receipts) start with
    | none => simp [h] at result
    | some mid =>
      simp only [h, Option.map_some] at result
      cases Option.some.inj result
      obtain ⟨replay, bindings, recorded⟩ := run _ _ _ h
      refine ⟨replay, ?_, ?_⟩
      · simp [start, List.map_reverse, recorded]
      · change (mid.patch.bindings.filter _) = _
        rw [bindings]
        simp only [start, crossBindings, List.filter_filter]
        congr 1
        funext b
        cases same : (b.origin == b.destination) <;> cases route : entryFirst b <;> simp_all [bne]

def Prepared.codec (s : Prepared) : Codec := Codec.forPatch s.image s.patch

def Prepared.move (s : Prepared) (p : s.codec.Slot) : Option Stage6Spelling.Move :=
  (s.entries.find? fun e => e.binding.destination == s.codec.path p).map fun e =>
    ⟨e.binding.origin, e.binding.destination, s.codec.encode (some e.oldBytes),
      s.codec.encode (some e.newBytes)⟩

/-- The host keys are measured for the original listing and the residual
support separately. A sensitive volume can distinguish two folded twins. -/
structure Fits (s : Prepared) (image : Image) (preKey postKey fold : Path → String) : Prop where
  beforeKeys : (image.toList.map (fun entry => preKey entry.1)).Nodup
  residualKeys : (s.codec.paths.map postKey).Nodup
  caseOnly : ∀ e ∈ s.entries, fold e.binding.origin = fold e.binding.destination ∧
    e.binding.origin.toList.reverse.dropWhile (· != '/') =
      e.binding.destination.toList.reverse.dropWhile (· != '/')
  entries : ∀ e ∈ s.entries, ∃ p, s.codec.slot? e.binding.destination = some p ∧
    s.move p = some ⟨e.binding.origin, e.binding.destination, s.codec.encode (some e.oldBytes),
      s.codec.encode (some e.newBytes)⟩ ∧ s.image[e.binding.destination]? = some e.oldBytes

inductive Residual (s : Prepared) (postKey : Path → String) : Option (Stage6.Batch s.codec.paths.length) → Prop where
  | empty : (∀ p, physicalWrite s.image s.patch (s.codec.path p) = none) → Residual s postKey none
  | batch {b} : encodeBatch postKey s.image s.patch = some b → Residual s postKey (some b)

#print axioms prepare_spec

end ModelComposition.Spelling
