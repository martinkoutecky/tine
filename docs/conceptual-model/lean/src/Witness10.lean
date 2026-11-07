/- Round 10 witnesses (OG parity: S15, the leading-slash empty prefix, R13, alias
spellings in the namespace tree). Toy parser of rounds 6-9 (`QueryWitness6.D`, key = id;
tokens `t:` title, `a:` alias, `l:` link, `g:` tag, `e:` stored `(page "...")` query;
blocks split by `;`). `DU` is `D` with a supplied URL-origin test (every `https://`
spelling has an origin, as `gp-util/url?` says, `util.cljs:82`); `mailto:` has none there
(origin "null"), so it stays a namespace name, as in OG. No production-parser claim. -/
import Ref10

set_option maxRecDepth 8192
set_option maxHeartbeats 1600000
namespace CM
namespace Witness10
open Params Merge7 Rulings9 Ref10
open Witnesses2 (pg)

abbrev D : Params String := QueryWitness6.D
abbrev R : Decided.Rewriter := QueryWitness6.R
abbrev R9 : QRewriter := Witness9.R9
abbrev JT : Merger := MergeWitness7.JT

def DU : Params String := { D with urlHasOrigin := fun s => s.startsWith "https://" }

/-! ## 1. S15: relative and URL names have no namespace prefixes, everywhere -/

/-- The fixture `r1-relative-and-url-names`, plus `mailto:` (a namespace in OG). -/
def GS : Graph := [pg "Notes" "l:./x l:../x/y l:https://a.b/c;l:a/b l:mailto:m/z"]

/-- **W10-1 (references and page existence).** `[[./x]]`, `[[../x/y]]` and
`[[https://a.b/c]]` reference only themselves; the control `[[a/b]]` still supplies `a`. -/
theorem relative_url_refs :
    DU.refNames GS ("Notes", 0) = ["./x", "../x/y", "https://a.b/c"] ∧
    DU.refNames GS ("Notes", 1) = ["a/b", "a", "mailto:m/z", "mailto:m"] ∧
    Decided.pages DU GS = [(.page, "Notes"), (.page, "./x"), (.page, "../x/y"),
      (.page, "https://a.b/c"), (.page, "a/b"), (.page, "a"), (.page, "mailto:m/z"),
      (.page, "mailto:m")] := by decide +kernel

/-- **W10-2 (Linked References and links).** No block references `.`, `..`, `../x`,
`https:` or `https://a.b`: none of them is a page, so none has Linked References, a count,
or an incoming link. -/
theorem relative_url_no_backlinks :
    [".", "..", "../x", "https:", "https://a.b"].all (fun s =>
      !(Decided.pages DU GS).contains (.page, s) &&
      !DU.referencesB GS ("Notes", 0) (.page, s) &&
      !DU.referencesB GS ("Notes", 1) (.page, s)) = true ∧
    DU.referencesB GS ("Notes", 1) (.page, "a") = true := by decide +kernel

/-- **W10-3 (the round-9 statement it replaces).** Round 9 closed every spelling with
`nsClosure`, which gives `./x` the parent `.`. -/
theorem round9_closure_counterexample :
    nsClosure "./x" = ["./x", "."] ∧ DU.occClosure "./x" = ["./x"] := by decide +kernel

def GSr : Graph := [pg "." "", pg "Notes" "l:./x l:a/b"]

/-- **W10-4 (rename).** Renaming a page `.` leaves `[[./x]]` alone (it is not in `.`'s
subtree); renaming the reference-only `./x` rewrites it whole; renaming `a` moves the
control `[[a/b]]` to `[[b/b]]`. -/
theorem relative_url_rename :
    Decided.rename DU R GSr "." "z" = some [pg "z" "", pg "Notes" "l:./x l:a/b"] ∧
    rename10 DU R GS "./x" "q" =
      some [pg "Notes" "l:q l:../x/y l:https://a.b/c;l:a/b l:mailto:m/z"] ∧
    rename10 DU R GS "a" "b" =
      some [pg "Notes" "l:./x l:../x/y l:https://a.b/c;l:b/b l:mailto:m/z"] := by decide +kernel

/-- **W10-5 (views).** The views' parent asks the same test: none for the relative and URL
names, `a` for `a/b`. -/
theorem relative_url_views :
    nsParent DU "./x" = none ∧ nsParent DU "../x/y" = none ∧
    nsParent DU "https://a.b/c" = none ∧ nsParent DU "a/b" = some "a" ∧
    nsParent DU "a/b/c" = some "a/b" := by decide +kernel

/-! ## 2. Leading slash (CM11: drop empty prefixes; deliberate OG difference) -/

/-- **W10-6, restated by R10-Q1 (round 11).** A title /n/y supplies /n
but no empty page. OG keeps the empty prefix; Martin explicitly rejects it. -/
theorem title_leading_slash :
    D.fileNsNames (pg "p" "t:/n/y") = ["/n"] ∧
    Decided.pages D [pg "p" "t:/n/y"] = [(.page, "/n/y"), (.page, "/n")] := by
  decide +kernel

/-- **W10-7.** A reference `[[/n/y]]` supplies `/n` but no `""` (OG `with-page-refs`
removes blank names, `block.cljs:367`). -/
theorem ref_leading_slash :
    D.refNames [pg "q" "l:/n/y"] ("q", 0) = ["/n/y", "/n"] ∧
    Decided.pages D [pg "q" "l:/n/y"] = [(.page, "q"), (.page, "/n/y"), (.page, "/n")] := by
  decide +kernel

/-! ## 3. R13: a reference-only source -/

def J : File := ⟨"2026-10-26", .journals, .md, "g:B"⟩
/-- The R13 case: `#B` in a journal block, a page file `n/y`. -/
def GR13 : Graph := [J, pg "n/y" "body"]
def GR13' : Graph := [⟨"2026-10-26", .journals, .md, "g:n/y"⟩, pg "n/y" "body"]

/-- **W10-8 (the round-9 refusal, R13).** `B` is a reference-only page; round 9's rename
and merge both refuse `B → n/y`. -/
theorem r13_round9_refuses :
    refSource D GR13 "B" = true ∧
    Decided.rename D R GR13 "B" "n/y" = none ∧
    merge D R JT GR13 "B" "n/y" = none := by decide +kernel

/-- **W10-9 (R13: onto a real page, a merge).** `rename10` refuses (a real target), and
`refMerge` re-links the tag to `n/y`: the journal block now references `n/y`, and `B` is no
longer a page (OG `merge-pages!`). -/
theorem r13_merge :
    rename10 D R GR13 "B" "n/y" = none ∧
    refMerge D R9 GR13 "B" "n/y" = some GR13' ∧
    D.referencesB GR13' ("2026-10-26", 0) (.page, "n/y") = true ∧
    (Decided.pages D GR13').contains (.page, "B") = false := by decide +kernel

/-- **W10-10 (onto an unused name, a rename).** -/
theorem r13_unused :
    rename10 D R [J] "B" "c" = some [⟨"2026-10-26", .journals, .md, "g:c"⟩] ∧
    refMerge D R9 [J] "B" "c" = none := by decide +kernel

/-- **W10-11 (onto a reference-only name, a rename that joins the two).** -/
theorem r13_ref_only_target :
    rename10 D R [J, pg "p" "l:x"] "B" "x" =
      some [⟨"2026-10-26", .journals, .md, "g:x"⟩, pg "p" "l:x"] ∧
    refMerge D R9 [J, pg "p" "l:x"] "B" "x" = none := by decide +kernel

def JA : File := ⟨"2026-10-26", .journals, .md, "g:B e:B"⟩
def GA : Graph := [JA, pg "o" "a:ML"]

/-- **W10-12 (historical transformations, not current offers).** Round 10
allowed the file-less plain rewrite as well as the owner merge. R10-Q4 now
offers only the latter through Rulings11.answer. This unchanged theorem retains
the counterexample to calling the first transformation "make a page": it
creates no file and writes stored e:ML instead of the owner's e:o. -/
theorem r13_alias_target :
    rename10 D (R9.at (nsSubst D.key D.urlHasOrigin "B" "ML")) GA "B" "ML" =
      some [⟨"2026-10-26", .journals, .md, "g:ML e:ML"⟩, pg "o" "a:ML"] ∧
    refMerge D R9 GA "B" "ML" =
      some [⟨"2026-10-26", .journals, .md, "g:ML e:o"⟩, pg "o" "a:ML"] ∧
    D.resolve [⟨"2026-10-26", .journals, .md, "g:ML e:o"⟩, pg "o" "a:ML"] "ML" = (.page, "o") := by
  decide +kernel

/-- **W10-13 (CM-OWN-CHILD).** `B → B/x` with a real `B/x`: refused both ways. -/
theorem r13_own_child :
    rename10 D R [J, pg "B/x" ""] "B" "B/x" = none ∧
    refMerge D R9 [J, pg "B/x" ""] "B" "B/x" = none := by decide +kernel

def GJR : Graph := [⟨"2026-10-26", .journals, .md, "a:B/sub"⟩, pg "p" "l:B/sub"]

/-- **W10-14 (CM3-1).** `B` is reference-only (the parent of `[[B/sub]]`); `B/sub` is a
journal's alias, so `B → C` would lose it: refused. -/
theorem r13_journal_risk :
    refSource D GJR "B" = true ∧
    Decided.journalRewriteRisks D GJR "B" "C" = ["B/sub"] ∧
    rename10 D R GJR "B" "C" = none := by decide +kernel

/-- **W10-15 (a parent-only source moves its real children).** `B` exists only as the
parent of the real page `B/c`; `B → n` moves `B/c` to `n/c` (OG `rename-namespace-pages!`). -/
theorem r13_parent_only :
    refSource D [J, pg "B/c" "x"] "B" = true ∧
    rename10 D R [J, pg "B/c" "x"] "B" "n" =
      some [⟨"2026-10-26", .journals, .md, "g:n"⟩, pg "n/c" "x"] := by decide +kernel

/-- The σ of the R13 witnesses: the rename map itself (key = id). -/
def mB (new : String) : String → String := nsSubst D.key D.urlHasOrigin "B" new

/-- **W10-18 (R10-15 not vacuous).** `MergeHyps` holds for the R13 merge `B → n/y` at the
survivor's query map, and R10-15 gives the journal block's reference to `n/y`. -/
theorem r13_merge_hyps :
    D.MergeHyps (R9.at (queryMap D "B" "n/y" (pg "n/y" "body"))).base (mB "n/y") (mB "n/y") GR13 where
  wf := by unfold WF; decide +kernel
  wf' := by unfold WF; decide +kernel
  laws := {
    dir := by decide +kernel
    format := by decide +kernel
    name := by decide +kernel
    path := by decide +kernel
    occ := by decide +kernel
    alias := by decide +kernel
    order := by decide +kernel }
  alias := by decide +kernel
  realInj := by decide +kernel

theorem r13_merge_references :
    D.References GR13' (((R9.at (queryMap D "B" "n/y" (pg "n/y" "body"))).base.apply (mB "n/y") J).path, 0)
      (σp (mB "n/y") (.page, "B")) ∧ σp (mB "n/y") (.page, "B") = (.page, "n/y") :=
  ⟨refMerge_references_forward (P := D) R9 (r13_merge.2.1) (by decide +kernel) r13_merge_hyps
    (f := J) (t := (.page, "B")) (by decide +kernel) 0 (by decide +kernel)
    ((D.referencesB_iff _ _ _).mp (by decide +kernel)), by decide +kernel⟩

/-- **W10-19 (R10-14 not vacuous).** `MergeHyps` holds for the reference-only rename `B → c`. -/
theorem r13_rename_hyps : D.MergeHyps R.base (mB "c") (mB "c") [J] where
  wf := by unfold WF; decide +kernel
  wf' := by unfold WF; decide +kernel
  laws := {
    dir := by decide +kernel
    format := by decide +kernel
    name := by decide +kernel
    path := by decide +kernel
    occ := by decide +kernel
    alias := by decide +kernel
    order := by decide +kernel }
  alias := by decide +kernel
  realInj := by decide +kernel

/-! ## 3b. Real pages with relative or URL names (Tine's rename still uses string prefixes) -/

def GDot : Graph := [pg "." "", pg "./x" "", pg "c" "l:./x"]
def GUrl : Graph := [pg "https:" "", pg "https://a.b/c" ""]

/-- **W10-20.** With real pages `.` and `./x`: renaming `.` moves nothing else (`./x` is no
child of `.`; OG `rename-namespace-pages!` selects by `:block/namespace`, never set for `./x`);
`.` → `./x` is not an own-child case, so the rename refuses only as a real target and the
merge is allowed. The same for a URL page under `https:`. -/
theorem relative_real_pages :
    Decided.rename D R GDot "." "z" = some [pg "z" "", pg "./x" "", pg "c" "l:./x"] ∧
    Decided.ownChildUsed D GDot "." "./x" = false ∧
    Decided.rename D R GDot "." "./x" = none ∧
    (merge D R JT GDot "." "./x").isSome = true ∧
    Decided.rename DU R GUrl "https:" "z" = some [pg "z" "", pg "https://a.b/c" ""] ∧
    Decided.ownChildUsed DU GUrl "https:" "https://a.b/c" = false := by decide +kernel

/-! ## 4. Alias spellings in the namespace tree (Q9 confirmed by CM11 R10-Q2) -/

/-- The fixture `r6-refused-journal-risk`. -/
def GA4 : Graph := [pg "old" "l:x", pg "p" "l:old/sub", ⟨"2026-10-26", .journals, .md, "a:old/sub"⟩]

/-- **W10-16.** `old/sub` is an alias of the journal: not a page (Q9), so no view lists
it; it resolves to the journal; its view parent would be `old`. OG instead lists `old/sub`
under `old` (an alias page entity with `:block/namespace` `old`). -/
theorem alias_spelling_not_a_node :
    Decided.pages D GA4 = [(.page, "old"), (.page, "p"), (.journal, "2026-10-26"), (.page, "x")] ∧
    D.resolve GA4 "old/sub" = (.journal, "2026-10-26") ∧
    nsParent D "old/sub" = some "old" := by decide +kernel

/-- **W10-17.** An alias `old/sub` alone (with OG's pre-block property reference, here an
explicit `l:`): its parent `old` is a reference-only page, as in OG; `old/sub` is not. -/
theorem alias_spelling_parent :
    Decided.pages D [⟨"2026-10-26", .journals, .md, "a:old/sub l:old/sub"⟩] =
      [(.journal, "2026-10-26"), (.page, "old")] := by decide +kernel

end Witness10
end CM
