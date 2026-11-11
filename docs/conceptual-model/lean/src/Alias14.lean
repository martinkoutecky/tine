/- Exact-spelling alias serialization and retained-or-approved-consumed text.
Classification belongs to the fixed input partition, not to the candidate. -/

import Journal14

namespace CM
namespace Alias14
open Consolidated13

inductive Role
  | opaqueText
  | name
  | key (format : Format)
  | delimiter
  deriving DecidableEq, Repr

/-- Deliberately narrow isSyntax. Odd spaces, wrappers and unknown remainders
are opaqueText spans; this whitelist cannot consume them. Native Org is #+alias:. -/
def isSyntax (role : Role) (s : String) : Bool :=
  match role with
  | .key .md => s == "alias:: " || s == "aliases:: "
  | .key .org => s == "#+alias: " || s == "#+aliases: "
  | .delimiter => s == ", " || s == "," || s == "\n" || s == "\r\n"
  | _ => false

/-- One native active field. Survivor spellings precede source spellings;
substitution precedes exact-byte deduplication. No semantic-key deduplication. -/
def members (m : String → String) (survivor source : List String) : List String :=
  ((survivor ++ source).map m).eraseDups

def fieldPrefix : Format → String
  | .md => "alias:: "
  | .org => "#+alias: "

def line (fmt : Format) (names : List String) (ending : String) : String :=
  fieldPrefix fmt ++ ", ".intercalate names ++ ending

structure Field where
  format : Format
  names : List String
  ending : String
  deriving DecidableEq, Repr

def Field.text (f : Field) : String := line f.format f.names f.ending

def mergeField (fmt : Format) (m : String → String) (survivor source : List String)
    (ending : String) : List Field :=
  if (members m survivor source).isEmpty then [] else [⟨fmt,members m survivor source,ending⟩]

theorem one_field (fmt : Format) (m : String → String) (survivor source : List String) (ending : String)
    (h : members m survivor source ≠ []) :
    (mergeField fmt m survivor source ending).length = 1 := by
  simp [mergeField,List.isEmpty_iff,h]

theorem members_exact (m : String → String) (survivor source : List String) (s : String) :
    s ∈ members m survivor source ↔ ∃ old ∈ survivor ++ source, m old = s := by
  simp only [members,List.mem_eraseDups,List.mem_map]

theorem erase_unique (xs : List String) : xs.eraseDups.Nodup := by
  cases xs with
  | nil => simp
  | cons a rest =>
    rw [List.eraseDups_cons,List.nodup_cons]
    exact ⟨by simp,erase_unique _⟩
termination_by xs.length
decreasing_by exact Nat.lt_succ_of_le (List.length_filter_le _ _)

theorem members_unique (m : String → String) (survivor source : List String) :
    (members m survivor source).Nodup := erase_unique _

/-- The classifier is fixed before the candidate, alongside the input spans.
Each consumed token is separately listed: conservation retains multiplicities. -/
structure Trace where
  input : File → List Piece
  role : Piece → Role
  output : File → List Placement
  consumed : List Piece
  scope : List File

def retained (T : Trace) (H : Graph) : List Piece :=
  H.flatMap (fun f => (T.output f).filterMap Placement.original?)

/-- Syntax consumption requires a retained native declaration key. Duplicate
names require an identical rendered retained NAME, never arbitrary matching prose. -/
def Approved (T : Trace) (m : String → String) (H : Graph) (p : Piece) : Prop :=
  (isSyntax (T.role p) p.text = true ∧
    ∃ q ∈ retained T H, ∃ fmt, T.role q = .key fmt ∧ isSyntax (T.role q) q.text = true) ∨
  (T.role p = .name ∧ ∃ q ∈ retained T H, T.role q = .name ∧ p.render m = q.render m)

structure Valid (T : Trace) (m : String → String) (G H : Graph) : Prop where
  before : ∀ f ∈ G, String.join ((T.input f).map Piece.text) = f.text
  after : ∀ f ∈ H, String.join ((T.output f).map (Placement.render m)) = f.text
  partition : (G.flatMap T.input).Perm (retained T H ++ T.consumed)
  approved : ∀ p ∈ T.consumed, Approved T m H p
  inScope : ∀ p ∈ T.consumed, ∃ f ∈ T.scope, f ∈ G ∧ p ∈ T.input f

theorem retained_placement {T : Trace} {H : Graph} {p : Piece} (h : p ∈ retained T H) :
    ∃ f ∈ H, Placement.original p ∈ T.output f := by
  obtain ⟨f,hf,hp⟩ := List.mem_flatMap.mp h
  obtain ⟨x,hx,he⟩ := List.mem_filterMap.mp hp
  cases x with
  | original q => cases Option.some.inj he; exact ⟨f,hf,hx⟩
  | insert s => cases he

/-- Complete-text headline: each input fragment is retained OR is exactly
approved alias isSyntax/duplicate-name consumption with its retained witness. -/
theorem complete_text {T : Trace} {m : String → String} {G H : Graph}
    (V : Valid T m G H) {f : File} (hf : f ∈ G) {p : Piece} (hp : p ∈ T.input f) :
    (∃ out ∈ H, Placement.original p ∈ T.output out) ∨ Approved T m H p := by
  have hm := V.partition.mem_iff.mp (List.mem_flatMap.mpr ⟨f,hf,hp⟩)
  rcases List.mem_append.mp hm with hr | hc
  · exact Or.inl (retained_placement hr)
  · exact Or.inr (V.approved p hc)

theorem opaqueText_not_consumed {T : Trace} {m : String → String} {G H : Graph}
    (V : Valid T m G H) {p : Piece} (hp : T.role p = .opaqueText) : p ∉ T.consumed := by
  intro hc
  rcases V.approved p hc with hs | hn
  · simp [hp,isSyntax] at hs
  · simp [hp] at hn

theorem opaqueText_bytes {T : Trace} {m : String → String} {G H : Graph}
    (V : Valid T m G H) {f : File} (hf : f ∈ G) {p : Piece} (hp : p ∈ T.input f)
    (hr : T.role p = .opaqueText) (he : p.editable = false) :
    ∃ out ∈ H, Placement.original p ∈ T.output out ∧
      Placement.render m (.original p) = p.text ∧
      String.join ((T.output out).map (Placement.render m)) = out.text := by
  have hm := V.partition.mem_iff.mp (List.mem_flatMap.mpr ⟨f,hf,hp⟩)
  have hret : p ∈ retained T H := (List.mem_append.mp hm).resolve_right (opaqueText_not_consumed V hr)
  obtain ⟨out,ho,hp⟩ := retained_placement hret
  exact ⟨out,ho,hp,by simp [Placement.render,Piece.render,he],V.after out ho⟩

theorem opaqueText_multiplicity {T : Trace} {m : String → String} {G H : Graph}
    (V : Valid T m G H) :
    ((G.flatMap T.input).filter (fun p => T.role p == .opaqueText)).Perm
      ((retained T H).filter (fun p => T.role p == .opaqueText)) := by
  have hp := V.partition.filter (fun p => T.role p == .opaqueText)
  have he : T.consumed.filter (fun p => T.role p == .opaqueText) = [] := by
    apply List.filter_eq_nil_iff.mpr
    intro p hp
    cases hr : T.role p <;> simp_all [opaqueText_not_consumed V]
  simpa [List.filter_append,he] using hp

end Alias14
end CM
