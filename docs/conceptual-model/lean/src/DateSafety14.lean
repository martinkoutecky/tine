/- Writer realization is an explicit syntax/complete-text boundary. No desired
reference, query or alias-component preservation is an input hypothesis. -/
import DateEffects14

set_option linter.unusedSectionVars false
namespace CM
namespace Date13
open Params Consolidated13 QueryRename6 Merge7
variable {K L : Type} [DecidableEq K] (P : Inputs K) (dates : Dates K) (M : DatePolicy K) (N : MemberNames P dates) (SP : StoredParser L)

structure AliasParser where
  fields : Format → String → List Alias14.Field

structure MergeRealization (A : AliasParser) (R : CM.Rewriter) (J : Merger)
    (m : String → String) (S D : File) : Prop where
  path : (J.apply m S D).path = D.path
  dir : (J.apply m S D).dir = D.dir
  format : (J.apply m S D).format = D.format
  name : P.key (P.name (J.apply m S D)) = P.key (P.name D)
  aliases : P.aliasDecl (J.apply m S D).format (J.apply m S D).text =
    Alias14.members m (P.aliasDecl D.format D.text) (P.aliasDecl S.format S.text)
  fields : A.fields (J.apply m S D).format (J.apply m S D).text =
    Alias14.mergeField D.format m (P.aliasDecl D.format D.text) (P.aliasDecl S.format S.text)
      (((A.fields D.format D.text).head?).map Alias14.Field.ending |>.getD "\n")
  nonempty : ∀ i, i < survLen P.toParams J m S D → J.origin m S D i ≠ []
  valid : ∀ i, i < survLen P.toParams J m S D → ∀ x ∈ J.origin m S D i,
    (originOcc P.toParams S D x).isSome = true
  occSubL : ∀ i, i < survLen P.toParams J m S D → ∀ o ∈ (blockOcc P.toParams (J.apply m S D) i).getD [],
    ∃ x ∈ J.origin m S D i, ∃ o' ∈ (originOcc P.toParams S D x).getD [], rwOcc R.covers m o' = o
  coverS : ∀ j, j < (P.outline S.format S.text).length → CoversAt P.toParams R J m S D (true,j)
  coverD : ∀ j, j < (P.outline D.format D.text).length → CoversAt P.toParams R J m S D (false,j)

/-- `input` and `role` are fixed independently of the chosen operation. The
scope equation permits consumption only in an actual content merge pair. -/
structure Realizes (Q : QParams K L) (A : AliasParser) (E : Engine) (G : Graph)
    (old : String) (c : Candidate) (input : File → List Piece) (role : Piece → Alias14.Role) where
  rewrites : QueryRename6.FaithfulOn P.toParams SP (rewriter E (subst P dates M old c))
    (subst P dates M old c) G
  trace : Alias14.Trace
  fixedInput : trace.input = input
  fixedRole : trace.role = role
  fixedScope : trace.scope = match source P dates M G old,c.survivor with
    | some S,some D => [S,D] | _,_ => []
  text : Alias14.Valid trace (subst P dates M old c) G (execute P dates M E G old c)
  merge : ∀ S D, source P dates M G old = some S → c.survivor = some D →
    MergeRealization P A (rewriter E (subst P dates M old c)).base (E.merger (subst P dates M old c))
      (subst P dates M old c) S D ∧
    Merge7b.BodyLaw P.toParams Q (rewriter E (subst P dates M old c)).base (E.merger (subst P dates M old c))
      (subst P dates M old c) S D ∧
    Merge7b.QueryLaw P.toParams SP (E.merger (subst P dates M old c)) (subst P dates M old c) S D

theorem offered_complete_text {Q : QParams K L} {A : AliasParser} {E : Engine} {G : Graph}
    {old new : String} {c : Candidate} {extras : List String}
    {input : File → List Piece} {role : Piece → Alias14.Role}
    (_h : c ∈ offers P dates M N SP E G extras old new)
    (F : Realizes P dates M SP Q A E G old c input role)
    {f : File} (hf : f ∈ G) {p : Piece} (hp : p ∈ input f) :
    (∃ out ∈ execute P dates M E G old c, Placement.original p ∈ F.trace.output out) ∨
      Alias14.Approved F.trace (subst P dates M old c) (execute P dates M E G old c) p :=
  Alias14.complete_text F.text hf (by rw [F.fixedInput]; exact hp)

theorem offered_protected_text {Q : QParams K L} {A : AliasParser} {E : Engine} {G : Graph}
    {old new : String} {c : Candidate} {extras : List String}
    {input : File → List Piece} {role : Piece → Alias14.Role}
    (_h : c ∈ offers P dates M N SP E G extras old new)
    (F : Realizes P dates M SP Q A E G old c input role)
    {f : File} (hf : f ∈ G) {p : Piece} (hp : p ∈ input f)
    (hr : role p = .opaqueText) (he : p.editable = false) :
    ∃ out ∈ execute P dates M E G old c, Placement.original p ∈ F.trace.output out ∧
      Placement.render (subst P dates M old c) (.original p) = p.text ∧
      String.join ((F.trace.output out).map (Placement.render (subst P dates M old c))) = out.text :=
  Alias14.opaqueText_bytes F.text hf (by rw [F.fixedInput]; exact hp) (by rw [F.fixedRole]; exact hr) he

/-- Extraction factors through ordinary literal keys, as in the reader. -/
theorem target_key_congr {a b : String} (h : P.key a = P.key b) :
    key P dates a = key P dates b := by simp [key,target,h]

theorem source_page {G : Graph} {old : String} {S : File}
    (h : source P dates M G old = some S) : kindOf S = .page := by
  unfold source at h
  have hh := (Option.filter_eq_some_iff.mp h).2
  simp only [Bool.and_eq_true,beq_iff_eq] at hh
  exact hh.1

theorem claim_of_same_name_path {f g : File} (hd : g.dir = f.dir) (hp : g.path = f.path)
    (hn : P.key (P.name g) = P.key (P.name f)) : claim P dates g = claim P dates f := by
  simp only [claim,hd,hp]
  split
  · cases dates.journalDay f.path with
    | some d => rfl
    | none => exact target_key_congr P dates hn
  · exact target_key_congr P dates hn

theorem realized_journal_fixed {Q : QParams K L} {A : AliasParser} {E : Engine} {G : Graph}
    {old : String} {c : Candidate} {input : File → List Piece} {role : Piece → Alias14.Role}
    (F : Realizes P dates M SP Q A E G old c input role)
    {f : File} (hf : f ∈ G) (hj : kindOf f = .journal) :
    ∃ out ∈ execute P dates M E G old c, out.path = f.path ∧ claim P dates out = claim P dates f := by
  have hkeep : f ∈ kept P dates M G old c := by
    unfold kept
    split
    · apply List.mem_filter.mpr
      refine ⟨hf,decide_eq_true ?_⟩
      intro hs
      have hp := source_page P dates M hs
      rw [hj] at hp; contradiction
    · exact hf
  rw [execute_map]
  refine ⟨outputFile P dates M E G old c f,List.mem_map_of_mem hkeep,?_⟩
  have hR : ((rewriter E (subst P dates M old c)).apply (subst P dates M old c) f).path = f.path ∧
      claim P dates ((rewriter E (subst P dates M old c)).apply (subst P dates M old c) f) = claim P dates f := by
    have hp := F.rewrites.path f hf (Or.inl hj)
    exact ⟨hp,claim_of_same_name_path P dates (F.rewrites.dir f hf) hp
      (by rw [F.rewrites.name f hf]; simp [hj])⟩
  cases hs : source P dates M G old with
  | none => simpa [outputFile,hs] using hR
  | some S =>
    cases ht : c.survivor with
    | none => simpa [outputFile,hs,ht] using hR
    | some D =>
      by_cases he : f = D
      · subst f
        simp only [outputFile,hs,ht,ite_true]
        have ML := (F.merge S D hs ht).1
        exact ⟨ML.path,claim_of_same_name_path P dates ML.dir ML.path ML.name⟩
      · simpa [outputFile,hs,ht,he] using hR

end Date13
end CM
