/- Round 8 witnesses (D-20261102-MC, D-20261102-MC5). Names: `src` plays
`ML notes`, `tgt` plays `ML`, `owner` plays `Machine Learning` (it declares
`alias:: ML`; no file claims `ML`). The toy parser splits tokens at spaces, so
names with spaces are not used. -/
import Merge8
import MergeWitness7b

set_option maxRecDepth 8192
set_option maxHeartbeats 1600000
namespace CM
namespace MergeWitness8
open Params Merge7 Merge7b Merge8 MergeWitness7
open Witnesses2 (pg)

abbrev Q : QParams String String := QueryWitness6.Q
abbrev SP : QueryRename6.StoredParser String := QueryWitness6.S

/-! ## M-Q5: rename `src` onto `tgt`, an alias-only name of `owner` -/

def S2 : File := pg "src" "l:y"
def O2 : File := pg "owner" "a:tgt"

/-- **MW15 (both outcomes are available; Tine asks).** On `G2` (round 7's
alias-owner graph) the decided merge resolves `tgt` to its owner and accepts,
and the decided rename (rule 6, unchanged) also accepts. -/
theorem mw_alias_both : mergeTarget D G2 "src" "tgt" = some O2 ∧
    mergeAllowed D G2 "src" "tgt" = true ∧ Decided.renameAllowed D G2 "src" "tgt" = true := by
  decide +kernel

/-! ### (a) merge into the owner -/

/-- The merge's page-level key map: the source key goes to the owner; every
other key as the rename map `mm` (`nsSubst "src" "tgt"`). -/
def τ2 (k : String) : String := if k = "src" then "owner" else mm k

theorem mw_via : ViaAlias D mm τ2 S2 O2 G2 where
  agree := by
    intro k hk
    have hS : (D.claim S2).2 = "src" := by decide +kernel
    rw [hS] at hk
    unfold τ2; rw [if_neg hk]
  declares := by decide +kernel
  fixed := by decide +kernel

/-- **MW16 (hypotheses of the alias-owner merge).** `G2`'s merge into `owner`
meets `MergeBodyHyps` with text map `mm` and page map `τ2`: the merge
hypotheses (now with `τ2 src = owner`, so `srcKey` holds), the body law, the
stored-query law and the rename laws. -/
theorem mw_alias_hyps : MergeBodyHyps D Q SP R.base JT mm mm τ2 S2 O2 G2 where
  lawful := Policy5.lawful _ _ Witnesses3.lawful
  wf := by unfold WF; decide +kernel
  wf' := by unfold WF; decide +kernel
  src := by decide +kernel
  dst := by decide +kernel
  ne := by decide +kernel
  srcPage := by decide +kernel
  srcKey := by decide +kernel
  dstKey := by decide +kernel
  srcUniq := by decide +kernel
  dstUniq := by decide +kernel
  aliasPage := by decide +kernel
  dstAlias := by decide +kernel
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
  merger := {
    path := by decide +kernel
    dir := by decide +kernel
    format := by decide +kernel
    name := by decide +kernel
    aliasL := by decide +kernel
    nonempty := by decide +kernel
    valid := by decide +kernel
    occSubL := by decide +kernel
    coverS := by
      intro j hj
      have h1 : (D.outline S2.format S2.text).length = 1 := by decide +kernel
      rw [h1] at hj
      match j, hj with
      | 0, _ => decide +kernel
    coverD := by
      intro j hj
      have h1 : (D.outline O2.format O2.text).length = 1 := by decide +kernel
      rw [h1] at hj
      match j, hj with
      | 0, _ => decide +kernel
    namesL := by decide +kernel }
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

/-- **MW17 (C11 on the example; round 8b: hypotheses now split, see
`MergeWitness8b` for the graphs where an alias spells the source).** The referrer `c`'s link to `src`
has an image that references `owner` (MA2); `tgt` stays in `owner`'s class
(MA4: `cls` after-graph `owner` `tgt`). -/
theorem mw_alias_refs : ∃ b' ∈ D.blocks (mergeBy R.base JT mm S2 O2 G2),
    Img R.base JT mm S2 O2 G2 ("c", 0) b' ∧
      D.References (mergeBy R.base JT mm S2 O2 G2) b' (D.claim O2) :=
  references_to_source_via mw_alias_hyps.toMergeBase mw_via (by decide +kernel)
    (by unfold NsForward; decide +kernel) (D.referencesB_iff _ _ _ |>.mp (by decide +kernel))

theorem mw_alias_class : cls (D.aliasEdges (mergeBy R.base JT mm S2 O2 G2)) (D.claim O2).2 "tgt" :=
  class_join_via (x := "src") mw_alias_hyps.toMergeBase mw_via (Or.inl (cls_self _ _))

/-- **MW18 (boundary of (a): page-name query leaves are key-exact).** A stored
`(page src)` query matched the source's block; the merge rewrites it to
`(page tgt)`, which matches nothing: `tgt` names no page any more (its blocks
are `owner`'s), and page-name leaves compare keys, not alias classes. The
`refs` leaves (and Linked References) read through classes and keep their
matches (MW17). Not covered by MA1–MA4, which are about references.
Round 9 (ruling (a), D-20261103-CMA): this is the round-8 operation, kept as
history; the decided round-9 merge `Rulings9.merge9` writes `(page owner)`
(`Witness9.ml_merge`). -/
def G2q : Graph := [pg "src" "l:y", pg "owner" "a:tgt", pg "c" "e:src"]
theorem mw_alias_page_leaf :
    merge D R JT G2q "src" "tgt" = some [pg "owner" "a:tgt;l:y", pg "c" "e:tgt"] ∧
    QueryRename6.storedAt D SP G2q ("c", 0) = [.leaf (.page (.name "src"))] ∧
    Decided.den D Q G2q .block (.leaf (.page (.name "src"))) = [.block ("src", 0)] ∧
    QueryRename6.storedAt D SP [pg "owner" "a:tgt;l:y", pg "c" "e:tgt"] ("c", 0) =
      [.leaf (.page (.name "tgt"))] ∧
    Decided.den D Q [pg "owner" "a:tgt;l:y", pg "c" "e:tgt"] .block (.leaf (.page (.name "tgt"))) = [] ∧
    Decided.den D Q [pg "owner" "a:tgt;l:y", pg "c" "e:tgt"] .block (.leaf (.page (.name "owner"))) =
      [.block ("owner", 0), .block ("owner", 1)] := by decide +kernel

/-! ### (b) make `tgt` its own page, still an alias of `owner` (the decided rename) -/

def G5 : Graph := [pg "src" "l:y", pg "owner" "a:tgt", pg "c" "l:src;l:tgt"]
def G5After : Graph := [pg "tgt" "l:y", pg "owner" "a:tgt", pg "c" "l:tgt;l:tgt"]

/-- **MW19 (rename onto an alias-only name: allowed, as specified).** The
unchanged rule-6 guard accepts; the source becomes the page `tgt` with its own
file; `tgt` now opens that file (AL1; before, it resolved to `owner`); `tgt`
is still declared an alias of `owner`; both of `c`'s blocks (one linked `src`,
one linked `tgt`) reference `tgt` and `owner` (AL2: one class). The `[[tgt]]`
link (`c` 1) counted toward `owner` before and still does; only where it opens
changes (D-20261102-MC5). -/
theorem mw_rename_alias :
    Decided.renameAllowed D G5 "src" "tgt" = true ∧
    Decided.rename D R G5 "src" "tgt" = some G5After ∧
    renameBy R.base mm G5 = G5After ∧
    D.resolve G5 "tgt" = (.page, "owner") ∧ D.resolve G5After "tgt" = (.page, "tgt") ∧
    D.aliasEdges G5After = [("owner", "tgt")] ∧
    Decided.pages D G5After = [(.page, "tgt"), (.page, "owner"), (.page, "c"), (.page, "y")] ∧
    (D.blocks G5).filter (fun b => D.referencesB G5 b (.page, "owner")) = [("c", 1)] ∧
    (D.blocks G5After).filter (fun b => D.referencesB G5After b (.page, "owner")) = [("c", 0), ("c", 1)] ∧
    (D.blocks G5After).filter (fun b => D.referencesB G5After b (.page, "tgt")) = [("c", 0), ("c", 1)] := by
  decide +kernel

theorem mw_rename_hyps : D.MergeHyps R.base mm mm G5 where
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

/-- **MW20 (MB1 on the example).** -/
theorem mw_rename_refs :
    D.References (renameBy R.base mm G5) ((R.base.apply mm (pg "c" "l:src;l:tgt")).path, 0)
        (.page, mm (D.claim S2).2) ∧
    D.References (renameBy R.base mm G5) ((R.base.apply mm (pg "c" "l:src;l:tgt")).path, 0) (D.claim O2) :=
  (rename_onto_alias mw_rename_hyps (S := S2) (D := O2) (by decide +kernel) (by decide +kernel)
    (by decide +kernel) (by decide +kernel) (by decide +kernel) (by decide +kernel)).2.2.2 _
    (by decide +kernel) 0 (by decide +kernel)
    (Or.inl (D.referencesB_iff _ _ _ |>.mp (by decide +kernel)))

/-! ## M-Q6: merging a page into its own namespace child is refused -/

def GFoo : Graph := [pg "Foo" "l:z", pg "Foo/bar" "", pg "c" "l:Foo"]
def Gal : Graph := [pg "a" "", pg "o" "a:a/x"]

/-- **MW21 (refusals).** `Foo` into `Foo/bar`: the target resolves, the merge
is refused. The typed-name form: `a/x` is an alias of `o` (not under `a`), but
the typed name is under `a`; the merge's rename map would rewrite `o`'s
`alias:: a/x` to `a/x/x`, so it is refused too. -/
theorem mw_subtree_refused :
    mergeTarget D GFoo "Foo" "Foo/bar" = some (pg "Foo/bar" "") ∧
    mergeAllowed D GFoo "Foo" "Foo/bar" = false ∧ merge D R JT GFoo "Foo" "Foo/bar" = none ∧
    mergeTarget D Gal "a" "a/x" = some (pg "o" "a:a/x") ∧
    (nsMatch D.key D.urlHasOrigin "a" "o").isSome = false ∧ nsSubst D.key D.urlHasOrigin "a" "a/x" "a/x" = "a/x/x" ∧
    mergeAllowed D Gal "a" "a/x" = false := by decide +kernel

/-- **MW21b (round 8 observation; superseded in round 9).** The round-8 RENAME
guard (`renameAllowedR8`) accepted `Foo` → `Foo/bar` on the same graph: the
existing `Foo/bar` is inside the moved subtree, so the unguarded operation moves
it to `Foo/bar/bar`, and `Foo`'s file becomes `Foo/bar`. Round 9 (CM-OWN-CHILD,
D-20261103-CMA) refuses this rename: `Foo/bar` is a real page below `Foo`.
Restated (ruling-driven): round 8's `renameAllowed = true` and
`rename = some …` are now facts about `renameAllowedR8` and `Params.rename`. -/
theorem mw_subtree_rename :
    Decided.renameAllowedR8 D GFoo "Foo" "Foo/bar" = true ∧
    D.rename R.base GFoo "Foo" "Foo/bar" =
      [pg "Foo/bar" "l:z", pg "Foo/bar/bar" "", pg "c" "l:Foo/bar"] ∧
    Decided.ownChildUsed D GFoo "Foo" "Foo/bar" = true ∧
    Decided.renameAllowed D GFoo "Foo" "Foo/bar" = false ∧
    Decided.rename D R GFoo "Foo" "Foo/bar" = none := by decide +kernel

/-! ## M-Q7: merging a page into a journal -/

open MergeWitness7b (Gj GjAfter)
def Sj : File := pg "src" "l:z"
def Dj : File := ⟨"2026_11_02", .journals, .md, "l:w"⟩
def mmj : String → String := nsSubst D.key D.urlHasOrigin "src" "2026_11_02"

/-- **MW22 (hypotheses of the journal merge).** `Gj`'s merge (page `src` into
the journal `2026_11_02`) meets `MergeBodyHyps` (survivor of either kind). -/
theorem mw_journal_hyps : MergeBodyHyps D Q SP R.base JT mmj mmj mmj Sj Dj Gj where
  lawful := Policy5.lawful _ _ Witnesses3.lawful
  wf := by unfold WF; decide +kernel
  wf' := by unfold WF; decide +kernel
  src := by decide +kernel
  dst := by decide +kernel
  ne := by decide +kernel
  srcPage := by decide +kernel
  srcKey := by decide +kernel
  dstKey := by decide +kernel
  srcUniq := by decide +kernel
  dstUniq := by decide +kernel
  aliasPage := by decide +kernel
  dstAlias := by decide +kernel
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
  merger := {
    path := by decide +kernel
    dir := by decide +kernel
    format := by decide +kernel
    name := by decide +kernel
    aliasL := by decide +kernel
    nonempty := by decide +kernel
    valid := by decide +kernel
    occSubL := by decide +kernel
    coverS := by
      intro j hj
      have h1 : (D.outline Sj.format Sj.text).length = 1 := by decide +kernel
      rw [h1] at hj
      match j, hj with
      | 0, _ => decide +kernel
    coverD := by
      intro j hj
      have h1 : (D.outline Dj.format Dj.text).length = 1 := by decide +kernel
      rw [h1] at hj
      match j, hj with
      | 0, _ => decide +kernel
    namesL := by decide +kernel }
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

/-- **MW23 (MJ3 on the example).** `c`'s link to `src` has an image that
references the journal, which is a real page afterwards; the after-graph is
`GjAfter` (the source's block appended to the journal). -/
theorem mw_journal_refs :
    mergeBy R.base JT mmj Sj Dj Gj = GjAfter ∧
    ∃ b' ∈ D.blocks (mergeBy R.base JT mmj Sj Dj Gj), Img R.base JT mmj Sj Dj Gj ("c", 0) b' ∧
      D.References (mergeBy R.base JT mmj Sj Dj Gj) b' (.journal, (D.claim Dj).2) ∧
      D.real (mergeBy R.base JT mmj Sj Dj Gj) (.journal, (D.claim Dj).2) = true :=
  ⟨by decide +kernel, journal_references mw_journal_hyps.toMergeBase (by decide +kernel)
    (by decide +kernel) (by unfold NsForward; decide +kernel)
    (D.referencesB_iff _ _ _ |>.mp (by decide +kernel))⟩

end MergeWitness8
end CM
