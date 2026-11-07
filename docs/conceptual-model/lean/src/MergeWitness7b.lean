/- Round 7b merge witnesses (verifier r7 findings 1, 2, 3, 4). New declarations
only; `MergeWitness7`'s data is reused, not changed. `Q` and `SP` are the round-6
query parameters (no pre-blocks); `Q2` adds a pre-block (a first block holding
the token `h`) to exercise the header path of the body law. -/
import Merge7b
import MergeWitness7

set_option maxRecDepth 8192
set_option maxHeartbeats 1600000
namespace CM
namespace MergeWitness7b
open Params Merge7 Merge7b QueryRename6 MergeWitness7
open Witnesses2 (pg blocksOf toks)

abbrev Q : QParams String String := QueryWitness6.Q
abbrev SP : StoredParser String := QueryWitness6.S

/-! ## The accepted merge of `MergeWitness7` satisfies the round-7b contract -/

/-- **MW7 (hypotheses).** `G1`'s merge meets the body law (no pre-blocks under
`Q`: the target's two blocks, then the source's two, each its own block with
its rewritten text), the survivor query law, the rename laws on the source and
target, and the CMQ law on the other files. -/
theorem mw7b_hyps : MergeHyps7b D Q SP R.base JT mm mm S1 T1 G1 where
  toMergeOpHyps := mw_hyps
  lawsSD := {
    dir := by decide +kernel
    format := by decide +kernel
    name := by decide +kernel
    path := by decide +kernel
    occ := by decide +kernel
    alias := by decide +kernel
    order := by decide +kernel }
  sameFormat := by decide +kernel
  body := by decide +kernel
  queries := by decide +kernel
  storedOthers := by decide +kernel

/-- **MW8 (a negation-free stored query goes forward, and may gain).** The
source's block 1 stores `refs tgt`; it reaches the survivor's block 3
unchanged (`m tgt = tgt`). Before, it matched `src` 1, `tgt` 0, `tgt` 1 and
`src/kid` 0; after, it matches all their images (`tgt` 3, `tgt` 0, `tgt` 1,
`tgt/kid` 0) and also `tgt` 2, `c` 0 and `c` 1, which referenced only `src`:
forward inclusion, not equality. -/
theorem mw_positive_forward :
    storedAt D SP G1 ("src", 1) = [.leaf (.refs "tgt")] ∧
    storedAt D SP After1 ("tgt", 3) = [.leaf (.refs "tgt")] ∧
    Decided.den D Q G1 .block (.leaf (.refs "tgt")) =
      [.block ("src", 1), .block ("tgt", 0), .block ("tgt", 1), .block ("src/kid", 0)] ∧
    Decided.den D Q After1 .block (.leaf (.refs "tgt")) =
      [.block ("tgt", 0), .block ("tgt", 1), .block ("tgt", 2), .block ("tgt", 3),
       .block ("c", 0), .block ("c", 1), .block ("tgt/kid", 0)] := by decide +kernel

/-! ## Finding 1: the verifier's collapse is no longer a lawful merger -/

def S0 : File := pg "src" "l:x"
def T0 : File := pg "tgt" "l:x"
def G0 : Graph := [S0, T0]
def m0 : String → String := nsSubst D.key D.urlHasOrigin "src" "tgt"

/-- The verifier's counterexample merger: discards the source body and claims
both old body blocks as origins of the one survivor block. -/
def Collapse : Merger where
  apply _ _ t := t
  origin _ _ _ _ := [(false, 0), (true, 0)]

/-- **MW9 (collapse rejected).** Two ordinary blocks become one: the body law
fails, so no `MergeHyps7b` instance exists for it (for any stored-query parser
and any key map). It satisfied every round-7 hypothesis (verifier,
`VerifyCounterexamples.lean`). -/
theorem collapse_rejected : ¬ BodyLaw D Q R.base Collapse m0 S0 T0 := by decide +kernel

theorem collapse_no_hyps (SP' : StoredParser String) (σ : String → String) :
    ¬ MergeHyps7b D Q SP' R.base Collapse m0 σ S0 T0 G0 :=
  fun H => collapse_rejected H.body

/-- **MW10 (query loss rejected).** The verifier's second counterexample: the
source block stores `v:src`; the collapsed survivor stores nothing. The query
law fails (and so does the body law). -/
def Sq : File := pg "src" "l:x v:src"
theorem query_loss_rejected : ¬ QueryLaw D SP Collapse m0 Sq T0 ∧
    ¬ BodyLaw D Q R.base Collapse m0 Sq T0 := by decide +kernel

/-! ## The header path of the body law -/

/-- Query parameters with a pre-block: a first block holding the token `h`. -/
def Q2 : QParams String String :=
  { QueryWitness6.Q with preBlock := fun _ t => (blocksOf t).head?.any (fun b => (toks b).contains "h") }

def S4 : File := pg "src" "h a:aka;l:x"
def T4 : File := pg "tgt" "h;l:y"
def G4 : Graph := [S4, T4]

/-- **MW11 (header path).** With pre-blocks, `JT`'s survivor is: the target's
header (p = 1), the target's body, the source's header as the one moved block
(q = 1), the source's body; the body law holds with that layout. -/
theorem mw_header_layout :
    merge D R JT G4 "src" "tgt" = some [pg "tgt" "h;l:y;h a:aka;l:x"] ∧
    hdr D Q2 S4 = 1 ∧ hdr D Q2 T4 = 1 ∧
    BodyAt D Q2 R.base JT mm S4 T4 1 1 ∧ BodyLaw D Q2 R.base JT mm S4 T4 := by decide +kernel

/-! ## Finding 3: bare alias members (model follows the decided rename rule 6.2) -/

def G3 : Graph := [pg "src" "a:src/friend", pg "tgt" ""]

/-- **MW12 (alias member rewritten in the model).** The source declares the
alias `src/friend`; in the model's survivor it reads `tgt/friend`, as the
decided rename contract (rule 6.2, `RwLaws.alias`) requires. Tine's merge
leaves `alias:: src/friend` unrewritten (a conformance difference, SPEC §9). -/
theorem mw_alias_member :
    merge D R JT G3 "src" "tgt" = some [pg "tgt" ";a:tgt/friend"] := by decide +kernel

/-! ## Finding 2: boundaries the operation accepts and the theorems exclude -/

def Gj : Graph := [pg "src" "l:z", ⟨"2026_11_02", .journals, .md, "l:w"⟩, pg "c" "l:src"]
def GjAfter : Graph :=
  [⟨"2026_11_02", .journals, .md, "l:w;l:z"⟩, pg "c" "l:2026_11_02"]

/-- **MW13 (journal survivor: accepted, outside the theorems).** Merging a
page into a journal is accepted. The source's block joins the journal, and
the referrer now links to the journal. `MergeOpHyps.dstPage` fails, and so
does the real-page theorem's statement (`real_merge7`): the σ-image of the
real page `src` is the page-kind `2026_11_02`, which is not a page of the
after-graph (the journal is). Round 8 (M-Q7, allowed): every conjunct still
holds; the theorems now cover this case through `MergeBase` (no `dstPage`) and
`Merge8.real_merge8`/`journal_references`; instance
`MergeWitness8.mw_journal_hyps`. -/
theorem mw_journal_boundary :
    mergeAllowed D Gj "src" "2026_11_02" = true ∧
    merge D R JT Gj "src" "2026_11_02" = some GjAfter ∧
    kindOf (⟨"2026_11_02", .journals, .md, "l:w"⟩ : File) ≠ .page ∧
    (.page, "src") ∈ Decided.pages D Gj ∧
    σp (nsSubst D.key D.urlHasOrigin "src" "2026_11_02") ((.page, "src") : Page String) = (.page, "2026_11_02") ∧
    (.page, "2026_11_02") ∉ Decided.pages D GjAfter ∧
    (.journal, "2026_11_02") ∈ Decided.pages D GjAfter ∧
    D.referencesB GjAfter ("c", 0) (.journal, "2026_11_02") = true := by decide +kernel

def Gs2 : Graph := [pg "a" "l:z", pg "a/b" "", pg "c" "l:a/b;l:a"]
def Gs2After : Graph := [pg "a/b" ";l:z", pg "c" "l:a/b/b;l:a/b"]

/-- **MW14 (survivor inside the source's namespace: refused since round 8).**
Martin's M-Q6 ruling (D-20261102-MC): merging `a` into its own child `a/b` is
refused (`mergeAllowed` round-8 clause, `merge_outside_subtree`). Round 7
accepted it; this witness kept that acceptance and is superseded. What round 7's
operation would have produced is kept as a historical fact about its body
`mergeBy … (nsSubst key "a" "a/b")` (the operation's output when the guard
passed): the key map sends the target's own name `a/b` to `a/b/b`, so the
referrer's link `l:a/b` becomes `l:a/b/b`, a name no file claims. That is why
the round-7 theorems excluded it, and why the refusal loses nothing. -/
theorem mw_subtree_boundary :
    mergeAllowed D Gs2 "a" "a/b" = false ∧
    merge D R JT Gs2 "a" "a/b" = none ∧
    nsSubst D.key D.urlHasOrigin "a" "a/b" "a/b" = "a/b/b" ∧
    mergeBy R.base JT (nsSubst D.key D.urlHasOrigin "a" "a/b") (pg "a" "l:z") (pg "a/b" "") Gs2 = Gs2After ∧
    D.occAt Gs2 ("c", 0) = [(.link, "a/b")] ∧ D.occAt Gs2After ("c", 0) = [(.link, "a/b/b")] ∧
    D.claimants Gs2After (.page, "a/b/b") = [] := by decide +kernel

end MergeWitness7b
end CM
