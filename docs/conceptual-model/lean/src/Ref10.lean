/- Round 10 (OG parity): renaming or merging a REFERENCE-ONLY name (divergence R13),
and the one namespace test asked by the views (divergence S15).

OG (0.10.9 `c67b8b5fa`, identical to `6e7afa8eb` for every file cited):
`src/main/frontend/handler/page.cljs:618 rename!` renames any page entity, with or without a
file (the page title is editable on every page, `components/page.cljs:242–290`). When `new`
names an existing page entity (`:block/name`, :637; `db/model.cljs:869 page-exists?` is entity existence) it calls `merge-pages!` (:568), which moves
the source's blocks (a reference-only page has none), rewrites every referencing block
(`rename-update-refs!` :393) and deletes the source; otherwise `rename-namespace-pages!` (:550)
renames the page and its namespace children and rewrites their references.

The model keeps its decided split: a rename onto a real page is refused and offered as a
merge (§5.5); onto an unused, reference-only or alias-only name it is a rename. Round 9's
`rename` needs a real source (`renameCoreAllowed`'s first conjunct) and `merge` needs a source
FILE (`mergeSource`), so both refuse a reference-only source (`rename_refuses_refSource`,
`merge_refuses_refSource`). Round 10 adds the reference-only source to both, with every other
round-9 clause unchanged: `rename10` (the round-9 guard minus the real-source conjunct) and
`refMerge` (round 8/9's merge guard minus the clauses about the source file, after-graph:
round 9's rename rewriter at ruling (a)'s query map; there is no source file to delete and no
block to append). -/
import Witness9

set_option linter.unusedSectionVars false

namespace CM
namespace Ref10
open Params Merge7 Rulings9

variable {K : Type} [DecidableEq K] (P : Params K)

/-! ## 1. The source -/

/-- `old` names a reference-only page of the decided universe: a page-kind entry of
`refOnlyPagesR true` (referenced or a name-derived parent; no file of either kind claims it;
no enabled owner declares it). -/
def refSource (G : Graph) (old : String) : Bool :=
  (P.refOnlyPagesR true G).contains (.page, P.key old)

theorem refSource_not_real {G : Graph} {old : String} (h : refSource P G old = true) :
    P.claimedKey G (P.key old) = false := by
  unfold refSource refOnlyPagesR at h
  simp only [List.contains_iff_mem, List.mem_map, List.mem_filter, Bool.and_eq_true,
    Bool.not_eq_true', Prod.mk.injEq] at h
  obtain ⟨k, ⟨_, hc, _⟩, _, rfl⟩ := h
  exact hc

theorem refSource_not_page_real {G : Graph} {old : String} (h : refSource P G old = true) :
    P.real G (.page, P.key old) = false := by
  have hc := refSource_not_real P h
  unfold claimedKey at hc
  simp only [Bool.or_eq_false_iff] at hc
  exact hc.2

theorem refSource_mem_pages {G : Graph} {old : String} (h : refSource P G old = true) :
    ((.page, P.key old) : Page K) ∈ Decided.pages P G := by
  unfold refSource at h
  unfold Decided.pages pagesR
  exact List.mem_append_right _ (List.contains_iff_mem.mp h)

/-- **R10-1 (R13, the round-9 refusal).** Round 9's decided rename refuses every
reference-only source: its guard requires a real page-kind source. -/
theorem rename_refuses_refSource (R : Decided.Rewriter) {G : Graph} {old new : String}
    (h : refSource P G old = true) : Decided.rename P R G old new = none := by
  unfold Decided.rename Decided.renameAllowed Decided.renameBaseAllowed Decided.renameCoreAllowed
  simp [refSource_not_page_real P h]

theorem mergeSource_none {G : Graph} {old : String} (h : refSource P G old = true) :
    mergeSource P G old = none := by
  have hr := refSource_not_page_real P h
  unfold mergeSource
  split
  · next f hc =>
    exfalso
    have hf : f ∈ P.claimants G (.page, P.key old) := by rw [hc]; exact List.mem_singleton_self _
    have := (P.mem_claimants).mp hf
    have : P.real G (.page, P.key old) = true := by
      unfold real; exact List.any_eq_true.mpr ⟨f, this.1, by simp [this.2]⟩
    rw [hr] at this; cases this
  · rfl

/-- **R10-2 (R13, the round-9 refusal).** Round 8's and round 9's merges refuse every
reference-only source: there is no source file. -/
theorem merge_refuses_refSource (R : Decided.Rewriter) (J : Merger) (RQ : QRewriter)
    (JQ : (String → String) → Merger) {G : Graph} {old new : String}
    (h : refSource P G old = true) :
    merge P R J G old new = none ∧ merge9 P RQ JQ G old new = none := by
  have hs := mergeSource_none P h
  refine ⟨by unfold merge; rw [hs], ?_⟩
  have e := merge9_isSome P RQ JQ R J G old new
  have : merge P R J G old new = none := by unfold merge; rw [hs]
  rw [this] at e
  simpa using e

/-! ## 2. Rename of a reference-only source -/

/-- `renameCoreAllowed` without its real-source conjunct (clauses verbatim). -/
def renameCoreFree (G : Graph) (old new : String) : Bool :=
  G.all (fun g => (decide (kindOf g = .page) && (nsMatch P.key P.urlHasOrigin old (P.name g)).isSome) ||
    (P.movedFiles G old).all (fun f =>
      decide ((P.claim g).2 ≠ P.key (nsSubst P.key P.urlHasOrigin old new (P.name f))))) &&
  !Decided.classClash P G old new

/-- `renameBaseAllowed` with `renameCoreFree` (clauses verbatim). -/
def renameBaseFree (G : Graph) (old new : String) : Bool :=
  renameCoreFree P G old new &&
  (Decided.movedNames P G old).all (fun s =>
    !P.conflictedB G (P.key s) &&
    !P.conflictedB G (P.key (nsSubst P.key P.urlHasOrigin old new s))) &&
  !P.conflictedB G (P.key new) &&
  (P.movedFiles G old).all (fun f => (P.movedFiles G old).all (fun g =>
    decide (P.key (nsSubst P.key P.urlHasOrigin old new (P.name f)) =
      P.key (nsSubst P.key P.urlHasOrigin old new (P.name g)) → (P.claim f).2 = (P.claim g).2))) &&
  G.all (fun g => decide (g ∈ P.movedFiles G old) ||
    (Decided.movedPageNames P G old).all (fun s =>
      decide ((P.claim g).2 ≠ P.key (nsSubst P.key P.urlHasOrigin old new s)) ||
        Decided.keepsKey P old new s))

/-- Round 9's whole rename guard without the real-source conjunct: the moved-subtree,
real-target, class, collision and conflict clauses, the journal-reference-loss refusal
(CM3-1) and the own-child refusal (CM-OWN-CHILD). -/
def renameAllowedFree (G : Graph) (old new : String) : Bool :=
  renameBaseFree P G old new && (Decided.journalRewriteRisks P G old new).isEmpty &&
    !Decided.ownChildUsed P G old new

/-- **R10-3.** Round 9's guard is exactly "the source is real" and the free guard: no
other clause is changed or added. -/
theorem renameAllowed_eq (G : Graph) (old new : String) :
    Decided.renameAllowed P G old new =
      (P.real G (.page, P.key old) && renameAllowedFree P G old new) := by
  unfold Decided.renameAllowed Decided.renameBaseAllowed Decided.renameCoreAllowed
    renameAllowedFree renameBaseFree renameCoreFree
  cases P.real G (.page, P.key old) <;> simp [Bool.and_assoc]

/-- **The round-10 rename.** A real source: round 9's `rename`, unchanged. A
reference-only source: the same guard without the real-source conjunct and the same
after-graph (`Params.rename`, every file rewritten by `nsSubst old new`). Any other source
(unused, alias-only, journal) is refused as before. -/
def rename10 (R : Decided.Rewriter) (G : Graph) (old new : String) : Option Graph :=
  if refSource P G old then
    (if renameAllowedFree P G old new then some (P.rename R.base G old new) else none)
  else Decided.rename P R G old new

/-- **R10-4.** For a source that is not reference-only, `rename10` is round 9's rename. -/
theorem rename10_eq_rename (R : Decided.Rewriter) {G : Graph} {old new : String}
    (h : refSource P G old = false) : rename10 P R G old new = Decided.rename P R G old new := by
  simp [rename10, h]

/-- **R10-5.** For a reference-only source, `rename10` accepts exactly under the free
guard, with round 9's after-graph. -/
theorem rename10_ref (R : Decided.Rewriter) {G : Graph} {old new : String}
    (h : refSource P G old = true) :
    rename10 P R G old new =
      if renameAllowedFree P G old new then some (P.rename R.base G old new) else none := by
  simp [rename10, h]

/-- **R10-6 (one guard).** Whenever `rename10` accepts, round 9's guard holds except
possibly its real-source conjunct; every refusal of round 9 other than "the source is not
real" stays a refusal. -/
theorem rename10_some_free (R : Decided.Rewriter) {G G' : Graph} {old new : String}
    (h : rename10 P R G old new = some G') :
    renameAllowedFree P G old new = true ∧ G' = P.rename R.base G old new := by
  unfold rename10 at h
  split at h
  · split at h
    · next ha => exact ⟨ha, (Option.some.inj h).symm⟩
    · cases h
  · unfold Decided.rename at h
    split at h
    · next ha =>
      rw [renameAllowed_eq] at ha
      simp only [Bool.and_eq_true] at ha
      exact ⟨ha.2, (Option.some.inj h).symm⟩
    · cases h

/-- **R10-7 (CM-OWN-CHILD for a reference-only source).** -/
theorem rename10_own_child (R : Decided.Rewriter) {G : Graph} {old new : String}
    (h : Decided.ownChildUsed P G old new = true) : rename10 P R G old new = none := by
  cases e : rename10 P R G old new with
  | none => rfl
  | some G' =>
    have := (rename10_some_free P R e).1
    unfold renameAllowedFree at this
    simp [h] at this

/-- **R10-8 (CM3-1 for a reference-only source).** -/
theorem rename10_journal_risk (R : Decided.Rewriter) {G : Graph} {old new : String}
    (h : Decided.JournalRisk P G old new) : rename10 P R G old new = none := by
  cases e : rename10 P R G old new with
  | none => rfl
  | some G' =>
    have := (rename10_some_free P R e).1
    have hr := (Decided.journalRewriteRisks_nonempty P).mpr h
    unfold renameAllowedFree at this
    simp only [Bool.and_eq_true] at this
    rw [hr] at this
    cases this.1.2

/-- **R10-9 (a real target is a merge, not a rename).** A reference-only source renamed
onto a different key that a file outside the moved subtree claims is refused, as for a
real source (§5.5): the operation for that case is `refMerge`. `hself` says the
substitution sends `old` itself to `new`'s key (`nsSubst_self` for a non-namespace `old`;
for a namespaced one it holds unless an earlier prefix of `old` has `old`'s key). -/
theorem rename10_real_target (R : Decided.Rewriter) {G : Graph} {old new : String} {g : File}
    (h : refSource P G old = true) (hg : g ∈ G) (hgk : (P.claim g).2 = P.key new)
    (hgm : g ∉ P.movedFiles G old) (hne : P.key new ≠ P.key old)
    (hself : P.key (nsSubst P.key P.urlHasOrigin old new old) = P.key new) :
    rename10 P R G old new = none := by
  cases e : rename10 P R G old new with
  | none => rfl
  | some G' =>
    have hf := (rename10_some_free P R e).1
    unfold renameAllowedFree renameBaseFree at hf
    simp only [Bool.and_eq_true] at hf
    have hall := List.all_eq_true.mp hf.1.1.2 g hg
    simp only [Bool.or_eq_true, decide_eq_true_eq, hgm, false_or] at hall
    have hmp : old ∈ Decided.movedPageNames P G old := by
      unfold Decided.movedPageNames
      refine List.mem_filter.mpr ⟨CaseRename7.old_mem_movedNames P, ?_⟩
      exact List.any_eq_true.mpr ⟨_, refSource_mem_pages P h, by simp⟩
    have := List.all_eq_true.mp hall old hmp
    simp only [Bool.or_eq_true, decide_eq_true_eq, Decided.keepsKey, Bool.and_eq_true] at this
    rcases this with h1 | ⟨h2, _⟩
    · exact absurd (hgk.trans hself.symm) h1
    · exact absurd h2 hne

/-- A spelling that is not a namespace name is replaced whole by its own rename. -/
theorem nsSubst_self_flat {old new : String} (h : isNsName P.urlHasOrigin old = false) :
    nsSubst P.key P.urlHasOrigin old new old = new := by
  unfold nsSubst nsMatch
  simp [h]

/-! ## 3. Merge of a reference-only source -/

/-- Round 8/9's `mergeAllowed`, for a source with no file: the clauses about the source file
(source ≠ survivor, equal formats, the source's own destination) are void; every other
clause is verbatim: survivor and typed name outside the source's subtree (M-Q6 /
CM-OWN-CHILD), descendants land on no real page outside the subtree but the survivor, moved
keys not Conflicted, no collision of distinct moved real keys, no other real target, no
journal-reference loss (CM3-1). -/
def noFileMergeCore (G : Graph) (old new : String) (D : File) : Bool :=
  !(nsMatch P.key P.urlHasOrigin old (P.name D)).isSome &&
  !(nsMatch P.key P.urlHasOrigin old new).isSome &&
  G.all (fun g => decide (g = D) ||
    (decide (kindOf g = .page) && (nsMatch P.key P.urlHasOrigin old (P.name g)).isSome) ||
    (P.movedFiles G old).all (fun f =>
      decide ((P.claim g).2 ≠ P.key (nsSubst P.key P.urlHasOrigin old new (P.name f))))) &&
  (Decided.movedNames P G old).all (fun s => !P.conflictedB G (P.key s)) &&
  (P.movedFiles G old).all (fun f => (P.movedFiles G old).all (fun g =>
    decide (P.key (nsSubst P.key P.urlHasOrigin old new (P.name f)) =
      P.key (nsSubst P.key P.urlHasOrigin old new (P.name g)) → (P.claim f).2 = (P.claim g).2))) &&
  G.all (fun g => decide (g = D) || decide (g ∈ P.movedFiles G old) ||
    (Decided.movedPageNames P G old).all (fun s => decide (P.key s = P.key old) ||
      decide ((P.claim g).2 ≠ P.key (nsSubst P.key P.urlHasOrigin old new s)))) &&
  (Decided.journalRewriteRisks P G old new).isEmpty

/-- The original reference-only guard, factored without changing a clause.
Round 12 uses the identical core for an alias-only source. -/
def refMergeAllowedTo (G : Graph) (old new : String) (D : File) : Bool :=
  refSource P G old && noFileMergeCore P G old new D

/-- Round 10 target selection, with the shared guard exposed for round 11. -/
def refMergeAllowed (G : Graph) (old new : String) : Bool :=
  match mergeTarget P G old new with
  | some D => refMergeAllowedTo P G old new D
  | none => false

/-- **The merge of a reference-only source** into `mergeTarget` (a file claiming `new`, or
the owner declaring `new` as an alias: M-Q5 answer one). The after-graph is the rename
rewrite of every file under `nsSubst old new`, with stored page-name queries naming the
source rewritten to the survivor's own name (ruling (a), `queryMap`) and children following
the typed name (ruling (b)). The survivor is the target file, rewritten like every file. -/
def refMerge (R : QRewriter) (G : Graph) (old new : String) : Option Graph :=
  match mergeTarget P G old new with
  | some D => if refMergeAllowed P G old new then
      some (renameBy (R.at (queryMap P old new D)).base (nsSubst P.key P.urlHasOrigin old new) G)
    else none
  | none => none

/-- **R10-10 (structure).** -/
theorem refMerge_structure (R : QRewriter) {G G' : Graph} {old new : String}
    (h : refMerge P R G old new = some G') :
    ∃ D, mergeTarget P G old new = some D ∧ refSource P G old = true ∧
      G' = renameBy (R.at (queryMap P old new D)).base (nsSubst P.key P.urlHasOrigin old new) G := by
  unfold refMerge at h
  cases hD : mergeTarget P G old new with
  | none => rw [hD] at h; cases h
  | some D =>
    rw [hD] at h
    simp only at h
    split at h
    · next ha =>
      unfold refMergeAllowed at ha
      rw [hD] at ha
      unfold refMergeAllowedTo at ha
      simp only [Bool.and_eq_true] at ha
      exact ⟨D, rfl, ha.1, (Option.some.inj h).symm⟩
    · cases h

/-- **R10-11 (disjoint from round 9).** `refMerge` accepts only reference-only sources,
for which round 9's merge refuses (R10-2); for a file source it refuses, and `merge9`
decides. -/
theorem refMerge_none_of_not_ref (R : QRewriter) {G : Graph} {old new : String}
    (h : refSource P G old = false) : refMerge P R G old new = none := by
  cases e : refMerge P R G old new with
  | none => rfl
  | some G' =>
    obtain ⟨_, _, hs, _⟩ := refMerge_structure P R e
    rw [h] at hs; cases hs

/-- **R10-12 (CM-OWN-CHILD / M-Q6 for a reference-only source).** -/
theorem refMerge_own_child (R : QRewriter) {G : Graph} {old new : String}
    (h : (nsMatch P.key P.urlHasOrigin old new).isSome = true) : refMerge P R G old new = none := by
  unfold refMerge
  cases hD : mergeTarget P G old new with
  | none => rfl
  | some D =>
    simp only
    have : refMergeAllowed P G old new = false := by
      unfold refMergeAllowed; rw [hD]; simp [refMergeAllowedTo, noFileMergeCore, h]
    simp [this]

/-- **R10-13 (CM3-1 for a reference-only source).** -/
theorem refMerge_journal_risk (R : QRewriter) {G : Graph} {old new : String}
    (h : Decided.JournalRisk P G old new) : refMerge P R G old new = none := by
  have hr := (Decided.journalRewriteRisks_nonempty P).mpr h
  unfold refMerge
  cases hD : mergeTarget P G old new with
  | none => rfl
  | some D =>
    simp only
    have : refMergeAllowed P G old new = false := by
      unfold refMergeAllowed; rw [hD]; simp [refMergeAllowedTo, noFileMergeCore, hr]
    simp [this]

/-! ## 4. Where the references go afterwards -/

variable {P}

/-- **R10-14 (references after a reference-only rename, exact).** Under `MergeHyps` (the
rename laws, alias compatibility and injectivity on REAL claim keys only, so the source's
key may join the target's: OG `merge-pages!` of a page with no blocks), a block of the
after-graph references `t` iff one of its old occurrences, rewritten, has a closure name in
`t`'s class of the old alias graph mapped by σ. -/
theorem rename10_references (R : Decided.Rewriter) {G G' : Graph} {old new : String}
    {σ : K → K} (h : rename10 P R G old new = some G')
    (H : P.MergeHyps R.base (nsSubst P.key P.urlHasOrigin old new) σ G)
    {f : File} (hf : f ∈ G) (i : Nat) (t : Page K) :
    P.References G' ((R.base.apply (nsSubst P.key P.urlHasOrigin old new) f).path, i) t ↔
      ∃ o ∈ P.occAt G (f.path, i),
        ∃ u ∈ P.occClosure (rwOcc R.base.covers (nsSubst P.key P.urlHasOrigin old new) o).2,
          cls (Reach.mapE σ (P.aliasEdges G)) (P.key u) t.2 := by
  rw [(rename10_some_free P R h).2]
  exact references_merge_exact H hf i t

/-- **R10-15 (references after a reference-only merge, forward).** Every old reference of a
block survives, moved by σ, in its rewritten block, given the forward closure condition
(`NsForward`, as for every merge: Merge7/Rulings9). -/
theorem refMerge_references_forward (R : QRewriter) {G G' : Graph} {old new : String}
    {σ : K → K} {D : File} (h : refMerge P R G old new = some G')
    (hD : mergeTarget P G old new = some D)
    (H : P.MergeHyps (R.at (queryMap P old new D)).base (nsSubst P.key P.urlHasOrigin old new) σ G)
    {f : File} (hf : f ∈ G) (i : Nat) {t : Page K}
    (hns : ∀ o ∈ P.occAt G (f.path, i), ∀ u ∈ P.occClosure o.2,
      ∃ v ∈ P.occClosure (rwOcc (R.at (queryMap P old new D)).base.covers
        (nsSubst P.key P.urlHasOrigin old new) o).2, P.key v = σ (P.key u))
    (hr : P.References G (f.path, i) t) :
    P.References G' (((R.at (queryMap P old new D)).base.apply
      (nsSubst P.key P.urlHasOrigin old new) f).path, i) (σp σ t) := by
  obtain ⟨D', hD', _, rfl⟩ := refMerge_structure P R h
  rw [hD] at hD'; cases hD'
  exact references_merge_forward H hf i hns hr

/-- **R10-16 (pages after either operation, exact).** The after-graph's page universe is
given by `pages_merge_exact` (MergeRename): real pages are σ-images of real pages, and
reference-only pages are the rewritten seeds that no file claims and no owner declares. -/
theorem rename10_after (R : Decided.Rewriter) {G G' : Graph} {old new : String}
    (h : rename10 P R G old new = some G') :
    G' = renameBy R.base (nsSubst P.key P.urlHasOrigin old new) G :=
  (rename10_some_free P R h).2

/-! ## 5. The namespace test of the views (S15) -/

variable (P)

/-- The namespace parent of a name, as OG's views place it (`:block/namespace`, set only
for a `namespace-page?` name: `block.cljs:304`, `handler/page.cljs:428`; the tree:
`db/model.cljs:1545 get-namespace-pages`, `:1572 get-namespace-hierarchy`;
`components/hierarchy.cljs:13`). The same test as references and file names. -/
def nsParent (s : String) : Option String :=
  if P.nameIsNamespace s then (P.occClosure s).tail.getLast? else none

/-- **R10-17.** A name that is not a namespace name has no parent in any view, and is
its own whole occurrence closure (no parent page from references). -/
theorem flat_name (s : String) (h : P.nameIsNamespace s = false) :
    nsParent P s = none ∧ P.occClosure s = [s] := by
  simp [nsParent, occClosure, h]

/-- **R10-18.** The view's parent of a referenced name is in its occurrence closure, so a
referenced child's parent is a page of the universe unless a file claims or an owner
declares it (`Universe.mem_refOnly…`): views, references and page existence ask one test. -/
theorem nsParent_mem (s p : String) (h : nsParent P s = some p) : p ∈ P.occClosure s := by
  unfold nsParent at h
  split at h
  · exact List.mem_of_mem_tail (List.mem_of_getLast? h)
  · cases h

end Ref10
end CM
