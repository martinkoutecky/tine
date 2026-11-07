/- Executable comparison graphs. Token syntax is a fixture extractor, not
Markdown/Org serialization. Full candidates, safety and after-graphs are logged. -/
import Consolidated13Comparison

set_option maxRecDepth 8192
set_option maxHeartbeats 4000000
namespace CM
namespace Witness13
open Params Consolidated13
open Witnesses2 (pg)
abbrev P := Inputs.ofParams Witness11.D
abbrev SP := Witness11.SP
def E : Engine := ⟨Witness11.R, Witness11.J⟩
def jr (name text : String) : File := ⟨name, .journals, .md, text⟩
def accept (G : Graph) (old new : String) := safe P SP E G [] old (literal P G old new)
def after (G : Graph) (old new : String) := execute P E G old (literal P G old new)
def oldOffers (G : Graph) (old new : String) := Rulings11.offers P.toParams G old new

def w1 : Graph := [pg "ML notes" "notes", pg "Machine Learning" "a:ML;owner",
  pg "ML notes/x" "child", pg "Q" "e:ML notes|l:ML notes/x"]
def w1b : Graph := [pg "O" "a:X", pg "Machine Learning" "a:ML;owner", pg "Q" "e:X|v:X"]
def w2 : Graph := [pg "A" "body", pg "O1" "a:X", pg "O2" "a:X"]
def w2y : Graph := w2 ++ [pg "R" "l:Y"]
def w3 : Graph := [pg "A" "parent", pg "A/x" "child", pg "R" "l:A|l:A/x"]
def w3a : Graph := [pg "A" "parent", pg "O" "a:A/x", pg "R" "l:A|l:A/x"]
def w4 : Graph := [pg "A" "l:A/x", jr "day" "a:A/x"]
def w4a : Graph := [jr "day" "a:Today", pg "R" "l:Today"]
def w5 : Graph := [pg "A" "body", jr "A/x" "journal"]
def w6 : Graph := [pg "A" "body", pg "P" "a:A/x|l:P", jr "A/x" "journal"]
def w7 : Graph := [pg "A/x" "body", jr "A" "journal", pg "R" "l:A/x/y"]
def PD : Inputs String := { P with refKind := fun k => if k == "2026-11-06" then .journal else .page }
def w8 : Graph := [pg "R" "l:2026-11-06"]
def w8b : Graph := [jr "day" "body", pg "R" "l:day"]
def w10 : Graph := [pg "TODO" "body", pg "R" "k:TODO|l:TODO"]
def w11 : Graph := [pg "A" "l:A/x", pg "B/x" "stationary"]
def w12 : Graph := [pg "X" "", pg "P" "a:X"]
def w15 : Graph := [pg "A" "source", pg "B" "target", pg "O" "a:A/x",
  pg "u1" "t:B/x|one", pg "u2" "t:B/x|two"]
def wTwin : Graph := [pg "A/x" "", pg "zz" "t:A/x|l:Hidden/y|a:Hidden"]

theorem W1_literal_and_options : accept w1 "ML notes" "ML" = true ∧
    after w1 "ML notes" "ML" = [pg "ML" "notes", pg "Machine Learning" "a:ML;owner",
      pg "ML/x" "child", pg "Q" "e:ML|l:ML/x"] ∧
    (offers P SP E w1 [] "ML notes" "ML").length = 2 := by decide +kernel
theorem W1b_no_content : accept w1b "X" "ML" = true ∧
    after w1b "X" "ML" = [pg "O" "a:ML", pg "Machine Learning" "a:ML;owner",
      pg "Q" "e:ML|v:ML"] := by decide +kernel
theorem W2_many_declarers : oldOffers w2 "A" "X" = [.cancel] ∧
    accept w2 "A" "X" = true ∧ accept w2y "Y" "X" = true := by decide +kernel
theorem W3_simultaneous : oldOffers w3 "A" "A/x" = [.cancel] ∧
    accept w3 "A" "A/x" = true ∧ after w3 "A" "A/x" =
      [pg "A/x" "parent", pg "A/x/x" "child", pg "R" "l:A/x|l:A/x/x"] ∧
    accept w3a "A" "A/x" = true := by decide +kernel
theorem W4_journal_alias : oldOffers w4 "A" "B" = [.cancel] ∧
    accept w4 "A" "B" = true ∧ accept w4a "Today" "Now" = true ∧
    after w4 "A" "B" = [pg "B" "l:B/x", jr "day" "a:B/x"] := by decide +kernel
theorem W5_unused_anchor : oldOffers w5 "A" "B" = [.rename, .cancel] ∧
    accept w5 "A" "B" = false := by decide +kernel
theorem W6_alias_anchor : oldOffers w6 "A" "B" = [.rename, .cancel] ∧
    accept w6 "A" "B" = false := by decide +kernel
theorem W7_derived_parent : oldOffers w7 "A/x" "B/z" = [.cancel] ∧
    accept w7 "A/x" "B/z" = true ∧
    Consolidated13.referencesB P w7 ("R", 0) (.journal, "A") = true ∧
    Consolidated13.referencesB P (after w7 "A/x" "B/z") ("R", 0) (.journal, "A") = false ∧
    (("R", 0), ("R", 0)) ∈ preview P E w7 "A/x" (literal P w7 "A/x" "B/z") := by
  decide +kernel
theorem W8_virtual_date : Rulings11.offers PD.toParams w8 "2026-11-06" "other" = [.cancel] ∧
    safe PD SP E w8 [] "2026-11-06" (literal PD w8 "2026-11-06" "other") = true ∧
    oldOffers w8b "day" "day" = [.cancel] ∧ accept w8b "day" "day" = true := by
  decide +kernel
theorem W10_fixed_task : oldOffers w10 "TODO" "Next" = [.rename, .cancel] ∧
    accept w10 "TODO" "Next" = false := by decide +kernel
theorem W11_retained_capture : oldOffers w11 "A" "B" = [.cancel] ∧
    accept w11 "A" "B" = false := by decide +kernel
theorem W12_real_empty : P.resolve w12 "X" = (.page, "P") ∧
    Consolidated13.resolve P w12 "X" = (.page, "X") := by decide +kernel
theorem W15_alias_image_conflict : oldOffers w15 "A" "B" = [.mergeOwner, .cancel] ∧
    accept w15 "A" "B" = false ∧
    "B/x" ∈ touched P SP w15 [] "A" (literal P w15 "A" "B") := by decide +kernel
theorem W16_twins : activeGraph P wTwin = [pg "A/x" ""] ∧
    (readParams P wTwin).aliasEdges (activeGraph P wTwin) = [] ∧
    (readParams P wTwin).nameKeys (activeGraph P wTwin) = [] ∧
    P.nameKeys wTwin = ["A"] ∧
    (Consolidated13.blocks P wTwin) = [("A/x", 0)] := by decide +kernel

def key9 (s : String) : Fin 7 :=
  if s == "A" then 0 else if s == "B" then 1
  else if s == "C" || s == "A/x" then 2 else if s == "B/x" then 3
  else if s == "O" then 4 else if s == "R" then 5 else 6
def spell9 (k : Fin 7) : String :=
  if k == 0 then "A" else if k == 1 then "B" else if k == 2 then "C"
  else if k == 3 then "B/x" else if k == 4 then "O" else if k == 5 then "R" else "_"
theorem key9laws : KeyLaws key9 spell9 := ⟨by decide +kernel⟩
def code9 (p : Page (Fin 7)) : List Nat :=
  [p.2.val, if p.1 == .page then 1 else 0]
theorem code9_inj (p q : Page (Fin 7)) (h : code9 p = code9 q) : p = q := by
  obtain ⟨a, i⟩ := p
  obtain ⟨b, j⟩ := q
  have he : i = j := Fin.ext (List.cons.inj h).1
  cases a <;> cases b <;> simp_all [code9]
def P9 : Inputs (Fin 7) where
  key := key9
  stem := P.stem
  title := P.title
  aliasDecl := P.aliasDecl
  outline := P.outline
  occ := P.occ
  fileLe := P.fileLe
  pageLe := fun a b => decide (code9 a ≤ code9 b)
  refKind := fun _ => .page
  kindPref := .page
  aliasKinds := fun _ => true
theorem P9_lawful : P9.toParams.Lawful where
  file := Orders.fileLe_linear
  page := {
    total := fun a b => by simpa [P9] using (Std.le_total (a := code9 a) (b := code9 b))
    antisymm := fun a b h1 h2 => code9_inj a b
      (Std.le_antisymm (by simpa [P9] using h1) (by simpa [P9] using h2))
    trans := fun a b c h1 h2 => by
      simpa [P9] using (Std.le_trans (by simpa [P9] using h1 : code9 a ≤ code9 b)
        (by simpa [P9] using h2 : code9 b ≤ code9 c)) }
def w9 : Graph := [pg "A" "text", pg "O" "a:A/x", pg "R" "l:C"]
theorem W9_inconsistent_keys : Rulings11.offers P9.toParams w9 "A" "B" = [.rename, .cancel] ∧
    safe P9 SP E w9 [] "A" (literal P9 w9 "A" "B") = false := by decide +kernel

def PK : Inputs String := { P with key := String.toLower }
def w14 : Graph := [pg "A" "body", pg "B" "body", pg "Q" "e:A|l:A"]
theorem W14_one_map :
    Rulings11.answer PK.toParams Witness11.R Witness11.J w14 "A" "b" .mergeOwner =
      some [pg "B" "body;body", pg "Q" "e:B|l:b"] ∧
    safe PK SP E w14 [] "A" (literal PK w14 "A" "b") = true ∧
    execute PK E w14 "A" (literal PK w14 "A" "b") =
      [pg "B" "body;body", pg "Q" "e:b|l:b"] := by decide +kernel

/-- Without realization, the finite guard cannot certify arbitrary writers.
This is a counterexample to an unconditional safety claim, not a valid engine. -/
def Bad : Engine := ⟨⟨fun m _ f => { f with path := m f.path, text := "" }⟩, Witness11.J⟩
theorem W17_realization_needed :
    safe P SP Bad [pg "A" "valuable prose"] [] "A"
      (literal P [pg "A" "valuable prose"] "A" "B") = true ∧
    execute P Bad [pg "A" "valuable prose"] "A"
      (literal P [pg "A" "valuable prose"] "A" "B") = [pg "B" ""] := by decide +kernel

def w18 : Graph := [pg "A" "source", pg "B" "a:C;target", pg "C" "other"]
theorem W18_real_group_choice : oldOffers w18 "A" "B" = [.mergeOwner, .cancel] ∧
    (offers P SP E w18 [] "A" "B").map Candidate.destination = ["B", "C"] ∧
    automatic P SP E w18 [] "A" "B" = none := by decide +kernel

def P19 : Inputs String := { P with stem := fun f =>
  if f.path == "journal-X" then "X" else f.path }
def w19 : Graph := [pg "X" "page", jr "journal-X" "journal|a:Hidden|l:Hidden"]
theorem W19_cross_kind_twin : P19.blocks w19 = [("X", 0), ("journal-X", 0)] ∧
    activeGraph P19 w19 = [pg "X" "page"] ∧
    Consolidated13.blocks P19 w19 = [("X", 0)] ∧
    (readParams P19 w19).aliasEdges (activeGraph P19 w19) = [] := by decide +kernel
theorem W19_paths_unique : WF w19 := by unfold WF; decide +kernel

def w20 : Graph := [pg "A" "source", pg "B" "target", pg "Q" "e:A/x",
  pg "u1" "t:B/x|one", pg "u2" "t:B/x|two"]
theorem W20_query_image_conflict : oldOffers w20 "A" "B" = [.mergeOwner, .cancel] ∧
    accept w20 "A" "B" = false := by decide +kernel

def w22 : Graph := [pg "O" "a:X", pg "B" "target", pg "Q" "e:X"]
theorem W22_fileless_literal_spelling :
    Rulings11.answer PK.toParams Witness11.R Witness11.J w22 "X" "b" .mergeOwner =
      some [pg "O" "a:b", pg "B" "target", pg "Q" "e:B"] ∧
    safe PK SP E w22 [] "X" (literal PK w22 "X" "b") = true ∧
    execute PK E w22 "X" (literal PK w22 "X" "b") =
      [pg "O" "a:b", pg "B" "target", pg "Q" "e:b"] := by decide +kernel

def scenarios : List (String × Graph × String × String) :=
  [("W1", w1, "ML notes", "ML"), ("W1b", w1b, "X", "ML"),
   ("W2", w2, "A", "X"), ("W2Y", w2y, "Y", "X"),
   ("W3", w3, "A", "A/x"), ("W3alias", w3a, "A", "A/x"),
   ("W4", w4, "A", "B"), ("W4Today", w4a, "Today", "Now"),
   ("W5", w5, "A", "B"), ("W6", w6, "A", "B"), ("W7", w7, "A/x", "B/z"),
   ("W10", w10, "TODO", "Next"), ("W11", w11, "A", "B"), ("W15", w15, "A", "B")]
#eval scenarios.map (fun (label, g, a, b) =>
  (label, oldOffers g a b, candidates P g a b, offers P SP E g [] a b,
    defaultCandidate P SP E g [] a b, after g a b,
    preview P E g a (literal P g a b)))

end Witness13
end CM
