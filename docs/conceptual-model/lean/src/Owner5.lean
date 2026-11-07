/- CI4 (D-20261020-CI), folded in round 5b.
The order is on pages; rank each page by its owner's graph-relative UTF-8 path.
The base page order only totalises absent owners / equal ranks. It cannot decide
between distinct declaring pages in a WF graph. Recompute ranks after rename.
-/
import Fold5 -- Keep the existing at-most-two-compiler dependency width.

set_option linter.unusedSectionVars false
set_option maxRecDepth 8192
set_option maxHeartbeats 1600000

namespace CM
namespace Owner5
open Params

/-- Explicit unsigned UTF-8 bytes, compared lexicographically, without locale,
case folding, path-component comparison, or Unicode normalization. -/
def bytes (s : String) : List Nat := s.toUTF8.data.toList.map UInt8.toNat

theorem bytes_injective {s t : String} (h : bytes s = bytes t) : s = t := by
  have h' := (List.map_inj_right (fun a b => UInt8.toNat_inj.mp)).mp h
  have h'' := Array.toList_inj.mp h'
  exact String.toByteArray_inj.mp (congrArg ByteArray.mk h'')

/-- Lexicographic primary rank plus the old lawful order on ties. -/
def rankLe {α : Type} (rank : α → List Nat) (le : α → α → Bool) (a b : α) : Bool :=
  decide (rank a ≤ rank b) && (if rank a = rank b then le a b else true)

theorem rankLe_iff {α : Type} (rank : α → List Nat) (le : α → α → Bool) (a b : α) :
    rankLe rank le a b = true ↔ rank a ≤ rank b ∧ (rank a = rank b → le a b = true) := by
  by_cases h : rank a = rank b <;> simp [rankLe, h]

theorem rankLe_linear {α : Type} (rank : α → List Nat) {le : α → α → Bool}
    (hl : Linear le) : Linear (rankLe rank le) where
  total a b := by
    by_cases h : rank a = rank b
    · rcases hl.total a b with hab | hba
      · exact Or.inl ((rankLe_iff ..).mpr ⟨by rw [h]; exact Std.le_refl _, fun _ => hab⟩)
      · exact Or.inr ((rankLe_iff ..).mpr ⟨by rw [h]; exact Std.le_refl _, fun _ => hba⟩)
    · rcases Std.le_total (a := rank a) (b := rank b) with hab | hba
      · exact Or.inl ((rankLe_iff ..).mpr ⟨hab, fun e => (h e).elim⟩)
      · exact Or.inr ((rankLe_iff ..).mpr ⟨hba, fun e => (h e.symm).elim⟩)
  antisymm a b hab hba := by
    obtain ⟨hab, hab'⟩ := (rankLe_iff ..).mp hab
    obtain ⟨hba, hba'⟩ := (rankLe_iff ..).mp hba
    have he := Std.le_antisymm hab hba
    exact hl.antisymm a b (hab' he) (hba' he.symm)
  trans a b c hab hbc := by
    obtain ⟨hab, hab'⟩ := (rankLe_iff ..).mp hab
    obtain ⟨hbc, hbc'⟩ := (rankLe_iff ..).mp hbc
    refine (rankLe_iff ..).mpr ⟨Std.le_trans hab hbc, ?_⟩
    intro hac
    have hba : rank b ≤ rank a := by simpa [hac] using hbc
    have he := Std.le_antisymm hab hba
    exact hl.trans a b c (hab' he) (hbc' (he.symm.trans hac))

variable {K : Type} [DecidableEq K]

def pathRank (P : Params K) (G : Graph) (p : Page K) : List Nat :=
  bytes (((P.owner G p).map File.path).getD "")

/-- A frozen snapshot supplies the page-to-owner-path map. Only pageLe changes
from round 5. Snapshot changes have separate T1/T4/W2 results below. -/
def params (P : Params K) (dateKey : K → Bool) (snapshot : Graph) : Params K :=
  { Policy5.params P dateKey with pageLe := rankLe (pathRank P snapshot) P.pageLe }

/-- Every Fold5 headline specialises directly through this equality. Its old
explicit hypotheses are retained; lawful below discharges the order premise. -/
theorem fold_instance (P : Params K) (dateKey : K → Bool) (G : Graph) :
    params P dateKey G = Policy5.params
      { P with pageLe := rankLe (pathRank P G) P.pageLe } dateKey := rfl

theorem lawful (P : Params K) (dateKey : K → Bool) (G : Graph) (hP : P.Lawful) :
    (params P dateKey G).Lawful := ⟨hP.file, rankLe_linear _ hP.page⟩

/-- The actual decided resolver recomputes the path order for its input. -/
def resolve (P : Params K) (dateKey : K → Bool) (G : Graph) (s : String) : Page K :=
  (params P dateKey G).resolve G s

theorem params_sameFiles (P : Params K) (dateKey : K → Bool) (hP : P.Lawful)
    {G H : Graph} (h : SameFiles G H) : params P dateKey G = params P dateKey H := by
  have hr : pathRank P G = pathRank P H := by
    funext p
    unfold pathRank
    rw [owner_congr hP h]
  simp only [params, hr]

/-- T1 with the order rebuilt on BOTH graphs, including repeated enumeration. -/
theorem T1_set (P : Params K) (dateKey : K → Bool) (hP : P.Lawful)
    {G H : Graph} (h : SameFiles G H) (s : String) :
    resolve P dateKey G s = resolve P dateKey H s := by
  unfold resolve
  rw [← params_sameFiles P dateKey hP h]
  exact resolve_sameFiles (lawful P dateKey G hP) h s

theorem T1_key (P : Params K) (dateKey : K → Bool) (G : Graph) {s t : String}
    (h : P.key s = P.key t) : resolve P dateKey G s = resolve P dateKey G t :=
  (resolve_key (P := params P dateKey G) G h).1

/-- Erase the choice of navigation owner: its concept is always the raw-key
component, even when ranks are recomputed. -/
theorem concept_erasure (P : Params K) (dateKey : K → Bool) (snapshot G : Graph) (s : String) :
    (params P dateKey snapshot).concept G s = (Policy5.params P dateKey).concept G s := by
  rw [concept_eq_class, concept_eq_class]
  rfl

theorem references_erasure (P : Params K) (dateKey : K → Bool) (snapshot G : Graph)
    (b : BlockId) (t : Page K) :
    (params P dateKey snapshot).References G b t ↔ (Policy5.params P dateKey).References G b t := by
  rw [references_iff_keys, references_iff_keys]
  rfl

/-- All non-navigation primitives ignore pageLe. These equalities also allow
old T2/T3/T4/T5/T6/W1/W2 statements to be used with freshly ranked graphs. -/
theorem structure_erasure (P : Params K) (dateKey : K → Bool) (snapshot G : Graph) :
    (params P dateKey snapshot).aliasEdges G = (Policy5.params P dateKey).aliasEdges G ∧
    Decided.pages (params P dateKey snapshot) G = Decided.pages (Policy5.params P dateKey) G ∧
    (params P dateKey snapshot).blocks G = (Policy5.params P dateKey).blocks G ∧
    (params P dateKey snapshot).conflictKeys G = (Policy5.params P dateKey).conflictKeys G :=
  ⟨rfl, rfl, rfl, rfl⟩

theorem pageLeaf_erasure (P : Params K) (dateKey : K → Bool) (snapshot G : Graph)
    {L : Type} (Q : QParams K L) (l : Leaf L) (p : Page K) :
    (params P dateKey snapshot).leafPage Q G l p =
      (Policy5.params P dateKey).leafPage Q G l p := by
  induction l with
  | refs => rfl
  | name => rfl
  | given => rfl
  | page l ih => exact ih

theorem blockLeaf_erasure (P : Params K) (dateKey : K → Bool) (snapshot G : Graph)
    {L : Type} (Q : QParams K L) (l : Leaf L) (b : BlockId) :
    (params P dateKey snapshot).leafBlock3 Q G l b =
      (Policy5.params P dateKey).leafBlock3 Q G l b := by
  cases l with
  | refs => rfl
  | name => rfl
  | given => rfl
  | page l =>
    have he : (params P dateKey snapshot).pageOfBlock G b =
        (Policy5.params P dateKey).pageOfBlock G b := rfl
    simp only [leafBlock3, he, pageLeaf_erasure]

theorem query_erasure (P : Params K) (dateKey : K → Bool) (snapshot G : Graph)
    {L : Type} (Q : QParams K L) (a : Anchor) (q : Query L) :
    Decided.den (params P dateKey snapshot) Q G a q =
      Decided.den (Policy5.params P dateKey) Q G a q := by
  have hu : (params P dateKey snapshot).univ true G a =
      (Policy5.params P dateKey).univ true G a := by cases a <;> rfl
  unfold Decided.den den3 denU3
  rw [hu]
  apply List.filter_congr
  intro n _
  unfold evalNode3
  congr 1
  funext l
  cases n with
  | page p => exact pageLeaf_erasure P dateKey snapshot G Q l p
  | block b => exact blockLeaf_erasure P dateKey snapshot G Q l b

theorem link_erasure (P : Params K) (dateKey : K → Bool) (snapshot G : Graph)
    (C : Decided.Config) (s t : Page K) :
    Decided.Link (params P dateKey snapshot) C G s t ↔
      Decided.Link (Policy5.params P dateKey) C G s t := by
  simp only [Decided.link_config_iff, references_erasure]
  rfl

theorem rename_erasure (P : Params K) (dateKey : K → Bool) (snapshot G : Graph)
    (R : Decided.Rewriter) (old new : String) :
    Decided.rename (params P dateKey snapshot) R G old new =
      Decided.rename (Policy5.params P dateKey) R G old new := rfl

/-- Every selected candidate has an actual declaring owner; pages are bridged
to paths through owner, not through their key or title. -/
theorem declarer_owner (P : Params K) (G : Graph) {k : K} {p : Page K}
    (hp : p ∈ P.declarers G k) :
    ∃ f ∈ G, P.owner G p = some f ∧ P.claim f = p ∧ P.declaresKey f k = true := by
  obtain ⟨f, hf, hd, hc⟩ := P.mem_declarers.mp hp
  obtain ⟨hm, ho, _⟩ := P.mem_declaringOwners.mp hf
  exact ⟨f, hm, P.owner_eq_some_iff.mpr ⟨hm, ho, hc⟩, hc, hd⟩

/-- CI4: in the alias-only, nonempty case the returned page is a direct
declarer and its actual owner's relative path has minimum UTF-8 bytes. -/
theorem CI4 (P : Params K) (dateKey : K → Bool) (hP : P.Lawful) (G : Graph) (s : String)
    (hj : P.real G (.journal, P.key s) = false)
    (hp : P.real G (.page, P.key s) = false)
    (hn : (Policy5.params P dateKey).declarers G (P.key s) ≠ []) :
    let D := params P dateKey G
    let q := resolve P dateKey G s
    q ∈ D.declarers G (P.key s) ∧
      ∃ f ∈ G, P.owner G q = some f ∧
        ∀ p ∈ D.declarers G (P.key s), ∀ g, P.owner G p = some g → bytes f.path ≤ bytes g.path := by
  let D := params P dateKey G
  have hm : minBy D.pageLe (D.declarers G (P.key s)) ≠ none := by
    intro he
    exact hn (minBy_eq_none.mp he)
  obtain ⟨q, hq⟩ : ∃ q, minBy D.pageLe (D.declarers G (P.key s)) = some q := by
    cases he : minBy D.pageLe (D.declarers G (P.key s)) with
    | none => exact (hm he).elim
    | some q => exact ⟨q, rfl⟩
  have hr : resolve P dateKey G s = q := by
    unfold resolve Params.resolve resolveKey
    have sj : P.solid G (.journal, P.key s) = false := by
      cases h : P.solid G (.journal, P.key s)
      · rfl
      · rw [P.solid_real h] at hj; exact hj
    have sp : P.solid G (.page, P.key s) = false := by
      cases h : P.solid G (.page, P.key s)
      · rfl
      · rw [P.solid_real h] at hp; exact hp
    change minBy (params P dateKey G).pageLe
      ((params P dateKey G).declarers G (P.key s)) = some q at hq
    change (if P.solid G (P.kindPref, P.key s) || (P.real G (P.kindPref, P.key s) &&
        (minBy (params P dateKey G).pageLe ((params P dateKey G).declarers G (P.key s))).isNone)
      then _ else if P.solid G (otherKind P.kindPref, P.key s) ||
        (P.real G (otherKind P.kindPref, P.key s) &&
        (minBy (params P dateKey G).pageLe ((params P dateKey G).declarers G (P.key s))).isNone)
      then _ else (match minBy (params P dateKey G).pageLe ((params P dateKey G).declarers G (P.key s)) with
        | some q => q
        | none => ((params P dateKey G).refKind (P.key s), P.key s))) = q
    cases P.kindPref <;> simp only [otherKind, sj, sp, hj, hp, hq, Option.isNone_some,
      Bool.false_and, Bool.false_or, Bool.false_eq_true, ↓reduceIte]
  change resolve P dateKey G s ∈ D.declarers G (P.key s) ∧ _
  rw [hr]
  have hmem := minBy_mem hq
  obtain ⟨f, hf, ho, _, _⟩ := declarer_owner D G hmem
  change P.owner G q = some f at ho
  refine ⟨hmem, f, hf, ho, ?_⟩
  intro p hp' g hg
  have hle := (rankLe_iff ..).mp (minBy_le (lawful P dateKey G hP).page hq p hp')
  simpa [pathRank, ho, hg] using hle.1

/-- On WF graphs, equal path ranks of actual owners imply equal pages.
Hence the fallback page order cannot break a tie between distinct declarers. -/
theorem owner_rank_injective (P : Params K) {G : Graph} (hw : WF G)
    {p q : Page K} {f g : File} (hp : P.owner G p = some f)
    (hq : P.owner G q = some g) (he : bytes f.path = bytes g.path) : p = q := by
  obtain ⟨hf, hc⟩ := P.owner_spec hp
  obtain ⟨hg, hd⟩ := P.owner_spec hq
  have hfg := eq_of_path_eq hw hf hg (bytes_injective he)
  exact hc.symm.trans (hfg ▸ hd)

/-- Any two direct declarers whose path ranks are least are the same page. -/
theorem minimum_unique (P : Params K) {G : Graph} (hw : WF G) {k : K}
    {p q : Page K} (hp : p ∈ P.declarers G k) (hq : q ∈ P.declarers G k)
    (hm : ∀ r ∈ P.declarers G k, pathRank P G p ≤ pathRank P G r)
    (hn : ∀ r ∈ P.declarers G k, pathRank P G q ≤ pathRank P G r) : p = q := by
  obtain ⟨f, _, hf, _, _⟩ := declarer_owner P G hp
  obtain ⟨g, _, hg, _, _⟩ := declarer_owner P G hq
  apply owner_rank_injective P hw hf hg
  have he := Std.le_antisymm (hm q hq) (hn p hp)
  simpa [pathRank, hf, hg] using he

theorem minBy_rank_congr {α : Type} (rank rank' : α → List Nat) (le : α → α → Bool)
    (l : List α) (h : ∀ p ∈ l, rank p = rank' p) :
    minBy (rankLe rank le) l = minBy (rankLe rank' le) l := by
  induction l with
  | nil => rfl
  | cons a l ih =>
    have ht : ∀ p ∈ l, rank p = rank' p := fun p hp => h p (List.mem_cons_of_mem _ hp)
    simp only [minBy, ih ht]
    cases he : minBy (rankLe rank' le) l with
    | none => rfl
    | some m =>
      have ha := h a (List.mem_cons_self ..)
      have hm := ht m (minBy_mem he)
      simp only [rankLe, ha, hm]
      rfl

/-- Rebuilding the rank table outside the candidate pages cannot affect this
navigation. No global equality of the two path tables is assumed. -/
theorem resolve_retarget (P : Params K) (dateKey : K → Bool) (A B target : Graph) (s : String)
    (h : ∀ p ∈ (Policy5.params P dateKey).declarers target (P.key s),
      pathRank P A p = pathRank P B p) :
    (params P dateKey A).resolve target s = (params P dateKey B).resolve target s := by
  have hm := minBy_rank_congr (pathRank P A) (pathRank P B) P.pageLe
    ((Policy5.params P dateKey).declarers target (P.key s)) h
  let fallback : Option (Page K) → Page K := fun r => match r with
    | some q => q
    | none => ((Policy5.params P dateKey).refKind (P.key s), P.key s)
  change (if P.solid target (P.kindPref, P.key s) || (P.real target (P.kindPref, P.key s) &&
        (minBy (rankLe (pathRank P A) P.pageLe)
          ((Policy5.params P dateKey).declarers target (P.key s))).isNone) then (P.kindPref, P.key s)
    else if P.solid target (otherKind P.kindPref, P.key s) ||
        (P.real target (otherKind P.kindPref, P.key s) && (minBy (rankLe (pathRank P A) P.pageLe)
          ((Policy5.params P dateKey).declarers target (P.key s))).isNone) then
      (otherKind P.kindPref, P.key s)
    else fallback (minBy (rankLe (pathRank P A) P.pageLe)
      ((Policy5.params P dateKey).declarers target (P.key s)))) =
    (if P.solid target (P.kindPref, P.key s) || (P.real target (P.kindPref, P.key s) &&
        (minBy (rankLe (pathRank P B) P.pageLe)
          ((Policy5.params P dateKey).declarers target (P.key s))).isNone) then (P.kindPref, P.key s)
    else if P.solid target (otherKind P.kindPref, P.key s) ||
        (P.real target (otherKind P.kindPref, P.key s) && (minBy (rankLe (pathRank P B) P.pageLe)
          ((Policy5.params P dateKey).declarers target (P.key s))).isNone) then
      (otherKind P.kindPref, P.key s)
    else fallback (minBy (rankLe (pathRank P B) P.pageLe)
      ((Policy5.params P dateKey).declarers target (P.key s))))
  rw [hm]

/-- T4 survives recomputation with its original class-disjoint condition. -/
theorem T4_resolve (P : Params K) (dateKey : K → Bool) (hP : P.Lawful)
    {π : String} {G H : Graph} (hd : DiffAt π G H) (s : String)
    (h1 : ∀ y, (Policy5.params P dateKey).aliasClass G (P.key s) y → ¬ P.Affected π G H y)
    (h2 : ∀ y, (Policy5.params P dateKey).aliasClass H (P.key s) y → ¬ P.Affected π G H y) :
    resolve P dateKey G s = resolve P dateKey H s := by
  have hold := (resolve_local (lawful P dateKey G hP) hd s h1 h2).1
  unfold resolve
  refine hold.trans (resolve_retarget P dateKey G H H s ?_)
  intro p hp
  have hc : (Policy5.params P dateKey).aliasClass H (P.key s) p.2 :=
    Reach.edge' ((Policy5.params P dateKey).declarer_edge hp)
  have he := owner_local hP hd (p := p) (fun f hf hpath he =>
    h2 p.2 hc ⟨f, hf, hpath, by rw [he]⟩)
  simp only [pathRank, he]

/-- W2 navigation survives recomputation under the SAME input Agreement.
Alias closure protects every candidate's key, freezing its whole claimant set. -/
theorem W2_resolve (P : Params K) (dateKey : K → Bool) (hP : P.Lawful)
    {S : List K} {G H : Graph} (A : Isolation.Agreement (Policy5.params P dateKey) S G H)
    {s : String} (hs : P.key s ∈ S) : resolve P dateKey G s = resolve P dateKey H s := by
  have A' : Isolation.Agreement (params P dateKey G) S G H :=
    ⟨A.files, A.closedG, A.closedH⟩
  have hold := Isolation.resolve_agree (params P dateKey G) (lawful P dateKey G hP) A' hs
  unfold resolve
  refine hold.trans (resolve_retarget P dateKey G H H s ?_)
  intro p hp
  have hedge := (Policy5.params P dateKey).declarer_edge hp
  have hkey := (A.closedH _ hedge).mpr hs
  have he := Isolation.owner_agree (Policy5.params P dateKey) (Policy5.lawful P dateKey hP)
    A (p := p) (Isolation.region_protected (Policy5.params P dateKey) hkey)
  change P.owner G p = P.owner H p at he
  simp only [pathRank, he]

end Owner5
end CM
