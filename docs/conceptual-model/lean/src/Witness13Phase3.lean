/- Kernel witnesses for the repaired boundary and exhaustive comparison. -/
import Consolidated13Coverage

set_option maxRecDepth 8192
set_option maxHeartbeats 4000000
namespace CM
namespace Witness13Phase3
open Params Consolidated13 Merge7
open Witnesses2 (pg)
abbrev P := Witness13.P
abbrev SP := Witness13.SP
abbrev E := Witness13.E
abbrev Q := Witness11.Q

def emptyTarget : Graph := [pg "A" "valuable prose", pg "B/x" ""]
def c := literal P emptyTarget "A" "B/x"
def m := subst P "A" c

def input (f : File) : List Piece :=
  if f.path == "A" then [⟨0, "valuable prose", false⟩] else [⟨1, "", false⟩]
def trace : TextTrace := ⟨input, fun _ =>
  [.original ⟨1, "", false⟩, .insert ";", .original ⟨0, "valuable prose", false⟩]⟩

theorem empty_faithful : QueryRename6.FaithfulOn P.toParams SP (rewriter E m) m emptyTarget := by
  constructor
  · constructor <;> intro f hf <;> simp [emptyTarget] at hf <;>
      rcases hf with rfl | rfl <;> decide +kernel
  · intro f hf; simp [emptyTarget] at hf
    rcases hf with rfl | rfl <;> decide +kernel

theorem empty_trace : TraceValid trace m emptyTarget (execute P E emptyTarget "A" c) := by
  constructor
  · intro f hf; simp [emptyTarget] at hf
    rcases hf with rfl | rfl <;> decide +kernel
  · intro f hf
    have he : execute P E emptyTarget "A" c = [pg "B/x" ";valuable prose"] := by decide +kernel
    rw [he] at hf; simp at hf; subst f; decide +kernel
  · decide +kernel

theorem empty_merge : MergeRealization P (rewriter E m).base (E.merger m) m
    (transport (demands P SP emptyTarget [] "A" c)) (pg "A" "valuable prose") (pg "B/x" "") := by
  constructor
  all_goals first | decide +kernel | skip
  all_goals
    intro j hj
    change j < 1 at hj
    have he : j = 0 := by omega
    subst j
    decide +kernel

def empty_realizes : Realizes P SP Q E emptyTarget "A" c input where
  rewrites := empty_faithful
  trace := trace
  fixedInput := rfl
  text := empty_trace
  merge := by
    intro S D hs hd
    have heS : source P emptyTarget "A" = some (pg "A" "valuable prose") := by decide +kernel
    have heD : c.survivor = some (pg "B/x" "") := by decide +kernel
    have hs' : S = pg "A" "valuable prose" := (Option.some.inj (heS.symm.trans hs)).symm
    have hd' : D = pg "B/x" "" := (Option.some.inj (heD.symm.trans hd)).symm
    subst S; subst D
    refine ⟨empty_merge, ?_, ?_⟩ <;> decide +kernel

theorem empty_offered_and_recomputed :
    c ∈ offers P SP E emptyTarget [] "A" "B/x" ∧
    execute P E emptyTarget "A" c = [pg "B/x" ";valuable prose"] ∧
    P.fileNsNames (pg "B/x" "") = [] ∧
    P.fileNsNames (pg "B/x" ";valuable prose") = ["B"] ∧
    (.page, "B") ∈ pages P (execute P E emptyTarget "A" c) := by decide +kernel

theorem empty_text_safe : ∃ f' ∈ execute P E emptyTarget "A" c,
    Placement.original ⟨0, "valuable prose", false⟩ ∈ empty_realizes.trace.output f' ∧
    Placement.render m (.original ⟨0, "valuable prose", false⟩) = "valuable prose" ∧
    String.join ((empty_realizes.trace.output f').map (Placement.render m)) = f'.text := by
  exact offered_text_safety P SP empty_offered_and_recomputed.1 empty_realizes
    (f := pg "A" "valuable prose") (by simp [emptyTarget]) (by decide +kernel) rfl

def fileless : Graph := [pg "B" "target", pg "R" "l:X|e:X"]
theorem fileless_comparison :
    (comparisonCases P SP E fileless [] "X" "B" .mergeOwner).all (fun x => !x.2) = true ∧
    defaultCandidate P SP E fileless [] "X" "B" = some (literal P fileless "X" "B") ∧
    Rulings11.answer P.toParams E.rewrite E.merger fileless "X" "B" .mergeOwner =
      some (execute P E fileless "X" (literal P fileless "X" "B")) := by decide +kernel

/-- The verifier's text-first order is ignored by the concrete reader. -/
def textFirst (f : File) : File := { f with path := f.text, text := f.path }
def PT : Inputs String := { P with fileLe := fun f g => Orders.fileLe (textFirst f) (textFirst g) }
def twins : Graph := [pg "A" "root", pg "u1" "t:U|a:A", pg "u2" "t:U|a:C"]
theorem concrete_twins :
    (activeGraph PT twins).map File.path = ["A", "u1"] ∧
    (activeGraph PT (execute PT E twins "A" (literal PT twins "A" "Z"))).map File.path = ["Z", "u1"] ∧
    (readParams PT (execute PT E twins "A" (literal PT twins "A" "Z"))).aliasEdges
      (activeGraph PT (execute PT E twins "A" (literal PT twins "A" "Z"))) = [("U", "Z")] ∧
    PT.fileLe (pg "u1" "t:U|a:Z") (pg "u2" "t:U|a:C") ≠
      (physicalParams PT).fileLe (pg "u1" "t:U|a:Z") (pg "u2" "t:U|a:C") := by decide +kernel

theorem empty_realization_exists : Nonempty (Realizes P SP Q E emptyTarget "A" c input) := ⟨empty_realizes⟩

/-- Every flag in the earlier inventory has an independently executed graph. -/
theorem original_flag_witnesses :
    ("alias destination/options", true) ∈ differenceCases P SP Witness13.w1 [] "ML notes" "ML" ∧
    ("additional group choice", true) ∈ differenceCases P SP Witness13.w18 [] "A" "B" ∧
    ("moving destination", true) ∈ differenceCases P SP Witness13.w3 [] "A" "A/x" ∧
    ("fixed anchor", true) ∈ differenceCases P SP Witness13.w5 [] "A" "B" ∧
    ("inconsistent sites", true) ∈ differenceCases Witness13.P9 SP Witness13.w9 [] "A" "B" ∧
    ("image conflict", true) ∈ differenceCases P SP Witness13.w15 [] "A" "B" ∧
    ("journal closure loss", true) ∈ differenceCases P SP Witness13.w7 [] "A/x" "B/z" ∧
    ("new source kind", true) ∈ differenceCases Witness13.PD SP Witness13.w8 [] "2026-11-06" "other" ∧
    ("literal target spelling", true) ∈ differenceCases Witness13.PK SP Witness13.w14 [] "A" "b" ∧
    ("twins", true) ∈ differenceCases P SP Witness13.wTwin [] "A/x" "B" := by decide +kernel

theorem selection_flag_witnesses :
    ("legacy own-page route", true) ∈ comparisonCases P SP E Witness13.w1 [] "ML notes" "ML" .ownPage ∧
    ("rename/content-pair selection", true) ∈ comparisonCases P SP E emptyTarget [] "A" "B/x" .rename ∧
    ("merge/content-pair selection", true) ∈ comparisonCases P SP E Witness13.w1 [] "ML notes" "ML" .mergeOwner ∧
    ("old target spelling", true) ∈ comparisonCases Witness13.PK SP E Witness13.w14 [] "A" "b" .mergeOwner := by decide +kernel

def mixed : Graph := [pg "A" "source", ⟨"B", .pages, .org, "target"⟩]
def duplicatePaths : Graph := [pg "A" "first", pg "A" "second"]
def collide : Engine := ⟨⟨fun _ _ f => { f with path := "one" }⟩, Witness11.J⟩
def two : Graph := [pg "A" "first", pg "R" "second"]

/-- Input/output-path and bad-writer rows deliberately exercise boundary errors;
they are diagnostic coverage, not new adopted product differences. -/
theorem refusal_flag_witnesses :
    ("source support", false) ∈ refusalChecks P SP E [] [] "X" "B" ∧
    ("key consistency", false) ∈ refusalChecks P SP E Witness13.w10 [] "TODO" "Next" ∧
    ("touched conflicts", false) ∈ refusalChecks P SP E Witness13.w15 [] "A" "B" ∧
    ("content pair", false) ∈ refusalChecks P SP E mixed [] "A" "B" ∧
    ("output claims", false) ∈ refusalChecks P SP collide two [] "A" "B" ∧
    ("virtual capture", false) ∈ refusalChecks P SP E Witness13.w11 [] "A" "B" ∧
    ("input paths", false) ∈ refusalChecks P SP E duplicatePaths [] "A" "B" ∧
    ("output paths", false) ∈ refusalChecks P SP collide two [] "A" "B" := by decide +kernel

def keyNS (s : String) : Fin 4 :=
  if s.startsWith "A" then 0 else if s.startsWith "B" then 1 else if s == "R" then 2 else 3
def spellNS (k : Fin 4) : String := if k == 0 then "A" else if k == 1 then "B" else if k == 2 then "R" else "_"
theorem keyNS_laws : KeyLaws keyNS spellNS := ⟨by decide +kernel⟩
def PNS : Inputs (Fin 4) where
  key := keyNS
  stem := P.stem
  title := P.title
  aliasDecl := P.aliasDecl
  outline := P.outline
  occ := P.occ
  fileLe := Orders.fileLe
  pageLe := fun p q => decide ([p.2.val, if p.1 == .page then 1 else 0] ≤ [q.2.val, if q.1 == .page then 1 else 0])
  refKind := fun _ => .page
  kindPref := .page
  aliasKinds := fun _ => true

def nsGraph : Graph := [pg "A" "source", pg "B" "target", pg "R" "e:A/x"]
/-- Equal root keys can still select an earlier namespace prefix. This is an
actual safe old/new query-text difference beyond the original target-spelling flag. -/
theorem namespace_query_split :
    safe PNS SP E nsGraph [] "A" (literal PNS nsGraph "A" "B") = true ∧
    Rulings11.answer PNS.toParams E.rewrite E.merger nsGraph "A" "B" .mergeOwner =
      some [pg "B" "target;source", pg "R" "e:B"] ∧
    execute PNS E nsGraph "A" (literal PNS nsGraph "A" "B") =
      [pg "B" "target;source", pg "R" "e:B/x"] ∧
    Rulings9.queryMap PNS.toParams "A" "B" (pg "B" "target") "A/x" ≠
      nsSubst PNS.key PNS.urlHasOrigin "A" "B" "A/x" := by decide +kernel

theorem old_inventory_incomplete :
    (differenceCases PNS SP nsGraph [] "A" "B").all (fun x => !x.2) = true ∧
    Rulings11.answer PNS.toParams E.rewrite E.merger nsGraph "A" "B" .mergeOwner ≠
      some (execute PNS E nsGraph "A" (literal PNS nsGraph "A" "B")) := by decide +kernel

theorem map_split_witness : mapSplit PNS nsGraph "A" "B" :=
  ⟨pg "B" "target", by decide +kernel, "A/x", namespace_query_split.2.2.2⟩

def ranks : Graph := [pg "z" "t:A", pg "a" "t:B"]
def noJournalAlias : Inputs String := { P with aliasKinds := fun k => k == .page }
theorem static_flag_witnesses :
    activeGraph P Witness13.wTwin ≠ Witness13.wTwin ∧
    emptyReal P Witness13.w12 "X" = true ∧
    noJournalAlias.aliasKinds .journal ≠ true ∧
    (readParams P ranks).pageLe (.page, "A") (.page, "B") ≠ P.pageLe (.page, "A") (.page, "B") := by decide +kernel

/-- Explicit query-result changes: stored root argument moves but the file-less
alias still has no literal page, whereas an owner-named old query has a row. -/
theorem literal_query_result_change :
    den P Q (execute P E Witness13.w1b "X" (literal P Witness13.w1b "X" "ML")) .page (.leaf (.name "ML")) = [] ∧
    den P Q [pg "O" "a:ML", pg "Machine Learning" "a:ML;owner", pg "Q" "e:Machine Learning|v:Machine Learning"]
      .page (.leaf (.name "Machine Learning")) = [.page (.page, "Machine Learning")] := by decide +kernel

end Witness13Phase3
end CM
