/- Round 9: Martin's rulings of 2026-11-03 (D-20261103-CMA).

1. CM-NAMES (2). A journal file's page name is its stem (its date), even when
   the file has a `title::` property; the title is then an ordinary property
   and names nothing (`Model.name`). The model does not decode file names:
   `Params.stem` is a given, so CM-NAMES (1) (file-name escapes) is outside it.
2. EMPTY-ALIAS. A content-free file (OG `page-empty-or-dummy?`; `Model.contentFree`)
   whose key some owner declares as an alias does not win navigation: `[[ML]]`
   opens the declaring owner (`Model.resolveKey`). With content the file wins
   (AL1). Only navigation changes: the empty file is still a real, listed page,
   and Linked References, counts and queries read alias classes, which never
   consulted `resolve` (`Universe.references_iff_keys`).
3. CM-OWN-CHILD. Renaming or merging `old` onto a strict namespace descendant
   `new` that is already used (claimed by a file, or declared as an alias) is
   refused (`Decided.ownChildUsed`, `Decided.renameAllowed`; the merge guard
   already refused every descendant target since round 8). Renaming onto an
   unused descendant stays allowed.
4. Merge via an alias name (confirmed (a)): stored page-name query arguments
   naming the merged source are written as the survivor's name (`queryMap`,
   `merge9`); children follow the typed name (confirmed (b): the text map is
   unchanged, `nsSubst old new`).

New data lives in new declarations only (no field is added to any structure
an existing `decide` witness evaluates). -/
import MergeWitness8b

set_option linter.unusedSectionVars false
namespace CM
namespace Rulings9
open Params Merge7 Merge7b Merge8 QueryRename6

variable {K : Type} [DecidableEq K] (P : Params K)

/-! ## 1. CM-NAMES (2): a journal is named by its stem -/

/-- **J9-1.** A journal file's name is its stem; its title is not read. -/
theorem name_journal {f : File} (h : f.dir = .journals) : P.name f = P.stem f := by
  simp [name, h]

/-- **J9-2.** A journal file claims the journal page of its stem's key. -/
theorem claim_journal_stem {f : File} (h : f.dir = .journals) :
    P.claim f = (.journal, P.key (P.stem f)) := by
  simp [claim, kindOf, name, h]

/-- **J9-3.** Page files are unchanged: the title, else the stem. -/
theorem name_page {f : File} (h : f.dir = .pages) :
    P.name f = (P.title f.format f.text).getD (P.stem f) := by
  simp [name, h]

/-- **J9-4.** Editing a journal's text (adding or changing `title::`) never
changes which page it claims. -/
theorem claim_journal_text {f : File} (h : f.dir = .journals) (t : String)
    (hs : P.stem { f with text := t } = P.stem f) :
    P.claim { f with text := t } = P.claim f := by
  rw [claim_journal_stem P h, claim_journal_stem P (f := { f with text := t }) h, hs]

/-! ## 2. EMPTY-ALIAS -/

/-- Round 8's resolution (AL1 for every real file), kept for comparison. -/
def resolveKeyR8 (G : Graph) (k : K) : Page K :=
  if P.real G (P.kindPref, k) then (P.kindPref, k)
  else if P.real G (otherKind P.kindPref, k) then (otherKind P.kindPref, k)
  else
    match minBy P.pageLe (P.declarers G k) with
    | some q => q
    | none => (P.refKind k, k)

/-- **E9-1 (AL1 for files with content).** A preferred-kind claimant with
content wins navigation. -/
theorem resolve_solid {G : Graph} {k : K} (h : P.solid G (P.kindPref, k) = true) :
    P.resolveKey G k = (P.kindPref, k) := by
  simp [resolveKey, h]

/-- **E9-2 (the ruling).** When no claimant of `k` (of either kind) has
content and some owner declares `k` as an alias, `k` opens a declaring owner
(the least under the O5 order), not the empty file. -/
theorem empty_alias_opens_declarer {G : Graph} {k : K}
    (h0 : P.solid G (P.kindPref, k) = false) (h1 : P.solid G (otherKind P.kindPref, k) = false)
    (hd : P.declarers G k ≠ []) :
    P.resolveKey G k ∈ P.declarers G k := by
  obtain ⟨q, hq⟩ : ∃ q, minBy P.pageLe (P.declarers G k) = some q := by
    cases he : minBy P.pageLe (P.declarers G k) with
    | none => exact absurd (minBy_eq_none.mp he) hd
    | some q => exact ⟨q, rfl⟩
  have : P.resolveKey G k = q := by simp [resolveKey, h0, h1, hq]
  rw [this]; exact minBy_mem hq

/-- **E9-3 (scope).** Without a declarer, resolution is round 8's: an empty
file that is nobody's alias still opens itself. -/
theorem resolve_eq_r8_undeclared {G : Graph} {k : K} (hd : P.declarers G k = []) :
    P.resolveKey G k = resolveKeyR8 P G k := by
  unfold resolveKey resolveKeyR8
  rw [hd]
  have e : ∀ p, (P.solid G p || P.real G p) = P.real G p := fun p => by
    cases h : P.solid G p
    · simp
    · simp [P.solid_real h]
  simp only [minBy, Option.isNone_none, Bool.and_true, e]

/-- **E9-4 (scope).** When every claimant of `k` that exists has content
(real = solid for both kinds), resolution is round 8's. -/
theorem resolve_eq_r8_of_content {G : Graph} {k : K}
    (h0 : P.solid G (P.kindPref, k) = P.real G (P.kindPref, k))
    (h1 : P.solid G (otherKind P.kindPref, k) = P.real G (otherKind P.kindPref, k)) :
    P.resolveKey G k = resolveKeyR8 P G k := by
  unfold resolveKey resolveKeyR8
  rw [h0, h1]
  cases r0 : P.real G (P.kindPref, k) <;> cases r1 : P.real G (otherKind P.kindPref, k) <;>
    simp
  all_goals (cases minBy P.pageLe (P.declarers G k) <;> simp)

/-- **E9-5 (Linked References follow the link, wherever it opens).** A block
whose text names `s` references the page `s` opens. -/
theorem link_references_opened {G : Graph} {b : BlockId} {s : String} (hs : s ∈ P.refNames G b) :
    P.References G b (P.resolve G s) := by
  rw [P.references_iff_keys]
  exact ⟨s, hs, Reach.symm (P.key_mem_concept G s)⟩

/-- **E9-6.** ... and it references the empty page itself, of either kind:
both pages are one alias class, so both show the block in Linked References. -/
theorem link_references_named {G : Graph} {b : BlockId} {s : String} (hs : s ∈ P.refNames G b)
    (κ : Kind) : P.References G b (κ, P.key s) := by
  rw [P.references_iff_keys]
  exact ⟨s, hs, Reach.refl _⟩

/-- **E9-7 (still a page).** A content-free file is a real page and is listed
in the decided page universe. -/
theorem empty_page_listed {G : Graph} {p : Page K} (h : P.real G p = true) :
    p ∈ Decided.pages P G := by
  unfold Decided.pages
  exact P.mem_pagesR.mpr (Or.inl h)

/-- **E9-8.** Content-freeness is OG's emptiness on the model's terms: no title,
no alias declaration, and at most one block, which is blank. -/
theorem contentFree_iff (f : File) :
    P.contentFree f = true ↔ (P.title f.format f.text).isNone = true ∧
      P.aliasDecl f.format f.text = [] ∧
      (P.outline f.format f.text = [] ∨ ∃ t, P.outline f.format f.text = [t] ∧ t = "") := by
  unfold contentFree
  cases P.outline f.format f.text with
  | nil => simp
  | cons t r => cases r <;> simp [String.isEmpty_iff, and_assoc]

/-! ## 3. CM-OWN-CHILD -/

theorem ownChildUsed_iff {G : Graph} {old new : String} :
    Decided.ownChildUsed P G old new = true ↔
      (nsMatch P.key P.urlHasOrigin old new).isSome = true ∧ P.key new ≠ P.key old ∧
        (P.claimedKey G (P.key new) = true ∨ P.declaredKey G (P.key new) = true) := by
  simp [Decided.ownChildUsed, and_assoc]

/-- **C9-1 (unused child: allowed as before).** The new clause never fires
when `new` is neither claimed nor declared. -/
theorem ownChild_unused {G : Graph} {old new : String}
    (hc : P.claimedKey G (P.key new) = false) (hd : P.declaredKey G (P.key new) = false) :
    Decided.ownChildUsed P G old new = false := by
  simp [Decided.ownChildUsed, hc, hd]

/-- **C9-2 (every round-8 refusal is kept).** -/
theorem renameAllowed_r8 {G : Graph} {old new : String}
    (h : Decided.renameAllowed P G old new = true) : Decided.renameAllowedR8 P G old new = true := by
  rw [Decided.renameAllowed_eq_r8, Bool.and_eq_true] at h
  exact h.1

/-- **C9-3 (exactly one new refusal).** The decided guard differs from round
8's only on used strict descendants. -/
theorem renameAllowed_eq_r8_of_not_ownChild {G : Graph} {old new : String}
    (h : Decided.ownChildUsed P G old new = false) :
    Decided.renameAllowed P G old new = Decided.renameAllowedR8 P G old new := by
  rw [Decided.renameAllowed_eq_r8, h, Bool.not_false, Bool.and_true]

/-- **C9-4 (merge side).** Every merge whose typed name lies strictly below
the source is refused (round 8's M-Q6 clause; a merge target is always used). -/
theorem merge_own_child_refused (R : Decided.Rewriter) (J : Merger) {G : Graph} {old new : String}
    (h : (nsMatch P.key P.urlHasOrigin old new).isSome = true) : merge P R J G old new = none := by
  unfold merge
  split
  · next S D hS hD =>
    have ha : mergeAllowed P G old new = false := by
      unfold mergeAllowed; rw [hS, hD]; simp [mergeAllowedTo, h]
    simp [ha]
  · rfl

theorem nsSubst_of_none {old new s : String} (h : nsMatch P.key P.urlHasOrigin old s = none) :
    nsSubst P.key P.urlHasOrigin old new s = s := by
  unfold nsSubst; simp [h]

/-- **C9-5 (MB1 without its fixed-alias premise).** Round 8's
`Merge8.rename_onto_alias` assumed `σ (σ (key S)) = σ (key S)`. When the
decided round-9 guard accepts the rename of `old` (the source `S`) onto the
alias spelling `new` declared by `D`, that premise holds automatically: the
CM-OWN-CHILD clause excludes `new` below `old` (the r8b counterexample), and
otherwise the rename map fixes `new`. `R`/`m` are the rename's rewriter and
spelling map (`m = nsSubst key old new`). -/
theorem rename_onto_alias_decided {R : CM.Rewriter} {m : String → String} {σ : K → K} {G : Graph}
    (H : P.MergeHyps R m σ G) {old new : String} (hm : m = nsSubst P.key P.urlHasOrigin old new)
    (hacc : Decided.renameAllowed P G old new = true)
    {S D : File} (hS : S ∈ G) (hD : D ∈ G) (hSp : kindOf S = .page)
    (hSo : (P.claim S).2 = P.key old) (hσS : σ (P.claim S).2 = P.key new)
    (hs : new ∈ P.aliasDecl D.format D.text)
    (he : ((P.claim D).2, P.key new) ∈ P.aliasEdges G)
    (hDk : σ (P.claim D).2 = (P.claim D).2) :
    P.real (renameBy R m G) (.page, P.key new) = true ∧
    P.real (renameBy R m G) (P.claim D) = true ∧
    cls (P.aliasEdges (renameBy R m G)) (P.claim D).2 (P.key new) ∧
    ∀ f ∈ G, ∀ i, (∀ o ∈ P.occAt G (f.path, i), ∀ u ∈ P.occClosure o.2,
        ∃ v ∈ P.occClosure (rwOcc R.covers m o).2, P.key v = σ (P.key u)) →
      (P.References G (f.path, i) (P.claim S) ∨
        P.References G (f.path, i) (.page, P.key new) ∨
        P.References G (f.path, i) (P.claim D)) →
      P.References (renameBy R m G) ((R.apply m f).path, i) (.page, P.key new) ∧
      P.References (renameBy R m G) ((R.apply m f).path, i) (P.claim D) := by
  have hown : Decided.ownChildUsed P G old new = false := by
    rw [Decided.renameAllowed_eq_r8, Bool.and_eq_true] at hacc
    simpa using hacc.2
  have hdecl : P.declaredKey G (P.key new) = true := P.declaredKey_iff.mpr ⟨_, he, rfl⟩
  have hfix : σ (P.key new) = P.key new := by
    by_cases hk : P.key new = P.key old
    · rw [hk, ← hSo, hσS, hSo]; exact hk
    · have hn : (nsMatch P.key P.urlHasOrigin old new).isSome = false := by
        simpa [Decided.ownChildUsed, hk, hdecl] using hown
      have hnone : nsMatch P.key P.urlHasOrigin old new = none := by simpa using hn
      have := H.alias D hD new hs
      rw [hm, nsSubst_of_none P hnone] at this
      exact this.symm
  have ha : σ (σ (P.claim S).2) = σ (P.claim S).2 := by rw [hσS]; exact hfix
  have he' : ((P.claim D).2, σ (P.claim S).2) ∈ P.aliasEdges G := by rw [hσS]; exact he
  have r := rename_onto_alias H hS hD hSp he' hDk ha
  simp only [hσS] at r
  exact r

/-! ## 4. Merge through an alias: stored page-name queries name the survivor -/

/-- The query-argument map of a merge into `D` (ruling (a)): an argument
naming the source (its key) is written as the survivor's own name; every
other argument (including the source's namespace children, ruling (b)) is
renamed like text, by `nsSubst old new`. -/
def queryMap (old new : String) (D : File) (s : String) : String :=
  if P.key s = P.key old then P.name D else nsSubst P.key P.urlHasOrigin old new s

/-- **Q9-1.** An argument naming the source is written as a spelling of the
survivor's key. For a direct merge (`D` claims `new`) that key is `new`'s. -/
theorem queryMap_old {old new s : String} {D : File} (h : P.key s = P.key old) :
    P.key (queryMap P old new D s) = (P.claim D).2 := by
  simp [queryMap, h, claim]

/-- **Q9-2.** Every other argument is renamed as text (children `old/x` go to
`new/x`, the typed name: ruling (b)). -/
theorem queryMap_other {old new s : String} {D : File} (h : P.key s ≠ P.key old) :
    queryMap P old new D s = nsSubst P.key P.urlHasOrigin old new s := by
  simp [queryMap, h]

/-- A rewriter whose query positions take their own spelling map. -/
structure QRewriter where
  apply : (String → String) → (String → String) → File → File

/-- The decided rewriter at a fixed query map. -/
def QRewriter.at (R : QRewriter) (mq : String → String) : Decided.Rewriter := ⟨fun m => R.apply m mq⟩

/-- **The round-9 merge.** Round 8's guard and after-graph shape, with the
rewriter and the survivor builder given the query map of the chosen target. -/
def merge9 (R : QRewriter) (J : (String → String) → Merger) (G : Graph) (old new : String) :
    Option Graph :=
  match mergeTarget P G old new with
  | some D => merge P (R.at (queryMap P old new D)) (J (queryMap P old new D)) G old new
  | none => none

/-- **Q9-3 (same guard).** `merge9` accepts exactly when round 8's merge does
(for any rewriter and builder): the rulings change no merge refusal. -/
theorem merge9_isSome (R : QRewriter) (J : (String → String) → Merger) (R0 : Decided.Rewriter)
    (J0 : Merger) (G : Graph) (old new : String) :
    (merge9 P R J G old new).isSome = (merge P R0 J0 G old new).isSome := by
  unfold merge9
  cases hD : mergeTarget P G old new with
  | none => cases hS : mergeSource P G old <;> simp [merge, hS, hD]
  | some D =>
    simp only
    cases hS : mergeSource P G old with
    | none => simp [merge, hS, hD]
    | some S => simp only [merge, hS, hD]; split <;> rfl

/-- **Q9-4.** The merge side of CM-OWN-CHILD holds for `merge9` too. -/
theorem merge9_own_child_refused (R : QRewriter) (J : (String → String) → Merger) {G : Graph}
    {old new : String} (h : (nsMatch P.key P.urlHasOrigin old new).isSome = true) :
    merge9 P R J G old new = none := by
  have e := merge9_isSome P R J ⟨fun _ f => f⟩ ⟨fun _ _ D => D, fun _ _ _ _ => []⟩ G old new
  rw [merge_own_child_refused P _ _ h] at e
  simpa using e

/-- **Q9-5.** An accepted `merge9` is round 8's after-graph shape at the
query map of its target. -/
theorem merge9_structure (R : QRewriter) (J : (String → String) → Merger) {G G' : Graph}
    {old new : String} (h : merge9 P R J G old new = some G') :
    ∃ S D, mergeSource P G old = some S ∧ mergeTarget P G old new = some D ∧
      G' = mergeBy (R.at (queryMap P old new D)).base (J (queryMap P old new D))
        (nsSubst P.key P.urlHasOrigin old new) S D G := by
  unfold merge9 at h
  cases hD : mergeTarget P G old new with
  | none => rw [hD] at h; cases h
  | some D =>
    rw [hD] at h
    obtain ⟨S, D', hS, hD', hG, -⟩ := merge_structure (P := P) _ _ h
    rw [hD] at hD'; cases hD'
    exact ⟨S, D, hS, rfl, hG⟩

section Queries
variable {L : Type} [DecidableEq L] (SP : StoredParser L)

/-- Survivor stored-query law of `merge9`: `QueryLaw` with the query map. -/
def QueryLaw9 (J : Merger) (m mq : String → String) (S D : File) : Prop :=
  ∀ i, i < (survOut P J m S D).length →
    (((survOut P J m S D)[i]?).map (SP.extract (J.apply m S D).format)).getD [] =
      (J.origin m S D i).flatMap (fun x => (originQueries P SP S D x).map (renameQuery mq))

/-- Other files' stored-query law of `merge9`. -/
def StoredOthers9 (R : CM.Rewriter) (m mq : String → String) (S D : File) (G : Graph) : Prop :=
  ∀ f ∈ others S D G,
    (P.outline (R.apply m f).format (R.apply m f).text).map (SP.extract (R.apply m f).format) =
      ((P.outline f.format f.text).map (SP.extract f.format)).map (List.map (renameQuery mq))

variable {P SP} {R : CM.Rewriter} {J : Merger} {m mq : String → String} {τ σa : K → K}
  {S D : File} {G : Graph}

/-- **Q9-6 (survivor blocks).** As MQ1, with the query map. -/
theorem storedAt_survivor9 (H : MergeBase P R J m τ S D G σa) (hq : QueryLaw9 P SP J m mq S D)
    {i : Nat} (hi : i < (survOut P J m S D).length) :
    storedAt P SP (mergeBy R J m S D G) (D.path, i) =
      (J.origin m S D i).flatMap (fun x =>
        (storedAt P SP G ((oldFile S D x).path, x.2)).map (renameQuery mq)) := by
  have h := storedAt_some P SP H.wf' (survivor_mem H) i
  rw [H.merger.path] at h
  rw [h]
  change (((survOut P J m S D)[i]?).map (SP.extract (J.apply m S D).format)).getD [] = _
  rw [hq i hi]
  congr 1
  funext x
  obtain ⟨c, j⟩ := x
  cases c
  · simp only [oldFile, originQueries]; rw [storedAt_some P SP H.wf H.dst]
  · simp only [oldFile, originQueries]; rw [storedAt_some P SP H.wf H.src]

/-- **Q9-7 (other blocks).** -/
theorem storedAt_other9 (H : MergeBase P R J m τ S D G σa) (ho : StoredOthers9 P SP R m mq S D G)
    {f : File} (hf : f ∈ others S D G) (i : Nat) :
    storedAt P SP (mergeBy R J m S D G) ((R.apply m f).path, i) =
      (storedAt P SP G (f.path, i)).map (renameQuery mq) := by
  rw [storedAt_some P SP H.wf' (other_mem hf), storedAt_some P SP H.wf (others_sub hf)]
  have h := congrArg (fun xs => xs[i]?) (ho f hf)
  simp only [List.getElem?_map] at h
  rw [h]
  cases (P.outline f.format f.text)[i]? <;> rfl

/-- **Q9-8 (ruling (a), the guarantee).** After the merge, a stored
`(page "<source>")` (any spelling of the source's key), rewritten by the query
map, matches every block of the merged page ... -/
theorem pageLeaf_survivor9 (Q : QParams K L) (H : MergeBase P R J m τ S D G σa)
    {old new s : String} (hs : P.key s = P.key old) (i : Nat) :
    P.leafBlock3 Q (mergeBy R J m S D G) (.page (.name (queryMap P old new D s))) (D.path, i) = true := by
  have h1 := fileAt_eq_of_mem H.wf' (survivor_mem H)
  rw [H.merger.path] at h1
  simp [leafBlock3, pageOfBlock, h1, leafPage, claim_survivor H, queryMap_old P hs]

/-- **Q9-9.** ... and no block of any other page. -/
theorem pageLeaf_other9 (Q : QParams K L) (H : MergeBase P R J m τ S D G σa)
    {old new s : String} (hs : P.key s = P.key old) {f : File} (hf : f ∈ others S D G) (i : Nat) :
    P.leafBlock3 Q (mergeBy R J m S D G) (.page (.name (queryMap P old new D s)))
      ((R.apply m f).path, i) = false := by
  have h1 := fileAt_eq_of_mem H.wf' (other_mem hf)
  have hc := claim_apply H.laws hf
  have hne := other_key_ne_dst H hf
  simp only [leafBlock3, pageOfBlock, h1, Option.map_some, leafPage, hc, σp, queryMap_old P hs,
    decide_eq_false_iff_not]
  exact fun e => hne e.symm

end Queries

end Rulings9
end CM
