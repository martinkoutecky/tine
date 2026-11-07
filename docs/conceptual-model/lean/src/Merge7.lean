/- Round 7 (D-20261102-CM4): MERGE as its own operation.

Tine's confirmed merge (`rename_or_merge_page(old, new, …, merge_into)`,
`tine-graph-features/src/pages.rs`, plan `pages_rename_plan.rs`, payload
`pages/merge_payload.rs`), after the user confirms "Page already exists, merge
to it?": the source page file `S` (claiming `old`) is merged into the survivor
`D` (the sole file claiming `new`). Every file is rewritten by the rename map
(references, namespace descendants, as in rename), the survivor keeps its path,
name and blocks, the source's header joins the survivor's (aliases united), and
every source block is appended (journal 2026-10-05 P2: always append, like OG
`merge-pages!`). The source file is deleted.

The model abstracts the survivor's text the way it abstracts a rename's
rewriter: `Merger.apply` builds the survivor, and `Merger.origin` says which old
blocks each survivor block comes from. `MergerLaws` are checked at the fixture
boundary; they say only what Tine's payload does at the level of claims, alias
keys and occurrences, never that references are preserved.

Unlike `MergeRename` (reference-only and alias-only keys may merge), here two
REAL claimant keys merge: `σ (key S) = key D`. Everything else keeps the round-6
injectivity (`MergeOpHyps.realInj`). -/
import CaseWitness7

set_option linter.unusedSectionVars false
namespace CM
namespace Merge7
open Params

/-- Where a survivor block comes from: `(true, j)` is source block `j`,
`(false, j)` is target block `j`. A survivor block may have several origins
(the survivor's header after the source header joined it). -/
abbrev Origin := Bool × Nat

structure Merger where
  apply : (String → String) → File → File → File
  origin : (String → String) → File → File → Nat → List Origin

/-- The after-graph: the source is gone, the survivor replaces the target, every
other file is rewritten by the rename rewriter. -/
def mergeBy (R : CM.Rewriter) (J : Merger) (m : String → String) (S D : File) (G : Graph) : Graph :=
  (G.filter (fun f => decide (f ≠ S))).map (fun f => if f = D then J.apply m S D else R.apply m f)

/-- The files a merge only rewrites. -/
def others (S D : File) (G : Graph) : Graph :=
  G.filter (fun f => decide (f ≠ S) && decide (f ≠ D))

variable {K : Type} [DecidableEq K] (P : Params K)

def blockOcc (f : File) (j : Nat) : Option (List (OccKind × String)) :=
  ((P.outline f.format f.text)[j]?).map (P.occ f.format)

def originOcc (S D : File) : Origin → Option (List (OccKind × String))
  | (true, j) => blockOcc P S j
  | (false, j) => blockOcc P D j

theorem blockOcc_isSome {P : Params K} {f : File} {i : Nat} :
    (blockOcc P f i).isSome = true ↔ i < (P.outline f.format f.text).length := by
  simp [blockOcc]

/-- Survivor block count. -/
abbrev survLen (J : Merger) (m : String → String) (S D : File) : Nat :=
  (P.outline (J.apply m S D).format (J.apply m S D).text).length

/-- An old block `x` survives: it is the origin of some survivor block, and each
of its occurrences, rewritten, is in a survivor block having `x` as an origin. -/
abbrev CoversAt (R : CM.Rewriter) (J : Merger) (m : String → String) (S D : File) (x : Origin) : Prop :=
  (∃ i, i < survLen P J m S D ∧ x ∈ J.origin m S D i) ∧
  ∀ o ∈ (originOcc P S D x).getD [], ∃ i, i < survLen P J m S D ∧ x ∈ J.origin m S D i ∧
    rwOcc R.covers m o ∈ (blockOcc P (J.apply m S D) i).getD []

/-- The survivor's laws (Tine's `merged_survivor` + `merge_payload`):
same path, folder, format and page name as the target (the source's `title::`
does not rebind it); alias keys are the union of both pages' alias keys under
the rename (`alias::` members united); every survivor block has old origins,
its occurrences are rewritten occurrences of its origins (`occSubL`), and every
occurrence of every source and target block survives, rewritten, in a survivor
block having that origin (`coverS`, `coverD`: blocks are appended, never
dropped); the survivor's namespace parents are the rewritten target's. All
fields are bounded, so they are decidable on a concrete fixture. -/
structure MergerLaws (R : CM.Rewriter) (J : Merger) (m : String → String) (σ : K → K)
    (S D : File) : Prop where
  path : (J.apply m S D).path = D.path
  dir : (J.apply m S D).dir = D.dir
  format : (J.apply m S D).format = D.format
  name : P.key (P.name (J.apply m S D)) = P.key (P.name D)
  aliasL : (∀ s ∈ P.aliasDecl (J.apply m S D).format (J.apply m S D).text,
      ∃ s0 ∈ P.aliasDecl D.format D.text ++ P.aliasDecl S.format S.text, σ (P.key s0) = P.key s) ∧
    (∀ s0 ∈ P.aliasDecl D.format D.text ++ P.aliasDecl S.format S.text,
      ∃ s ∈ P.aliasDecl (J.apply m S D).format (J.apply m S D).text, P.key s = σ (P.key s0))
  nonempty : ∀ i, i < survLen P J m S D → J.origin m S D i ≠ []
  valid : ∀ i, i < survLen P J m S D → ∀ x ∈ J.origin m S D i, (originOcc P S D x).isSome = true
  occSubL : ∀ i, i < survLen P J m S D → ∀ o ∈ (blockOcc P (J.apply m S D) i).getD [],
    ∃ x ∈ J.origin m S D i, ∃ o' ∈ (originOcc P S D x).getD [], rwOcc R.covers m o' = o
  coverS : ∀ j, j < (P.outline S.format S.text).length → CoversAt P R J m S D (true, j)
  coverD : ∀ j, j < (P.outline D.format D.text).length → CoversAt P R J m S D (false, j)
  namesL : (∀ s ∈ P.fileNsNames (J.apply m S D), ∃ s' ∈ P.fileNsNames (R.apply m D), P.key s' = P.key s) ∧
    (∀ s' ∈ P.fileNsNames (R.apply m D), ∃ s ∈ P.fileNsNames (J.apply m S D), P.key s = P.key s')

section
variable {P} {R : CM.Rewriter} {J : Merger} {m : String → String} {σ : K → K} {S D : File}

theorem MergerLaws.alias (L : MergerLaws P R J m σ S D) (k : K) :
    (∃ s ∈ P.aliasDecl (J.apply m S D).format (J.apply m S D).text, P.key s = k) ↔
      ∃ s ∈ P.aliasDecl D.format D.text ++ P.aliasDecl S.format S.text, σ (P.key s) = k := by
  constructor
  · rintro ⟨s, hs, e⟩
    obtain ⟨s0, hs0, e0⟩ := L.aliasL.1 s hs
    exact ⟨s0, hs0, e0.trans e⟩
  · rintro ⟨s0, hs0, e⟩
    obtain ⟨s, hs, e1⟩ := L.aliasL.2 s0 hs0
    exact ⟨s, hs, e1.trans e⟩

theorem MergerLaws.names (L : MergerLaws P R J m σ S D) (k : K) :
    k ∈ (P.fileNsNames (J.apply m S D)).map P.key ↔ k ∈ (P.fileNsNames (R.apply m D)).map P.key := by
  simp only [List.mem_map]
  constructor
  · rintro ⟨s, hs, rfl⟩
    obtain ⟨s', hs', e⟩ := L.namesL.1 s hs
    exact ⟨s', hs', e⟩
  · rintro ⟨s', hs', rfl⟩
    obtain ⟨s, hs, e⟩ := L.namesL.2 s' hs'
    exact ⟨s, hs, e⟩

theorem MergerLaws.occ_sub (L : MergerLaws P R J m σ S D) (i : Nat) (os : List (OccKind × String))
    (h : blockOcc P (J.apply m S D) i = some os) (o : OccKind × String) (ho : o ∈ os) :
    ∃ x ∈ J.origin m S D i, ∃ os', originOcc P S D x = some os' ∧
      ∃ o' ∈ os', rwOcc R.covers m o' = o := by
  have hi : i < survLen P J m S D := blockOcc_isSome.mp (by rw [h]; rfl)
  obtain ⟨x, hx, o', ho', e⟩ := L.occSubL i hi o (by rw [h]; exact ho)
  obtain ⟨os', hos'⟩ := Option.isSome_iff_exists.mp (L.valid i hi x hx)
  exact ⟨x, hx, os', hos', o', by rw [hos'] at ho'; exact ho', e⟩

theorem MergerLaws.cover (L : MergerLaws P R J m σ S D) (x : Origin) (os : List (OccKind × String))
    (h : originOcc P S D x = some os) :
    (∃ i, x ∈ J.origin m S D i ∧ (blockOcc P (J.apply m S D) i).isSome = true) ∧
    ∀ o ∈ os, ∃ i, x ∈ J.origin m S D i ∧ ∃ os'', blockOcc P (J.apply m S D) i = some os'' ∧
      rwOcc R.covers m o ∈ os'' := by
  have hc : CoversAt P R J m S D x := by
    obtain ⟨c, j⟩ := x
    have hs : (originOcc P S D (c, j)).isSome = true := by rw [h]; rfl
    cases c
    · exact L.coverD j (blockOcc_isSome.mp hs)
    · exact L.coverS j (blockOcc_isSome.mp hs)
  obtain ⟨⟨i, hi, hx⟩, hall⟩ := hc
  refine ⟨⟨i, hx, blockOcc_isSome.mpr hi⟩, fun o ho => ?_⟩
  obtain ⟨i, hi, hx, hm⟩ := hall o (by rw [h]; exact ho)
  obtain ⟨os'', hos''⟩ := Option.isSome_iff_exists.mp (blockOcc_isSome.mpr hi)
  exact ⟨i, hx, os'', hos'', by rw [hos''] at hm; exact hm⟩

end

/-- Round 8 (D-20261102-MC, M-Q7): the merge hypotheses for a survivor of
either kind. As `MergeOpHyps` without `dstPage`; `dstAlias` (the survivor's kind
may declare aliases: O4, decided for both kinds) is all that the alias-edge
results need of the survivor's kind. Every lemma below whose statement holds for
a journal survivor takes this structure; `MergeOpHyps` (page survivor) extends
it, so each such lemma applies to every round-7 instance through
`MergeOpHyps.toMergeBase`.

Round 8b: the alias contracts (`alias`, `merger.aliasL`) read alias spellings
under `σa`, the map the rewritten TEXT applies to alias keys; every other field
reads page keys under `σ`. `σa` defaults to `σ` (round 7: one map), so
`MergeBase P R J m σ S D G` is round 7's structure. A merge into the owner of
an alias (M-Q5a, `Merge8`) has `σ` = the page map (`src ↦ owner`) and `σa` =
the text map (`src ↦ new`): an `alias:: src` anywhere is rewritten to
`alias:: new`, not to the owner's name. Lemmas that do not read alias
declarations are stated for any `σa`. -/
structure MergeBase (R : CM.Rewriter) (J : Merger) (m : String → String) (σ : K → K)
    (S D : File) (G : Graph) (σa : K → K := σ) : Prop where
  lawful : P.Lawful
  wf : WF G
  wf' : WF (mergeBy R J m S D G)
  src : S ∈ G
  dst : D ∈ G
  ne : S ≠ D
  srcPage : kindOf S = .page
  srcKey : σ (P.claim S).2 = (P.claim D).2
  dstKey : σ (P.claim D).2 = (P.claim D).2
  srcUniq : ∀ f ∈ G, (P.claim f).2 = (P.claim S).2 → f = S
  dstUniq : ∀ f ∈ G, (P.claim f).2 = (P.claim D).2 → f = D
  aliasPage : P.aliasKinds .page = true
  dstAlias : P.aliasKinds (kindOf D) = true
  laws : P.RwLaws R m σ (others S D G)
  alias : ∀ f ∈ others S D G, ∀ s ∈ P.aliasDecl f.format f.text, P.key (m s) = σa (P.key s)
  realInj : ∀ f ∈ others S D G, ∀ g ∈ G, g ≠ S →
    σ (P.claim f).2 = σ (P.claim g).2 → (P.claim f).2 = (P.claim g).2
  merger : MergerLaws P R J m σa S D

/-- Hypotheses of the merge theorems: well-formed graphs; the source and target
are distinct page files, each the only file claiming its key; `σ` sends the
source key to the target key and fixes the target key; the other files obey
the rename laws, and real keys other than the source stay injective.
(Round 8: the fields other than `dstPage` form `MergeBase`; its `dstAlias`
follows from `dstPage` and `aliasPage`.) -/
structure MergeOpHyps (R : CM.Rewriter) (J : Merger) (m : String → String) (σ : K → K)
    (S D : File) (G : Graph) : Prop extends MergeBase P R J m σ S D G where
  dstPage : kindOf D = .page

variable {P} {R : CM.Rewriter} {J : Merger} {m : String → String} {σ σa : K → K} {S D : File}
  {G : Graph}

/-! ## Files and claims -/

theorem mem_others {f : File} : f ∈ others S D G ↔ f ∈ G ∧ f ≠ S ∧ f ≠ D := by
  simp [others]

theorem mem_mergeBy {f' : File} :
    f' ∈ mergeBy R J m S D G ↔
      (D ∈ G ∧ D ≠ S ∧ f' = J.apply m S D) ∨ ∃ f ∈ others S D G, R.apply m f = f' := by
  simp only [mergeBy, List.mem_map, List.mem_filter, decide_eq_true_eq, mem_others]
  constructor
  · rintro ⟨f, ⟨hf, hfs⟩, e⟩
    by_cases hd : f = D
    · subst hd; rw [if_pos rfl] at e; exact Or.inl ⟨hf, hfs, e.symm⟩
    · rw [if_neg hd] at e; exact Or.inr ⟨f, ⟨hf, hfs, hd⟩, e⟩
  · rintro (⟨hD, hDS, e⟩ | ⟨f, ⟨hf, hfs, hd⟩, e⟩)
    · exact ⟨D, ⟨hD, hDS⟩, by rw [if_pos rfl, e]⟩
    · exact ⟨f, ⟨hf, hfs⟩, by rw [if_neg hd, e]⟩

theorem survivor_mem (H : MergeBase P R J m σ S D G σa) : J.apply m S D ∈ mergeBy R J m S D G :=
  mem_mergeBy.mpr (Or.inl ⟨H.dst, fun e => H.ne e.symm, rfl⟩)

theorem other_mem {f : File} (hf : f ∈ others S D G) : R.apply m f ∈ mergeBy R J m S D G :=
  mem_mergeBy.mpr (Or.inr ⟨f, hf, rfl⟩)

theorem claim_survivor (H : MergeBase P R J m σ S D G σa) : P.claim (J.apply m S D) = P.claim D := by
  unfold claim
  have hk : kindOf (J.apply m S D) = kindOf D := by unfold kindOf; rw [H.merger.dir]
  rw [hk, H.merger.name]

theorem σp_src (H : MergeOpHyps P R J m σ S D G) : σp σ (P.claim S) = P.claim D := by
  have e1 : P.claim S = (.page, (P.claim S).2) := by unfold claim; rw [H.srcPage]
  have e2 : P.claim D = (.page, (P.claim D).2) := by unfold claim; rw [H.dstPage]
  rw [e1]; simp only [σp]; rw [H.srcKey]; exact e2.symm

theorem σp_dst (H : MergeOpHyps P R J m σ S D G) : σp σ (P.claim D) = P.claim D := by
  have e2 : P.claim D = (.page, (P.claim D).2) := by unfold claim; rw [H.dstPage]
  rw [e2]; simp only [σp]; rw [H.dstKey]

theorem others_sub {f : File} (hf : f ∈ others S D G) : f ∈ G := (mem_others.mp hf).1

theorem other_key_ne_dst (H : MergeBase P R J m σ S D G σa) {f : File} (hf : f ∈ others S D G) :
    σ (P.claim f).2 ≠ (P.claim D).2 := by
  intro e
  have hk := H.realInj f hf D H.dst (fun e' => H.ne e'.symm) (by rw [e, H.dstKey])
  exact (mem_others.mp hf).2.2 (H.dstUniq f (others_sub hf) hk)

theorem other_claim_ne {f g : File} (H : MergeBase P R J m σ S D G σa) (hf : f ∈ others S D G)
    (hg : g ∈ G) (hc : P.claim g = P.claim f) : g ∈ others S D G := by
  refine mem_others.mpr ⟨hg, fun e => ?_, fun e => ?_⟩
  · subst e
    exact (mem_others.mp hf).2.1 (H.srcUniq f (others_sub hf) (by rw [hc]))
  · subst e
    exact (mem_others.mp hf).2.2 (H.dstUniq f (others_sub hf) (by rw [hc]))

theorem σp_inj_other (H : MergeBase P R J m σ S D G σa) {f g : File} (hf : f ∈ others S D G)
    (hg : g ∈ G) (hgS : g ≠ S) (e : σp σ (P.claim f) = σp σ (P.claim g)) : P.claim f = P.claim g := by
  have hk := congrArg (fun p : Page K => p.1) e
  have hv := congrArg (fun p : Page K => p.2) e
  exact Prod.ext hk (H.realInj f hf g hg hgS hv)

/-- The rewrite is injective on the files a merge keeps. -/
theorem kept_inj (H : MergeBase P R J m σ S D G σa) {f g : File}
    (hf : f ∈ G) (hfS : f ≠ S) (hg : g ∈ G) (hgS : g ≠ S)
    (e : (if f = D then J.apply m S D else R.apply m f) = (if g = D then J.apply m S D else R.apply m g)) :
    f = g := by
  have hn := H.wf'
  unfold WF mergeBy at hn
  rw [List.map_map] at hn
  exact CaseRelabel7.inj_of_nodup_map hn (List.mem_filter.mpr ⟨hf, by simpa using hfS⟩)
    (List.mem_filter.mpr ⟨hg, by simpa using hgS⟩) (by simp only [Function.comp]; rw [e])

theorem apply_inj_other (H : MergeBase P R J m σ S D G σa) {f g : File}
    (hf : f ∈ others S D G) (hg : g ∈ others S D G) (e : R.apply m f = R.apply m g) : f = g := by
  have hf' := mem_others.mp hf
  have hg' := mem_others.mp hg
  exact kept_inj H hf'.1 hf'.2.1 hg'.1 hg'.2.1 (by rw [if_neg hf'.2.2, if_neg hg'.2.2, e])

theorem survivor_ne_other (H : MergeBase P R J m σ S D G σa) {f : File} (hf : f ∈ others S D G) :
    R.apply m f ≠ J.apply m S D := by
  intro e
  have hf' := mem_others.mp hf
  exact hf'.2.2 (kept_inj H hf'.1 hf'.2.1 H.dst (fun e => H.ne e.symm) (by rw [if_neg hf'.2.2, if_pos rfl, e]))

/-! ## Real pages and owners -/

/-- **MG1 (real pages).** The after-graph's real pages are exactly the σ-image
of the before-graph's: the source page has become the target page. -/
theorem real_merge7 (H : MergeOpHyps P R J m σ S D G) (q : Page K) :
    P.real (mergeBy R J m S D G) q = true ↔ ∃ p, P.real G p = true ∧ σp σ p = q := by
  rw [P.real_iff]
  constructor
  · rintro ⟨f', hf', e⟩
    rcases mem_mergeBy.mp hf' with ⟨_, _, rfl⟩ | ⟨f, hf, rfl⟩
    · exact ⟨P.claim D, P.real_iff.mpr ⟨D, H.dst, rfl⟩, by rw [σp_dst H, ← claim_survivor H.toMergeBase, e]⟩
    · exact ⟨P.claim f, P.real_iff.mpr ⟨f, others_sub hf, rfl⟩, by
        rw [← claim_apply H.laws hf, e]⟩
  · rintro ⟨p, hp, rfl⟩
    obtain ⟨f, hf, rfl⟩ := P.real_iff.mp hp
    by_cases hS : f = S
    · subst hS; exact ⟨_, survivor_mem H.toMergeBase, by rw [claim_survivor H.toMergeBase, σp_src H]⟩
    by_cases hD : f = D
    · subst hD; exact ⟨_, survivor_mem H.toMergeBase, by rw [claim_survivor H.toMergeBase, σp_dst H]⟩
    have ho : f ∈ others S D G := mem_others.mpr ⟨hf, hS, hD⟩
    exact ⟨_, other_mem ho, claim_apply H.laws ho⟩

theorem claimant_dst (H : MergeBase P R J m σ S D G σa) {f' : File}
    (hf' : f' ∈ mergeBy R J m S D G) (e : P.claim f' = P.claim D) : f' = J.apply m S D := by
  rcases mem_mergeBy.mp hf' with ⟨_, _, rfl⟩ | ⟨f, hf, rfl⟩
  · rfl
  · rw [claim_apply H.laws hf] at e
    exact absurd (congrArg Prod.snd e) (other_key_ne_dst H hf)

/-- **MG2a (the survivor owns the merged page).** -/
theorem owner_dst (H : MergeBase P R J m σ S D G σa) :
    P.owner (mergeBy R J m S D G) (P.claim D) = some (J.apply m S D) := by
  have e : P.owner (mergeBy R J m S D G) (P.claim D) = minBy P.fileLe [J.apply m S D] := by
    unfold owner
    apply minBy_congr H.lawful.file
    intro x
    rw [P.mem_claimants, List.mem_singleton]
    constructor
    · rintro ⟨hx, e⟩; exact claimant_dst H hx e
    · rintro rfl; exact ⟨survivor_mem H, claim_survivor H⟩
  rw [e]; rfl

theorem claimants_other (H : MergeBase P R J m σ S D G σa) {f : File} (hf : f ∈ others S D G)
    (x : File) : x ∈ P.claimants (mergeBy R J m S D G) (σp σ (P.claim f)) ↔
      x ∈ (P.claimants G (P.claim f)).map (R.apply m) := by
  rw [P.mem_claimants, List.mem_map]
  constructor
  · rintro ⟨hx, e⟩
    rcases mem_mergeBy.mp hx with ⟨_, _, rfl⟩ | ⟨g, hg, rfl⟩
    · rw [claim_survivor H] at e
      exact absurd (congrArg Prod.snd e).symm (other_key_ne_dst H hf)
    · rw [claim_apply H.laws hg] at e
      have hg' := mem_others.mp hg
      exact ⟨g, P.mem_claimants.mpr ⟨hg'.1,
        (σp_inj_other H hf hg'.1 hg'.2.1 e.symm).symm⟩, rfl⟩
  · rintro ⟨g, hg, rfl⟩
    obtain ⟨hgG, hc⟩ := P.mem_claimants.mp hg
    have ho := other_claim_ne H hf hgG hc
    exact ⟨other_mem ho, by rw [claim_apply H.laws ho, hc]⟩

/-- **MG2b (owners of every other page).** -/
theorem owner_other (H : MergeBase P R J m σ S D G σa) {f : File} (hf : f ∈ others S D G) :
    P.owner (mergeBy R J m S D G) (σp σ (P.claim f)) = (P.owner G (P.claim f)).map (R.apply m) := by
  unfold owner
  rw [minBy_congr H.lawful.file (claimants_other H hf)]
  apply minBy_map
  intro x hx y hy
  obtain ⟨hxG, hxc⟩ := P.mem_claimants.mp hx
  obtain ⟨hyG, hyc⟩ := P.mem_claimants.mp hy
  exact H.laws.order x (other_claim_ne H hf hxG hxc) y (other_claim_ne H hf hyG hyc)
    (hxc.trans hyc.symm)

theorem isOwner_survivor (H : MergeBase P R J m σ S D G σa) :
    P.isOwner (mergeBy R J m S D G) (J.apply m S D) = true := by
  unfold isOwner; rw [claim_survivor H, owner_dst H]; simp

theorem isOwner_other (H : MergeBase P R J m σ S D G σa) {f : File} (hf : f ∈ others S D G) :
    P.isOwner (mergeBy R J m S D G) (R.apply m f) = P.isOwner G f := by
  unfold isOwner
  rw [claim_apply H.laws hf, owner_other H hf]
  apply decide_eq_decide.mpr
  constructor
  · intro e
    obtain ⟨g, hg, eg⟩ := Option.map_eq_some_iff.mp e
    obtain ⟨hgG, hgc⟩ := P.owner_spec hg
    rw [hg, apply_inj_other H (other_claim_ne H hf hgG hgc) hf eg]
  · intro e; rw [e]; rfl

theorem unique_owner {f : File} (hP : P.Lawful) (hf : f ∈ G)
    (hu : ∀ g ∈ G, (P.claim g).2 = (P.claim f).2 → g = f) : P.isOwner G f = true := by
  have e : P.owner G (P.claim f) = minBy P.fileLe [f] := by
    unfold owner
    apply minBy_congr hP.file
    intro x
    rw [P.mem_claimants, List.mem_singleton]
    constructor
    · rintro ⟨hx, e⟩; exact hu x hx (by rw [e])
    · rintro rfl; exact ⟨hf, rfl⟩
  unfold isOwner; rw [e]; simp [minBy]

theorem isOwner_src (H : MergeBase P R J m σ S D G σa) : P.isOwner G S = true :=
  unique_owner H.lawful H.src H.srcUniq
theorem isOwner_dst (H : MergeBase P R J m σ S D G σa) : P.isOwner G D = true :=
  unique_owner H.lawful H.dst H.dstUniq

/-! ## Alias classes -/

/-- **MG3 (alias edges), round 8b split form.** The after-graph's alias edges
are exactly the before-graph's edges with the declaring owner's key mapped by
the page map `σ` and the alias key by the text map `σa`: the source's and the
target's alias declarations both become the survivor's. -/
theorem aliasEdges_split (H : MergeBase P R J m σ S D G σa) (e : K × K) :
    e ∈ P.aliasEdges (mergeBy R J m S D G) ↔
      e ∈ (P.aliasEdges G).map (fun e => (σ e.1, σa e.2)) := by
  obtain ⟨a, b⟩ := e
  have hmap : (a, b) ∈ (P.aliasEdges G).map (fun e => (σ e.1, σa e.2)) ↔
      ∃ x y, (x, y) ∈ P.aliasEdges G ∧ σ x = a ∧ σa y = b := by
    simp only [List.mem_map, Prod.mk.injEq]
    constructor
    · rintro ⟨⟨x, y⟩, h, hx, hy⟩; exact ⟨x, y, h, hx, hy⟩
    · rintro ⟨x, y, h, hx, hy⟩; exact ⟨(x, y), h, hx, hy⟩
  rw [P.mem_aliasEdges, hmap]
  constructor
  · rintro ⟨f', hf', ha, s', hs', hb⟩
    obtain ⟨hf'G, _, _⟩ := P.mem_declaringOwners.mp hf'
    rcases mem_mergeBy.mp hf'G with ⟨_, _, rfl⟩ | ⟨f, hf, rfl⟩
    · obtain ⟨s, hs, hk⟩ := (H.merger.alias b).mp ⟨s', hs', hb⟩
      rw [claim_survivor H] at ha
      rcases List.mem_append.mp hs with hs | hs
      · refine ⟨(P.claim D).2, P.key s, ?_, by rw [H.dstKey, ha], hk⟩
        exact P.mem_aliasEdges.mpr ⟨D, P.mem_declaringOwners.mpr
          ⟨H.dst, isOwner_dst H, H.dstAlias⟩, rfl, s, hs, rfl⟩
      · refine ⟨(P.claim S).2, P.key s, ?_, by rw [H.srcKey, ha], hk⟩
        exact P.mem_aliasEdges.mpr ⟨S, P.mem_declaringOwners.mpr
          ⟨H.src, isOwner_src H, by rw [H.srcPage]; exact H.aliasPage⟩, rfl, s, hs, rfl⟩
    · have hfG := others_sub hf
      obtain ⟨_, ho, hk⟩ := P.mem_declaringOwners.mp hf'
      rw [isOwner_other H hf] at ho
      have hkind : kindOf (R.apply m f) = kindOf f := by unfold kindOf; rw [H.laws.dir f hf]
      rw [H.laws.alias f hf] at hs'
      obtain ⟨s, hs, rfl⟩ := List.mem_map.mp hs'
      refine ⟨(P.claim f).2, P.key s, P.mem_aliasEdges.mpr ⟨f, P.mem_declaringOwners.mpr
        ⟨hfG, ho, by rw [← hkind]; exact hk⟩, rfl, s, hs, rfl⟩, ?_, ?_⟩
      · rw [claim_apply H.laws hf] at ha; exact ha
      · rw [← H.alias f hf s hs]; exact hb
  · rintro ⟨x, y, hxy, rfl, rfl⟩
    obtain ⟨g, hg, hx, s, hs, hy⟩ := P.mem_aliasEdges.mp hxy
    obtain ⟨hgG, hgo, hgk⟩ := P.mem_declaringOwners.mp hg
    have hsurv : J.apply m S D ∈ P.declaringOwners (mergeBy R J m S D G) :=
      P.mem_declaringOwners.mpr ⟨survivor_mem H, isOwner_survivor H, by
        have : kindOf (J.apply m S D) = kindOf D := by unfold kindOf; rw [H.merger.dir]
        rw [this]; exact H.dstAlias⟩
    by_cases hS : g = S
    · subst hS
      obtain ⟨s', hs', hk⟩ := (H.merger.alias (σa (P.key s))).mpr
        ⟨s, List.mem_append_right _ hs, rfl⟩
      exact ⟨_, hsurv, by rw [claim_survivor H, ← hx, H.srcKey], s', hs', by rw [hk, hy]⟩
    by_cases hD : g = D
    · subst hD
      obtain ⟨s', hs', hk⟩ := (H.merger.alias (σa (P.key s))).mpr
        ⟨s, List.mem_append_left _ hs, rfl⟩
      exact ⟨_, hsurv, by rw [claim_survivor H, ← hx, H.dstKey], s', hs', by rw [hk, hy]⟩
    have ho : g ∈ others S D G := mem_others.mpr ⟨hgG, hS, hD⟩
    have hkind : kindOf (R.apply m g) = kindOf g := by unfold kindOf; rw [H.laws.dir g ho]
    refine ⟨R.apply m g, P.mem_declaringOwners.mpr ⟨other_mem ho, by rw [isOwner_other H ho]; exact hgo,
      by rw [hkind]; exact hgk⟩, by rw [claim_apply H.laws ho]; simp only [σp]; rw [hx], m s, ?_, ?_⟩
    · rw [H.laws.alias g ho]; exact List.mem_map_of_mem hs
    · rw [H.alias g ho s hs, hy]

/-- **MG3 (alias edges)**, round 7's single-map form (`σa = σ`). -/
theorem aliasEdges_merge7 (H : MergeBase P R J m σ S D G) (e : K × K) :
    e ∈ P.aliasEdges (mergeBy R J m S D G) ↔ e ∈ Reach.mapE σ (P.aliasEdges G) :=
  aliasEdges_split H e

/-- **MG3b (alias classes join).** Every key in the source's or the target's
alias class is, after the merge, in the merged page's class. -/
theorem class_join (H : MergeBase P R J m σ S D G) {x : K}
    (hx : cls (P.aliasEdges G) (P.claim S).2 x ∨ cls (P.aliasEdges G) (P.claim D).2 x) :
    cls (P.aliasEdges (mergeBy R J m S D G)) (P.claim D).2 (σ x) := by
  unfold cls
  rw [Reach.congr (aliasEdges_merge7 H)]
  rcases hx with hx | hx
  · have := Reach.map_fwd σ hx; rwa [H.srcKey] at this
  · have := Reach.map_fwd σ hx; rwa [H.dstKey] at this

/-! ## Blocks -/

theorem occAt_survivor (H : MergeBase P R J m σ S D G σa) (i : Nat) :
    P.occAt (mergeBy R J m S D G) (D.path, i) = (blockOcc P (J.apply m S D) i).getD [] := by
  have h := occAt_some (P := P) (i := i) H.wf' (survivor_mem H)
  rw [H.merger.path] at h
  rw [h]; rfl

theorem occAt_other (H : MergeBase P R J m σ S D G σa) {f : File} (hf : f ∈ others S D G) (i : Nat) :
    P.occAt (mergeBy R J m S D G) ((R.apply m f).path, i) =
      (P.occAt G (f.path, i)).map (rwOcc R.covers m) := by
  rw [occAt_some H.wf' (other_mem hf), occAt_some H.wf (others_sub hf)]
  have h := congrArg (fun l => l[i]?) (H.laws.occ f hf)
  simp only [List.getElem?_map] at h
  rw [← h]
  cases (P.outline f.format f.text)[i]? <;> rfl

theorem occAt_eq_blockOcc {f : File} (hw : WF G) (hf : f ∈ G) (i : Nat) :
    P.occAt G (f.path, i) = (blockOcc P f i).getD [] := by
  rw [occAt_some hw hf]; rfl

theorem path_ne {f g : File} (hw : WF G) (hf : f ∈ G) (hg : g ∈ G) (h : f ≠ g) : f.path ≠ g.path :=
  fun e => h (eq_of_path_eq hw hf hg e)

/-- The image relation of blocks: a source or target block goes to every
survivor block that has it as an origin; any other block goes to its rename
image. -/
def Img (R : CM.Rewriter) (J : Merger) (m : String → String) (S D : File)
    (G : Graph) (b b' : BlockId) : Prop :=
  (b.1 = S.path ∧ b'.1 = D.path ∧ (true, b.2) ∈ J.origin m S D b'.2) ∨
  (b.1 = D.path ∧ b'.1 = D.path ∧ (false, b.2) ∈ J.origin m S D b'.2) ∨
  (b.1 ≠ S.path ∧ b.1 ≠ D.path ∧ b' = blockMap R m G b)

theorem mem_blocks_merge7 (H : MergeBase P R J m σ S D G σa) (b' : BlockId) :
    b' ∈ P.blocks (mergeBy R J m S D G) ↔
      (b'.1 = D.path ∧ b'.2 < (P.outline (J.apply m S D).format (J.apply m S D).text).length) ∨
      ∃ f ∈ others S D G, P.isOwner G f = true ∧ b'.1 = (R.apply m f).path ∧
        b'.2 < (P.outline f.format f.text).length := by
  rw [P.mem_blocks]
  constructor
  · rintro ⟨f', hf', ho, hp, hi⟩
    rcases mem_mergeBy.mp hf' with ⟨_, _, rfl⟩ | ⟨f, hf, rfl⟩
    · exact Or.inl ⟨by rw [← hp, H.merger.path], hi⟩
    · refine Or.inr ⟨f, hf, by rw [← isOwner_other H hf]; exact ho, hp.symm, ?_⟩
      rw [← outline_length_rename H.laws hf]; exact hi
  · rintro (⟨hp, hi⟩ | ⟨f, hf, ho, hp, hi⟩)
    · exact ⟨_, survivor_mem H, isOwner_survivor H, by rw [H.merger.path, hp], hi⟩
    · exact ⟨_, other_mem hf, by rw [isOwner_other H hf]; exact ho, hp.symm,
        by rw [outline_length_rename H.laws hf]; exact hi⟩

/-- **MG4 (occurrence level only).** Every block of the before-graph, in
particular every source block, has an image block in the after-graph. This
does NOT say body blocks stay separate: under `MergeOpHyps` alone a merger may
collapse several blocks into one (verifier r7). The block-preservation result
is `Merge7b.bodies_kept`, under the body law. -/
theorem no_block_lost (H : MergeBase P R J m σ S D G σa) {b : BlockId} (hb : b ∈ P.blocks G) :
    ∃ b' ∈ P.blocks (mergeBy R J m S D G), Img R J m S D G b b' := by
  obtain ⟨f, hf, ho, hp, hi⟩ := P.mem_blocks.mp hb
  obtain ⟨π, j⟩ := b
  simp only at hp hi; subst hp
  have hlen : ∀ i, (blockOcc P (J.apply m S D) i).isSome = true →
      (D.path, i) ∈ P.blocks (mergeBy R J m S D G) := fun i hi =>
    (mem_blocks_merge7 H _).mpr (Or.inl ⟨rfl, blockOcc_isSome.mp hi⟩)
  by_cases hS : f = S
  · subst hS
    have hx : originOcc P f D (true, j) = some _ := (Option.isSome_iff_exists.mp
      (blockOcc_isSome.mpr hi)).choose_spec
    obtain ⟨⟨i, hxi, hiS⟩, _⟩ := H.merger.cover _ _ hx
    exact ⟨_, hlen i hiS, Or.inl ⟨rfl, rfl, hxi⟩⟩
  by_cases hD : f = D
  · subst hD
    have hx : originOcc P S f (false, j) = some _ := (Option.isSome_iff_exists.mp
      (blockOcc_isSome.mpr hi)).choose_spec
    obtain ⟨⟨i, hxi, hiS⟩, _⟩ := H.merger.cover _ _ hx
    exact ⟨_, hlen i hiS, Or.inr (Or.inl ⟨rfl, rfl, hxi⟩)⟩
  · have ho' : f ∈ others S D G := mem_others.mpr ⟨hf, hS, hD⟩
    refine ⟨((R.apply m f).path, j), (mem_blocks_merge7 H _).mpr
      (Or.inr ⟨f, ho', ho, rfl, hi⟩), Or.inr (Or.inr ⟨path_ne H.wf hf H.src hS,
        path_ne H.wf hf H.dst hD, ?_⟩)⟩
    rw [blockMap_eq H.wf hf]

/-- **MG4b (no block is invented).** Every after-graph block is the image of an
old block. -/
theorem no_block_new (H : MergeBase P R J m σ S D G σa) {b' : BlockId}
    (hb' : b' ∈ P.blocks (mergeBy R J m S D G)) : ∃ b ∈ P.blocks G, Img R J m S D G b b' := by
  rcases (mem_blocks_merge7 H _).mp hb' with ⟨hp, hi⟩ | ⟨f, hf, ho, hp, hi⟩
  · obtain ⟨x, hx⟩ := List.exists_mem_of_ne_nil _ (H.merger.nonempty _ hi)
    have hv := H.merger.valid _ hi x hx
    obtain ⟨c, j⟩ := x
    cases c
    · refine ⟨(D.path, j), P.mem_blocks.mpr ⟨D, H.dst, isOwner_dst H, rfl,
        blockOcc_isSome.mp hv⟩, Or.inr (Or.inl ⟨rfl, hp, hx⟩)⟩
    · refine ⟨(S.path, j), P.mem_blocks.mpr ⟨S, H.src, isOwner_src H, rfl,
        blockOcc_isSome.mp hv⟩, Or.inl ⟨rfl, hp, hx⟩⟩
  · have hf' := mem_others.mp hf
    refine ⟨(f.path, b'.2), P.mem_blocks.mpr ⟨f, hf'.1, ho, rfl, hi⟩,
      Or.inr (Or.inr ⟨path_ne H.wf hf'.1 H.src hf'.2.1, path_ne H.wf hf'.1 H.dst hf'.2.2, ?_⟩)⟩
    rw [blockMap_eq H.wf hf'.1, ← hp]

/-! ## References -/

theorem aliasClass_merge7 (H : MergeBase P R J m σ S D G) (k t : K) :
    P.aliasClass (mergeBy R J m S D G) k t ↔ cls (Reach.mapE σ (P.aliasEdges G)) k t := by
  unfold aliasClass cls
  exact Reach.congr (aliasEdges_merge7 H)

/-- **MG5a (exact references of survivor blocks).** -/
theorem references_survivor_in (H : MergeBase P R J m σ S D G σa) {Ea : List (K × K)}
    (hA : ∀ k t, P.aliasClass (mergeBy R J m S D G) k t ↔ cls Ea k t) (i : Nat) (t : Page K) :
    P.References (mergeBy R J m S D G) (D.path, i) t ↔
      ∃ o ∈ (blockOcc P (J.apply m S D) i).getD [], ∃ u ∈ P.occClosure o.2,
        cls Ea (P.key u) t.2 := by
  rw [P.references_iff_keys]
  simp only [P.mem_refNames, occAt_survivor H, hA]
  constructor
  · rintro ⟨u, ⟨o, ho, hu⟩, hc⟩; exact ⟨o, ho, u, hu, hc⟩
  · rintro ⟨o, ho, u, hu, hc⟩; exact ⟨u, ⟨o, ho, hu⟩, hc⟩

theorem references_survivor (H : MergeBase P R J m σ S D G) (i : Nat) (t : Page K) :
    P.References (mergeBy R J m S D G) (D.path, i) t ↔
      ∃ o ∈ (blockOcc P (J.apply m S D) i).getD [], ∃ u ∈ P.occClosure o.2,
        cls (Reach.mapE σ (P.aliasEdges G)) (P.key u) t.2 :=
  references_survivor_in H (aliasClass_merge7 H) i t

/-- **MG5b (exact references of every other block)**, as `references_merge_exact`. -/
theorem references_other_in (H : MergeBase P R J m σ S D G σa) {Ea : List (K × K)}
    (hA : ∀ k t, P.aliasClass (mergeBy R J m S D G) k t ↔ cls Ea k t) {f : File} (hf : f ∈ others S D G)
    (i : Nat) (t : Page K) :
    P.References (mergeBy R J m S D G) ((R.apply m f).path, i) t ↔
      ∃ o ∈ P.occAt G (f.path, i), ∃ u ∈ P.occClosure (rwOcc R.covers m o).2,
        cls Ea (P.key u) t.2 := by
  rw [P.references_iff_keys]
  simp only [P.mem_refNames, occAt_other H hf, hA, List.mem_map]
  constructor
  · rintro ⟨u, ⟨o', ⟨o, ho, rfl⟩, hu⟩, hc⟩; exact ⟨o, ho, u, hu, hc⟩
  · rintro ⟨o, ho, u, hu, hc⟩; exact ⟨u, ⟨_, ⟨o, ho, rfl⟩, hu⟩, hc⟩

theorem references_other (H : MergeBase P R J m σ S D G) {f : File} (hf : f ∈ others S D G)
    (i : Nat) (t : Page K) :
    P.References (mergeBy R J m S D G) ((R.apply m f).path, i) t ↔
      ∃ o ∈ P.occAt G (f.path, i), ∃ u ∈ P.occClosure (rwOcc R.covers m o).2,
        cls (Reach.mapE σ (P.aliasEdges G)) (P.key u) t.2 :=
  references_other_in H (aliasClass_merge7 H) hf i t

/-- **MG5c (survivor references come from origins).** A survivor block
references only what one of its origin blocks' rewritten occurrences reaches in
the σ-mapped alias graph. -/
theorem references_survivor_origin_in (H : MergeBase P R J m σ S D G σa) {Ea : List (K × K)}
    (hA : ∀ k t, P.aliasClass (mergeBy R J m S D G) k t ↔ cls Ea k t) {i : Nat} {t : Page K}
    (h : P.References (mergeBy R J m S D G) (D.path, i) t) :
    ∃ x ∈ J.origin m S D i, ∃ os, originOcc P S D x = some os ∧ ∃ o ∈ os,
      ∃ u ∈ P.occClosure (rwOcc R.covers m o).2, cls Ea (P.key u) t.2 := by
  obtain ⟨o, ho, u, hu, hc⟩ := (references_survivor_in H hA i t).mp h
  cases hb : blockOcc P (J.apply m S D) i with
  | none => rw [hb] at ho; cases ho
  | some os =>
    rw [hb] at ho
    obtain ⟨x, hx, os', hos', o', ho', e⟩ := H.merger.occ_sub i os hb o ho
    exact ⟨x, hx, os', hos', o', ho', u, by rw [e]; exact hu, hc⟩

theorem references_survivor_origin (H : MergeBase P R J m σ S D G) {i : Nat} {t : Page K}
    (h : P.References (mergeBy R J m S D G) (D.path, i) t) :
    ∃ x ∈ J.origin m S D i, ∃ os, originOcc P S D x = some os ∧ ∃ o ∈ os,
      ∃ u ∈ P.occClosure (rwOcc R.covers m o).2, cls (Reach.mapE σ (P.aliasEdges G)) (P.key u) t.2 :=
  references_survivor_origin_in H (aliasClass_merge7 H) h

/-- The namespace condition of `references_merge_forward`, per block. -/
def NsForward (P : Params K) (R : CM.Rewriter) (m : String → String) (σ : K → K)
    (os : List (OccKind × String)) : Prop :=
  ∀ o ∈ os, ∀ u ∈ P.occClosure o.2, ∃ v ∈ P.occClosure (rwOcc R.covers m o).2, P.key v = σ (P.key u)

/-- Round 8: the transport step of MG6 with its key-level premise exposed.
If every rewritten occurrence of block `b` whose closure reached `t` (before)
reaches the key `k'` in the after-graph's alias graph, `b` has an image block
that references `k'`. MG6 (`references_forward`) is the case `k' = σ t`; the
alias-owner merge (`Merge8`) uses it with a key map for the alias graph that
differs from the text's. -/
theorem references_forward_cls_in (H : MergeBase P R J m σ S D G σa) {Ea : List (K × K)}
    (hA : ∀ k t, P.aliasClass (mergeBy R J m S D G) k t ↔ cls Ea k t) {b : BlockId} (hb : b ∈ P.blocks G)
    {t : Page K} {k' : K}
    (hstep : ∀ o ∈ P.occAt G b, ∀ u ∈ P.occClosure o.2, cls (P.aliasEdges G) (P.key u) t.2 →
      ∃ v ∈ P.occClosure (rwOcc R.covers m o).2, cls Ea (P.key v) k')
    (hr : P.References G b t) :
    ∃ b' ∈ P.blocks (mergeBy R J m S D G), Img R J m S D G b b' ∧
      P.References (mergeBy R J m S D G) b' (t.1, k') := by
  obtain ⟨u, hu, hc⟩ := P.references_iff_keys.mp hr
  obtain ⟨o, ho, hu⟩ := P.mem_refNames.mp hu
  obtain ⟨v, hv, hcls⟩ := hstep o ho u hu hc
  obtain ⟨f, hf, hfo, hp, hi⟩ := P.mem_blocks.mp hb
  obtain ⟨π, j⟩ := b
  simp only at hp hi; subst hp
  rw [occAt_eq_blockOcc H.wf hf] at ho
  -- the survivor case, for an origin `x` of `f`'s block
  have surv : ∀ x, originOcc P S D x = blockOcc P f j →
      (∀ i, x ∈ J.origin m S D i → Img R J m S D G (f.path, j) (D.path, i)) →
      ∃ b' ∈ P.blocks (mergeBy R J m S D G), Img R J m S D G (f.path, j) b' ∧
        P.References (mergeBy R J m S D G) b' (t.1, k') := by
    intro x hx himg
    cases hbo : blockOcc P f j with
    | none => rw [hbo] at ho; cases ho
    | some os =>
      rw [hbo] at ho hx
      obtain ⟨_, hcov⟩ := H.merger.cover x os hx
      obtain ⟨i, hxi, os'', hos'', hmem⟩ := hcov o ho
      refine ⟨(D.path, i), (mem_blocks_merge7 H _).mpr (Or.inl ⟨rfl, blockOcc_isSome.mp
        (by rw [hos'']; rfl)⟩), himg i hxi, ?_⟩
      exact (references_survivor_in H hA i _).mpr ⟨_, by rw [hos'']; exact hmem, v, hv, hcls⟩
  by_cases hS : f = S
  · subst hS
    exact surv (true, j) rfl (fun i hxi => Or.inl ⟨rfl, rfl, hxi⟩)
  by_cases hD : f = D
  · subst hD
    exact surv (false, j) rfl (fun i hxi => Or.inr (Or.inl ⟨rfl, rfl, hxi⟩))
  · have ho' : f ∈ others S D G := mem_others.mpr ⟨hf, hS, hD⟩
    refine ⟨((R.apply m f).path, j), (mem_blocks_merge7 H _).mpr (Or.inr ⟨f, ho', hfo, rfl, hi⟩),
      Or.inr (Or.inr ⟨path_ne H.wf hf H.src hS, path_ne H.wf hf H.dst hD, by
        rw [blockMap_eq H.wf hf]⟩), ?_⟩
    rw [← occAt_eq_blockOcc H.wf hf] at ho
    exact (references_other_in H hA ho' j _).mpr ⟨o, ho, v, hv, hcls⟩

theorem references_forward_cls (H : MergeBase P R J m σ S D G) {b : BlockId} (hb : b ∈ P.blocks G)
    {t : Page K} {k' : K}
    (hstep : ∀ o ∈ P.occAt G b, ∀ u ∈ P.occClosure o.2, cls (P.aliasEdges G) (P.key u) t.2 →
      ∃ v ∈ P.occClosure (rwOcc R.covers m o).2, cls (Reach.mapE σ (P.aliasEdges G)) (P.key v) k')
    (hr : P.References G b t) :
    ∃ b' ∈ P.blocks (mergeBy R J m S D G), Img R J m S D G b b' ∧
      P.References (mergeBy R J m S D G) b' (t.1, k') :=
  references_forward_cls_in H (aliasClass_merge7 H) hb hstep hr

/-- **MG6 (references follow the merge).** Under the forward namespace
condition, a block of the before-graph that references `t` has an image block
that references `σ t`. With `t` the source page: references to the source now
refer to the target (`references_to_source`). -/
theorem references_forward (H : MergeBase P R J m σ S D G) {b : BlockId} (hb : b ∈ P.blocks G)
    {t : Page K} (hns : NsForward P R m σ (P.occAt G b)) (hr : P.References G b t) :
    ∃ b' ∈ P.blocks (mergeBy R J m S D G), Img R J m S D G b b' ∧
      P.References (mergeBy R J m S D G) b' (σp σ t) :=
  references_forward_cls H hb (fun o ho u hu hc => by
    obtain ⟨v, hv, he⟩ := hns o ho u hu
    exact ⟨v, hv, by rw [he]; exact Reach.map_fwd σ hc⟩) hr

/-- Round 8: whether a block references a page depends only on the page's key
(references are read through alias classes of keys), so a journal and a page of
the same key have the same Linked References. -/
theorem references_key {G : Graph} {b : BlockId} {t t' : Page K} (h : t.2 = t'.2) :
    P.References G b t ↔ P.References G b t' := by
  unfold References; rw [h]

theorem references_to_source (H : MergeBase P R J m σ S D G) {b : BlockId} (hb : b ∈ P.blocks G)
    (hns : NsForward P R m σ (P.occAt G b)) (hr : P.References G b (P.claim S)) :
    ∃ b' ∈ P.blocks (mergeBy R J m S D G), Img R J m S D G b b' ∧
      P.References (mergeBy R J m S D G) b' (P.claim D) := by
  obtain ⟨b', hb', hi, hr'⟩ := references_forward H hb hns hr
  exact ⟨b', hb', hi, (references_key (t := σp σ (P.claim S)) (t' := P.claim D) H.srcKey).mp hr'⟩

/-- **MG7 (Linked References of the merged page).** Forward: every block that
referenced the source or the target has an image that references the merged
page. Backward (exact): every after-graph block referencing the merged page is
the image of an old block one of whose rewritten occurrences reaches the
target key in the σ-mapped alias graph. Round 8b: the backward half is
`linked_references_back_in`, stated for any alias-class transport `hA`. -/
theorem linked_references_back_in (H : MergeBase P R J m σ S D G σa) {Ea : List (K × K)}
    (hA : ∀ k t, P.aliasClass (mergeBy R J m S D G) k t ↔ cls Ea k t) :
    ∀ b' ∈ P.blocks (mergeBy R J m S D G), P.References (mergeBy R J m S D G) b' (P.claim D) →
      ∃ b ∈ P.blocks G, Img R J m S D G b b' ∧ ∃ o ∈ P.occAt G b,
        ∃ u ∈ P.occClosure (rwOcc R.covers m o).2, cls Ea (P.key u) (P.claim D).2 := by
  intro b' hb' hr
  rcases (mem_blocks_merge7 H _).mp hb' with ⟨hp, hi⟩ | ⟨f, hf, ho, hp, hi⟩
  · obtain ⟨π, i⟩ := b'
    simp only at hp hi; subst hp
    obtain ⟨x, hx, os, hos, o, ho, u, hu, hc⟩ := references_survivor_origin_in H hA hr
    obtain ⟨c, j⟩ := x
    cases c
    · have hj : j < (P.outline D.format D.text).length := blockOcc_isSome.mp (by
        change (blockOcc P D j).isSome = true; simp only [originOcc] at hos; rw [hos]; rfl)
      refine ⟨(D.path, j), P.mem_blocks.mpr ⟨D, H.dst, isOwner_dst H, rfl, hj⟩,
        Or.inr (Or.inl ⟨rfl, rfl, hx⟩), o, ?_, u, hu, hc⟩
      rw [occAt_eq_blockOcc H.wf H.dst]; simp only [originOcc] at hos; rw [hos]; exact ho
    · have hj : j < (P.outline S.format S.text).length := blockOcc_isSome.mp (by
        change (blockOcc P S j).isSome = true; simp only [originOcc] at hos; rw [hos]; rfl)
      refine ⟨(S.path, j), P.mem_blocks.mpr ⟨S, H.src, isOwner_src H, rfl, hj⟩,
        Or.inl ⟨rfl, rfl, hx⟩, o, ?_, u, hu, hc⟩
      rw [occAt_eq_blockOcc H.wf H.src]; simp only [originOcc] at hos; rw [hos]; exact ho
  · have hf' := mem_others.mp hf
    obtain ⟨π, i⟩ := b'
    simp only at hp hi; subst hp
    obtain ⟨o, ho', u, hu, hc⟩ := (references_other_in H hA hf i _).mp hr
    exact ⟨(f.path, i), P.mem_blocks.mpr ⟨f, hf'.1, ho, rfl, hi⟩,
      Or.inr (Or.inr ⟨path_ne H.wf hf'.1 H.src hf'.2.1, path_ne H.wf hf'.1 H.dst hf'.2.2,
        by rw [blockMap_eq H.wf hf'.1]⟩), o, ho', u, hu, hc⟩

theorem linked_references_merge7 (H : MergeBase P R J m σ S D G) :
    (∀ b ∈ P.blocks G, NsForward P R m σ (P.occAt G b) →
      (P.References G b (P.claim S) ∨ P.References G b (P.claim D)) →
      ∃ b' ∈ P.blocks (mergeBy R J m S D G), Img R J m S D G b b' ∧
        P.References (mergeBy R J m S D G) b' (P.claim D)) ∧
    (∀ b' ∈ P.blocks (mergeBy R J m S D G), P.References (mergeBy R J m S D G) b' (P.claim D) →
      ∃ b ∈ P.blocks G, Img R J m S D G b b' ∧ ∃ o ∈ P.occAt G b,
        ∃ u ∈ P.occClosure (rwOcc R.covers m o).2,
          cls (Reach.mapE σ (P.aliasEdges G)) (P.key u) (P.claim D).2) := by
  refine ⟨fun b hb hns hr => ?_, linked_references_back_in H (aliasClass_merge7 H)⟩
  · rcases hr with hr | hr
    · exact references_to_source H hb hns hr
    · obtain ⟨b', hb', hi, hr'⟩ := references_forward H hb hns hr
      exact ⟨b', hb', hi, (references_key (t := σp σ (P.claim D)) (t' := P.claim D) H.dstKey).mp hr'⟩

/-! ## Pages -/

/-- Namespace parents after a merge: the source file's are gone. -/
def keptNameKeys (P : Params K) (R : CM.Rewriter) (m : String → String) (S : File) (G : Graph) : List K :=
  (G.filter (fun f => decide (f ≠ S))).flatMap (fun f => (P.fileNsNames (R.apply m f)).map P.key)

theorem claimedKey_merge7 (H : MergeBase P R J m σ S D G σa) (k' : K) :
    P.claimedKey (mergeBy R J m S D G) k' = true ↔ ∃ k, P.claimedKey G k = true ∧ σ k = k' := by
  rw [P.claimedKey_iff]
  constructor
  · rintro ⟨f', hf', e⟩
    rcases mem_mergeBy.mp hf' with ⟨_, _, rfl⟩ | ⟨f, hf, rfl⟩
    · rw [claim_survivor H] at e
      exact ⟨(P.claim D).2, P.claimedKey_iff.mpr ⟨D, H.dst, rfl⟩, by rw [H.dstKey, e]⟩
    · rw [claim_apply H.laws hf] at e
      exact ⟨(P.claim f).2, P.claimedKey_iff.mpr ⟨f, others_sub hf, rfl⟩, e⟩
  · rintro ⟨k, hk, rfl⟩
    obtain ⟨f, hf, rfl⟩ := P.claimedKey_iff.mp hk
    by_cases hS : f = S
    · subst hS; exact ⟨_, survivor_mem H, by rw [claim_survivor H, H.srcKey]⟩
    by_cases hD : f = D
    · subst hD; exact ⟨_, survivor_mem H, by rw [claim_survivor H, H.dstKey]⟩
    have ho : f ∈ others S D G := mem_others.mpr ⟨hf, hS, hD⟩
    exact ⟨_, other_mem ho, by rw [claim_apply H.laws ho]; rfl⟩

theorem refKeys_merge7 (H : MergeBase P R J m σ S D G σa) (k : K) :
    k ∈ P.refKeys (mergeBy R J m S D G) ↔ k ∈ rewrittenRefKeys P R m G := by
  rw [P.mem_refKeys]
  simp only [rewrittenRefKeys, List.mem_flatMap, List.mem_map]
  constructor
  · rintro ⟨b', hb', u, hu0, he⟩
    obtain ⟨o', ho', hu⟩ := P.mem_refNames.mp hu0
    clear hu0
    rcases (mem_blocks_merge7 H _).mp hb' with ⟨hp, hi⟩ | ⟨f, hf, ho, hp, hi⟩
    · obtain ⟨π, i⟩ := b'
      simp only at hp hi; subst hp
      rw [occAt_survivor H] at ho'
      cases hb : blockOcc P (J.apply m S D) i with
      | none => rw [hb] at ho'; cases ho'
      | some os =>
        rw [hb] at ho'
        obtain ⟨x, hx, os', hos', o, ho, e⟩ := H.merger.occ_sub i os hb o' ho'
        obtain ⟨c, j⟩ := x
        cases c
        · simp only [originOcc] at hos'
          have hj : j < (P.outline D.format D.text).length := blockOcc_isSome.mp (by rw [hos']; rfl)
          refine ⟨(D.path, j), P.mem_blocks.mpr ⟨D, H.dst, isOwner_dst H, rfl, hj⟩, o, ?_, u,
            by rw [e]; exact hu, he⟩
          rw [occAt_eq_blockOcc H.wf H.dst, hos']; exact ho
        · simp only [originOcc] at hos'
          have hj : j < (P.outline S.format S.text).length := blockOcc_isSome.mp (by rw [hos']; rfl)
          refine ⟨(S.path, j), P.mem_blocks.mpr ⟨S, H.src, isOwner_src H, rfl, hj⟩, o, ?_, u,
            by rw [e]; exact hu, he⟩
          rw [occAt_eq_blockOcc H.wf H.src, hos']; exact ho
    · have hf' := mem_others.mp hf
      obtain ⟨π, i⟩ := b'
      simp only at hp hi; subst hp
      rw [occAt_other H hf] at ho'
      obtain ⟨o, hoc, rfl⟩ := List.mem_map.mp ho'
      exact ⟨(f.path, i), P.mem_blocks.mpr ⟨f, hf'.1, ho, rfl, hi⟩, o, hoc, u, hu, he⟩
  · rintro ⟨b, hb, o, ho, u, hu, he⟩
    obtain ⟨f, hf, hfo, hp, hi⟩ := P.mem_blocks.mp hb
    obtain ⟨π, j⟩ := b
    simp only at hp hi; subst hp
    rw [occAt_eq_blockOcc H.wf hf] at ho
    have surv : ∀ x, originOcc P S D x = blockOcc P f j →
        ∃ b' ∈ P.blocks (mergeBy R J m S D G), ∃ u ∈ P.refNames (mergeBy R J m S D G) b', P.key u = k := by
      intro x hx
      cases hbo : blockOcc P f j with
      | none => rw [hbo] at ho; cases ho
      | some os =>
        rw [hbo] at ho hx
        obtain ⟨_, hcov⟩ := H.merger.cover x os hx
        obtain ⟨i, _, os'', hos'', hmem⟩ := hcov o ho
        refine ⟨(D.path, i), (mem_blocks_merge7 H _).mpr (Or.inl ⟨rfl, blockOcc_isSome.mp
          (by rw [hos'']; rfl)⟩), u, P.mem_refNames.mpr ⟨_, ?_, hu⟩, he⟩
        rw [occAt_survivor H, hos'']; exact hmem
    by_cases hS : f = S
    · subst hS; exact surv (true, j) rfl
    by_cases hD : f = D
    · subst hD; exact surv (false, j) rfl
    · have ho' : f ∈ others S D G := mem_others.mpr ⟨hf, hS, hD⟩
      rw [← occAt_eq_blockOcc H.wf hf] at ho
      refine ⟨((R.apply m f).path, j), (mem_blocks_merge7 H _).mpr (Or.inr ⟨f, ho', hfo, rfl, hi⟩),
        u, P.mem_refNames.mpr ⟨_, ?_, hu⟩, he⟩
      rw [occAt_other H ho']; exact List.mem_map_of_mem ho

theorem nameKeys_merge7 (H : MergeBase P R J m σ S D G σa) (k : K) :
    k ∈ P.nameKeys (mergeBy R J m S D G) ↔ k ∈ keptNameKeys P R m S G := by
  rw [P.mem_nameKeys]
  simp only [keptNameKeys, List.mem_flatMap, List.mem_filter, decide_eq_true_eq, List.mem_map]
  constructor
  · rintro ⟨f', hf', s, hs, rfl⟩
    rcases mem_mergeBy.mp hf' with ⟨_, _, rfl⟩ | ⟨f, hf, rfl⟩
    · obtain ⟨s', hs', e⟩ := List.mem_map.mp ((H.merger.names _).mp (List.mem_map_of_mem hs))
      exact ⟨D, ⟨H.dst, fun e => H.ne e.symm⟩, s', hs', e⟩
    · exact ⟨f, ⟨others_sub hf, (mem_others.mp hf).2.1⟩, s, hs, rfl⟩
  · rintro ⟨f, ⟨hf, hfS⟩, s, hs, rfl⟩
    by_cases hD : f = D
    · subst hD
      obtain ⟨s', hs', e⟩ := List.mem_map.mp ((H.merger.names _).mpr (List.mem_map_of_mem hs))
      exact ⟨_, survivor_mem H, s', hs', e⟩
    · exact ⟨_, other_mem (mem_others.mpr ⟨hf, hfS, hD⟩), s, hs, rfl⟩

/-- **MG8 (exact page universe after a merge).** All right-hand data is old
data: real pages are the σ-image (the source is now the target); a
reference-only page is a rewritten old seed that no file now claims and no
alias now declares. The source file's own namespace parents are no longer
seeds (`keptNameKeys`). -/
theorem pages_merge7 (H : MergeOpHyps P R J m σ S D G) (p : Page K) :
    p ∈ Decided.pages P (mergeBy R J m S D G) ↔
      (∃ q, P.real G q = true ∧ σp σ q = p) ∨
      (p = (P.refKind p.2, p.2) ∧
        (p.2 ∈ rewrittenRefKeys P R m G ∨ p.2 ∈ keptNameKeys P R m S G) ∧
        ¬ (∃ k, P.claimedKey G k = true ∧ σ k = p.2) ∧
        ¬ (∃ e ∈ P.aliasEdges G, σ e.2 = p.2)) := by
  have hd : P.declaredKey (mergeBy R J m S D G) p.2 = true ↔
      ∃ e ∈ P.aliasEdges G, σ e.2 = p.2 := by
    rw [P.declaredKey_iff]
    simp only [aliasEdges_merge7 H.toMergeBase, Reach.mapE, List.mem_map]
    constructor
    · rintro ⟨e', ⟨e, he, rfl⟩, hk⟩; exact ⟨e, he, hk⟩
    · rintro ⟨e, he, hk⟩; exact ⟨_, ⟨e, he, rfl⟩, hk⟩
  have hs : p.2 ∈ P.pageSeedKeys (mergeBy R J m S D G) ↔
      (p.2 ∈ rewrittenRefKeys P R m G ∨ p.2 ∈ keptNameKeys P R m S G) := by
    simp only [pageSeedKeys, List.mem_append, refKeys_merge7 H.toMergeBase, nameKeys_merge7 H.toMergeBase]
  simp only [Decided.pages, P.mem_pagesR, real_merge7 H, hs, Bool.true_and, Bool.eq_false_iff,
    ne_eq, claimedKey_merge7 H.toMergeBase, hd]

/-! ## The decided merge operation -/

section Decided7
variable (P)

/-- The merge source: the only page file claiming `old`'s key. -/
def mergeSource (G : Graph) (old : String) : Option File :=
  match P.claimants G (.page, P.key old) with
  | [f] => some f
  | _ => none

/-- The survivor (Tine `plan_rename`: the sole file claiming `new`, directly or
as an alias of its owner). A same-key `new` is a rename, never a merge. -/
def mergeTarget (G : Graph) (old new : String) : Option File :=
  if P.key new = P.key old then none else
  match G.filter (fun f => decide ((P.claim f).2 = P.key new)) with
  | [f] => some f
  | [] => match (P.declaringOwners G).filter (fun f => P.declaresKey f (P.key new)) with
    | [f] => some f
    | _ => none
  | _ => none

/-- Rule 6 for merge: the rename guard with the primary collision (`new` is a
real page outside the subtree, the survivor) turned into the merge, and Tine's
format check (`merged_survivor` refuses files of different formats). Every other
decided rename refusal is kept: descendants may not land on real pages outside
the subtree, moved keys are not conflicted, distinct moved real keys do not
collide, journal spellings are not lost (CM3-1).
Round 7b: the survivor may be a journal (Martin, M-Q7: allowed; round 8 proves
the theorems for it, `MergeBase`).
Round 8 (Martin, D-20261102-MC M-Q6): merging a page into its own namespace
child is refused: the survivor's name, and the typed name `new` (which differs
from it when `new` is an alias of the survivor), must not lie in `old`'s
namespace subtree. -/
def mergeAllowedTo (G : Graph) (old new : String) (S D : File) : Bool :=
  decide (S ≠ D) && decide (S.format = D.format) &&
  !(nsMatch P.key P.urlHasOrigin old (P.name D)).isSome && !(nsMatch P.key P.urlHasOrigin old new).isSome &&
  G.all (fun g => decide (g = D) ||
    (decide (kindOf g = .page) && (nsMatch P.key P.urlHasOrigin old (P.name g)).isSome) ||
    (P.movedFiles G old).all (fun f => decide (f = S) ||
      decide ((P.claim g).2 ≠ P.key (nsSubst P.key P.urlHasOrigin old new (P.name f))))) &&
  (Decided.movedNames P G old).all (fun s => !P.conflictedB G (P.key s)) &&
  (P.movedFiles G old).all (fun f => (P.movedFiles G old).all (fun g =>
    decide (P.key (nsSubst P.key P.urlHasOrigin old new (P.name f)) =
      P.key (nsSubst P.key P.urlHasOrigin old new (P.name g)) → (P.claim f).2 = (P.claim g).2))) &&
  G.all (fun g => decide (g = D) || decide (g ∈ P.movedFiles G old) ||
    (Decided.movedPageNames P G old).all (fun s => decide (P.key s = P.key old) ||
      decide ((P.claim g).2 ≠ P.key (nsSubst P.key P.urlHasOrigin old new s)))) &&
  (Decided.journalRewriteRisks P G old new).isEmpty

/-- Target selection is separate from the unchanged guard (round 11). -/
def mergeAllowed (G : Graph) (old new : String) : Bool :=
  match mergeSource P G old, mergeTarget P G old new with
  | some S, some D => mergeAllowedTo P G old new S D
  | _, _ => false

/-- **The merge operation** (executable, refusing). -/
def merge (R : Decided.Rewriter) (J : Merger) (G : Graph) (old new : String) : Option Graph :=
  match mergeSource P G old, mergeTarget P G old new with
  | some S, some D =>
    if mergeAllowed P G old new then some (mergeBy R.base J (nsSubst P.key P.urlHasOrigin old new) S D G) else none
  | _, _ => none

end Decided7

theorem mergeSource_spec {old : String} {S : File} (h : mergeSource P G old = some S) :
    S ∈ G ∧ P.claim S = (.page, P.key old) := by
  unfold mergeSource at h
  split at h
  · next f hc =>
    cases h
    have : S ∈ P.claimants G (.page, P.key old) := by rw [hc]; exact List.mem_singleton_self _
    exact P.mem_claimants.mp this
  · cases h

theorem mergeTarget_direct {old new : String} {D : File} (h : mergeTarget P G old new = some D)
    (hk : (P.claim D).2 = P.key new) :
    D ∈ G ∧ P.key new ≠ P.key old ∧ ∀ f ∈ G, (P.claim f).2 = P.key new → f = D := by
  unfold mergeTarget at h
  split at h
  · cases h
  · next hne =>
    split at h
    · next f hc =>
      cases h
      have hmem : ∀ g, g ∈ G.filter (fun f => decide ((P.claim f).2 = P.key new)) ↔ g = D := by
        intro g; rw [hc]; simp
      refine ⟨(List.mem_filter.mp ((hmem D).mpr rfl)).1, hne, fun g hg hgk => (hmem g).mp ?_⟩
      exact List.mem_filter.mpr ⟨hg, by simpa using hgk⟩
    · next hc =>
      split at h
      · next f hc2 =>
        cases h
        have hD : D ∈ P.declaringOwners G := by
          have : D ∈ (P.declaringOwners G).filter (fun f => P.declaresKey f (P.key new)) := by
            rw [hc2]; exact List.mem_singleton_self _
          exact (List.mem_filter.mp this).1
        have hDG := (P.mem_declaringOwners.mp hD).1
        have : D ∈ G.filter (fun f => decide ((P.claim f).2 = P.key new)) :=
          List.mem_filter.mpr ⟨hDG, by simpa using hk⟩
        rw [hc] at this; cases this
      · cases h
    · cases h

/-- **MG0 (the decided merge establishes the structural hypotheses).** An
accepted merge: source and survivor are distinct files of the same format, the
source a page and the only file claiming its key (the survivor too, when it
claims `new` directly), and the result is `mergeBy`. -/
theorem merge_structure (R : Decided.Rewriter) (J : Merger) {old new : String} {G' : Graph}
    (h : merge P R J G old new = some G') :
    ∃ S D, mergeSource P G old = some S ∧ mergeTarget P G old new = some D ∧
      G' = mergeBy R.base J (nsSubst P.key P.urlHasOrigin old new) S D G ∧
      S ∈ G ∧ D ∈ G ∧ S ≠ D ∧ kindOf S = .page ∧ S.format = D.format ∧
      (P.claim S).2 = P.key old ∧
      (∀ f ∈ G, (P.claim f).2 = (P.claim S).2 → f = S) ∧
      ((P.claim D).2 = P.key new → ∀ f ∈ G, (P.claim f).2 = (P.claim D).2 → f = D) := by
  unfold merge at h
  split at h
  · next S D hS hD =>
    split at h
    · next ha =>
      cases h
      have ha' := ha
      unfold mergeAllowed at ha'
      rw [hS, hD] at ha'
      unfold mergeAllowedTo at ha'
      simp only [Bool.and_eq_true, decide_eq_true_eq, List.all_eq_true] at ha'
      obtain ⟨⟨⟨⟨⟨⟨⟨⟨hne, hfmt⟩, _⟩, _⟩, _⟩, hconf⟩, _⟩, _⟩, _⟩ := ha'
      obtain ⟨hSG, hSc⟩ := mergeSource_spec hS
      have hSp : kindOf S = .page := congrArg Prod.fst hSc
      have hSk : (P.claim S).2 = P.key old := congrArg Prod.snd hSc
      refine ⟨S, D, hS, hD, rfl, hSG, ?_, hne, hSp, hfmt, hSk, ?_, ?_⟩
      · unfold mergeTarget at hD
        split at hD
        · cases hD
        · split at hD
          · next f hc =>
            rw [← Option.some.inj hD]
            have : f ∈ G.filter (fun g => decide ((P.claim g).2 = P.key new)) := by
              rw [hc]; exact List.mem_singleton_self _
            exact (List.mem_filter.mp this).1
          · split at hD
            · next f hc2 =>
              rw [← Option.some.inj hD]
              have : f ∈ (P.declaringOwners G).filter (fun g => P.declaresKey g (P.key new)) := by
                rw [hc2]; exact List.mem_singleton_self _
              exact (P.mem_declaringOwners.mp (List.mem_filter.mp this).1).1
            · cases hD
          · cases hD
      · intro f hf hfk
        refine Classical.byContradiction fun hfS => ?_
        have hc := hconf old (CaseRename7.old_mem_movedNames P)
        rw [Bool.not_eq_true', ← Bool.not_eq_true, P.conflictedB_iff] at hc
        exact hc ⟨f, hf, S, hSG, hfS, by rw [hfk, hSk], hSk⟩
      · intro hk f hf hfk
        exact (mergeTarget_direct hD hk).2.2 f hf (by rw [hfk, hk])
    · cases h
  · cases h

/-- **MG0b (round 8, M-Q6).** An accepted decided merge never has its survivor,
or the typed name, inside the source's namespace subtree; so the rename map
leaves the survivor's name unchanged. -/
theorem merge_outside_subtree (R : Decided.Rewriter) (J : Merger) {old new : String} {G' : Graph}
    (h : merge P R J G old new = some G') :
    ∃ D, mergeTarget P G old new = some D ∧ nsMatch P.key P.urlHasOrigin old (P.name D) = none ∧
      nsMatch P.key P.urlHasOrigin old new = none ∧ nsSubst P.key P.urlHasOrigin old new (P.name D) = P.name D := by
  unfold merge at h
  split at h
  · next S D hS hD =>
    split at h
    · next ha =>
      unfold mergeAllowed at ha
      rw [hS, hD] at ha
      unfold mergeAllowedTo at ha
      simp only [Bool.and_eq_true, Bool.not_eq_true', Option.isSome_eq_false_iff,
        Option.isNone_iff_eq_none] at ha
      obtain ⟨⟨⟨⟨⟨⟨⟨⟨_, _⟩, hsub⟩, hnew⟩, _⟩, _⟩, _⟩, _⟩, _⟩ := ha
      exact ⟨D, hD, hsub, hnew, by unfold nsSubst; simp only [hsub]⟩
    · cases h
  · cases h

end Merge7
end CM
