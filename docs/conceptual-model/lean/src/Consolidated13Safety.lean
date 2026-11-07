/- Syntax realization and a finite text-edit certificate. A certificate exposes
all unchanged text and every permitted replacement; it is not an occurrence-only
"no loss" assumption. Production parsing/certificate construction is unverified. -/
import Consolidated13Proofs

set_option linter.unusedSectionVars false
namespace CM
namespace Consolidated13
open Params Merge7 QueryRename6
variable {K L : Type} [DecidableEq K] (P : Inputs K) (SP : StoredParser L)

structure Piece where
  id : Nat
  text : String
  editable : Bool
  deriving DecidableEq, Repr

def Piece.render (m : String → String) (p : Piece) : String :=
  if p.editable then m p.text else p.text

inductive Placement where
  | original (p : Piece)
  | insert (text : String)
  deriving DecidableEq, Repr

def Placement.original? : Placement → Option Piece
  | .original p => some p
  | .insert _ => none

def Placement.render (m : String → String) : Placement → String
  | .original p => p.render m
  | .insert s => s

/-- The span parser supplies `input`: a partition of the whole raw file, not
just its outline. Its editable spans must be exactly legal name-edit spans
(title/aliases/covered occurrences/stored arguments), excluding fixed markers.
That grammar obligation is separate from the algebraic certificate below. -/
structure TextTrace where
  input : File → List Piece
  output : File → List Placement

/-- Checkable equations over concrete text. Permutation tracks multiplicities
and provenance of original fragments. Insertions represent serialization syntax.
No equality of reference sets or desired safety conclusion occurs as a field. -/
structure TraceValid (T : TextTrace) (m : String → String) (G H : Graph) : Prop where
  before : ∀ f ∈ G, String.join ((T.input f).map Piece.text) = f.text
  after : ∀ f ∈ H, String.join ((T.output f).map (Placement.render m)) = f.text
  partition : (G.flatMap T.input).Perm
    (H.flatMap (fun f => (T.output f).filterMap Placement.original?))

theorem trace_original_survives {T : TextTrace} {m : String → String} {G H : Graph}
    (V : TraceValid T m G H) {f : File} (hf : f ∈ G) {p : Piece} (hp : p ∈ T.input f) :
    ∃ f' ∈ H, Placement.original p ∈ T.output f' := by
  have hm : p ∈ G.flatMap T.input := List.mem_flatMap.mpr ⟨f, hf, hp⟩
  have hn := V.partition.mem_iff.mp hm
  obtain ⟨f', hf', h⟩ := List.mem_flatMap.mp hn
  obtain ⟨x, hx, he⟩ := List.mem_filterMap.mp h
  cases x with
  | original q => cases Option.some.inj he; exact ⟨f', hf', hx⟩
  | insert s => cases he

theorem trace_protected_bytes {T : TextTrace} {m : String → String} {G H : Graph}
    (V : TraceValid T m G H) {f : File} (hf : f ∈ G) {p : Piece} (hp : p ∈ T.input f)
    (he : p.editable = false) :
    ∃ f' ∈ H, Placement.original p ∈ T.output f' ∧
      Placement.render m (.original p) = p.text ∧
      String.join ((T.output f').map (Placement.render m)) = f'.text := by
  obtain ⟨f', hf', h⟩ := trace_original_survives V hf hp
  exact ⟨f', hf', h, by simp [Placement.render, Piece.render, he], V.after f' hf'⟩

def execute_map (E : Engine) (G : Graph) (old : String) (c : Candidate) : Prop :=
  execute P E G old c = (kept P G old c).map (outputFile P E G old c)

theorem execute_map_eq (E : Engine) (G : Graph) (old : String) (c : Candidate) :
    execute_map P E G old c := by
  have ht : G.filter (fun _ => true) = G := by induction G <;> simp_all
  unfold execute_map execute kept outputFile
  cases hs : source P G old <;> cases hd : c.survivor <;>
    simp [hs, hd, mergeBy, renameBy, Decided.Rewriter.base, eq_comm, ht]

/-- Current merge syntax boundary. Namespace support is computed from the
resulting complete text and effective name; it is not pinned to the empty or
nonempty state of the separately rewritten target. The other inherited fields
specify physical identity, alias union, exact occurrence provenance and coverage,
not preservation of a derived semantic reading. -/
structure MergeRealization (R : CM.Rewriter) (J : Merger) (m : String → String)
    (σ : K → K) (S D : File) : Prop where
  path : (J.apply m S D).path = D.path
  dir : (J.apply m S D).dir = D.dir
  format : (J.apply m S D).format = D.format
  name : P.key (P.name (J.apply m S D)) = P.key (P.name D)
  aliasL : (∀ s ∈ P.aliasDecl (J.apply m S D).format (J.apply m S D).text,
      ∃ s0 ∈ P.aliasDecl D.format D.text ++ P.aliasDecl S.format S.text, σ (P.key s0) = P.key s) ∧
    (∀ s0 ∈ P.aliasDecl D.format D.text ++ P.aliasDecl S.format S.text,
      ∃ s ∈ P.aliasDecl (J.apply m S D).format (J.apply m S D).text, P.key s = σ (P.key s0))
  nonempty : ∀ i, i < survLen P.toParams J m S D → J.origin m S D i ≠ []
  valid : ∀ i, i < survLen P.toParams J m S D → ∀ x ∈ J.origin m S D i, (originOcc P.toParams S D x).isSome = true
  occSubL : ∀ i, i < survLen P.toParams J m S D → ∀ o ∈ (blockOcc P.toParams (J.apply m S D) i).getD [],
    ∃ x ∈ J.origin m S D i, ∃ o' ∈ (originOcc P.toParams S D x).getD [], rwOcc R.covers m o' = o
  coverS : ∀ j, j < (P.outline S.format S.text).length → CoversAt P.toParams R J m S D (true, j)
  coverD : ∀ j, j < (P.outline D.format D.text).length → CoversAt P.toParams R J m S D (false, j)

/-- Grammar, query and layout laws are explicit; the finite guard does not
pretend to prove that an arbitrary writer obeys them. Header promotion is the
specified nonclashing-property union, still an unimplemented parser boundary.
Span legality is a production obligation on T.input, as described above. -/
structure Realizes (Q : QParams K L) (E : Engine) (G : Graph) (old : String) (c : Candidate)
    (input : File → List Piece) where
  rewrites : QueryRename6.FaithfulOn P.toParams SP (rewriter E (subst P old c)) (subst P old c) G
  trace : TextTrace
  fixedInput : trace.input = input
  text : TraceValid trace (subst P old c) G (execute P E G old c)
  merge : ∀ S D, source P G old = some S → c.survivor = some D →
    MergeRealization P (rewriter E (subst P old c)).base (E.merger (subst P old c))
      (subst P old c) (transport (demands P SP G [] old c)) S D ∧
    Merge7b.BodyLaw P.toParams Q (rewriter E (subst P old c)).base (E.merger (subst P old c))
      (subst P old c) S D ∧
    Merge7b.QueryLaw P.toParams SP (E.merger (subst P old c)) (subst P old c) S D

theorem offered_text_safety {Q : QParams K L} {E : Engine} {G : Graph}
    {old new : String} {c : Candidate} {extras : List String}
    {input : File → List Piece}
    (_h : c ∈ offers P SP E G extras old new) (F : Realizes P SP Q E G old c input)
    {f : File} (hf : f ∈ G) {p : Piece} (hp : p ∈ input f)
    (he : p.editable = false) :
    ∃ f' ∈ execute P E G old c, Placement.original p ∈ F.trace.output f' ∧
      Placement.render (subst P old c) (.original p) = p.text ∧
      String.join ((F.trace.output f').map (Placement.render (subst P old c))) = f'.text :=
  trace_protected_bytes F.text hf (by rw [F.fixedInput]; exact hp) he

theorem journal_anchor_mem {G : Graph} {f : File} (hf : f ∈ G)
    (hj : kindOf f = .journal) : P.name f ∈ anchors P G := by
  apply List.mem_flatMap.mpr
  refine ⟨f, hf, ?_⟩
  simp [anchors, hj]

theorem safe_journal_transport {E : Engine} {G : Graph} {extras : List String} {old : String}
    {c : Candidate} (h : safe P SP E G extras old c = true) {f : File} (hf : f ∈ G)
    (hj : kindOf f = .journal) :
    transport (demands P SP G extras old c) (P.key (P.name f)) = P.key (P.name f) :=
  safe_anchor P SP h (journal_anchor_mem P hf hj)

theorem safe_no_new_claim_collision {E : Engine} {G : Graph} {extras : List String}
    {old : String} {c : Candidate} (h : safe P SP E G extras old c = true)
    {f g : File} (hf : f ∈ kept P G old c) (hg : g ∈ kept P G old c)
    (he : (P.claim (outputFile P E G old c f)).2 =
      (P.claim (outputFile P E G old c g)).2) : (P.claim f).2 = (P.claim g).2 := by
  have ho : ownership P E G old c = true := by
    simp only [safe, Bool.and_eq_true] at h
    exact h.1.1.1.2
  simp only [ownership, List.all_eq_true, decide_eq_true_eq] at ho
  exact ho f hf g hg he

theorem safe_local_conflict {E : Engine} {G : Graph} {extras : List String}
    {old : String} {c : Candidate} (h : safe P SP E G extras old c = true)
    {k : K} (hk : k ∈ touched P SP G extras old c) : P.conflictedB G k = false := by
  have hc : localClean P SP G extras old c = true := by
    simp only [safe, Bool.and_eq_true] at h
    exact h.1.1.1.1.1.2
  have hx := List.all_eq_true.mp hc k hk
  cases he : P.conflictedB G k
  · rfl
  · simp [he] at hx

theorem source_page {G : Graph} {old : String} {S : File}
    (h : source P G old = some S) : kindOf S = .page := by
  unfold source at h
  cases hu : uniqueClaim P G (P.key old) with
  | none => simp [hu] at h
  | some f =>
    simp [hu] at h
    exact h.2

theorem realized_journal_fixed {Q : QParams K L} {E : Engine} {G : Graph}
    {old : String} {c : Candidate} {input : File → List Piece}
    (F : Realizes P SP Q E G old c input)
    {f : File} (hf : f ∈ G) (hj : kindOf f = .journal) :
    ∃ f' ∈ execute P E G old c, f'.path = f.path ∧ P.claim f' = P.claim f := by
  have hkeep : f ∈ kept P G old c := by
    unfold kept
    split
    · apply List.mem_filter.mpr
      refine ⟨hf, decide_eq_true ?_⟩
      intro he
      have hp := source_page P he
      rw [hj] at hp
      contradiction
    · exact hf
  have hm := execute_map_eq P E G old c
  unfold execute_map at hm
  rw [hm]
  refine ⟨outputFile P E G old c f, List.mem_map_of_mem hkeep, ?_⟩
  have hR : ((rewriter E (subst P old c)).apply (subst P old c) f).path = f.path ∧
      P.claim ((rewriter E (subst P old c)).apply (subst P old c) f) = P.claim f := by
    exact ⟨F.rewrites.path f hf (Or.inl hj), by
    unfold claim
    have hk : kindOf ((rewriter E (subst P old c)).apply (subst P old c) f) = kindOf f := by
      unfold kindOf; rw [F.rewrites.dir f hf]
    rw [hk, F.rewrites.name f hf]
    simp [hj]⟩
  cases hs : source P G old with
  | none => simpa only [outputFile, hs] using hR
  | some S =>
    cases hd : c.survivor with
    | none => simpa only [outputFile, hs, hd] using hR
    | some D =>
      by_cases he : f = D
      · subst f
        simp only [outputFile, hs, hd, ite_true]
        have ML := (F.merge S D hs hd).1
        refine ⟨ML.path, ?_⟩
        unfold claim
        have hk : kindOf ((E.merger (subst P old c)).apply (subst P old c) S D) = kindOf D := by
          unfold kindOf; rw [ML.dir]
        rw [hk, ML.name]
      · simpa only [outputFile, hs, hd, he, ite_false] using hR

theorem fixed_marker_spelling (m : String → String) (s : String) :
    rwOcc Decided.covers m (.taskMarker, s) = (.taskMarker, s) ∧
    rwOcc Decided.covers m (.priority, s) = (.priority, s) := by
  simp [rwOcc, Decided.covers]

end Consolidated13
end CM
