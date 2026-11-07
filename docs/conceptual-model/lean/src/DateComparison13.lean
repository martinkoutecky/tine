/- Comparison instrumentation only. No public reader or operation rule changes. -/
import DateWitness13

set_option linter.unusedSectionVars false
set_option linter.unusedSimpArgs false
namespace CM
namespace Date13
open Params Consolidated13 QueryRename6
variable {K L : Type} [DecidableEq K] (P : Inputs K)

/-- The generic real-first reader's target before direct-alias routing. -/
def genericTarget (G : Graph) (s : String) : Page K :=
  let R := readParams P G
  let A := activeGraph P G
  if R.real A (P.kindPref, P.key s) then (P.kindPref, P.key s)
  else if R.real A (otherKind P.kindPref, P.key s) then (otherKind P.kindPref, P.key s)
  else (P.refKind (P.key s), P.key s)

/-- Only an actually used direct-declarer choice is compared. Real targets
bypass aliases in both readers; the optional choice preserves that branch. -/
def genericDirect (G : Graph) (s : String) : Option (Page K) :=
  let R := readParams P G
  let A := activeGraph P G
  if R.real A (P.kindPref, P.key s) then none
  else if R.real A (otherKind P.kindPref, P.key s) then none
  else minBy R.pageLe (R.declarers A (P.key s))

def finalDirect (D : Dates K) (G : Graph) (s : String) : Option (Page K) :=
  if !(realFiles P G (target P D G s)).isEmpty then none else
    (minBy (Owner5.rankLe (fun f => Owner5.bytes f.path) Orders.fileLe)
      (declarers P D G (target P D G s))).map P.claim

def navigationCases (D : Dates K) (G : Graph) (s : String) : List (String × Bool) :=
  [("navigation target", decide (target P D G s ≠ genericTarget P G s)),
   ("navigation direct declarer", decide (finalDirect P D G s ≠ genericDirect P G s))]

theorem generic_resolve_inputs (G : Graph) (s : String) :
    Consolidated13.resolve P G s = (genericDirect P G s).getD (genericTarget P G s) := by
  unfold Consolidated13.resolve Rulings9.resolveKeyR8 genericDirect genericTarget
  simp only [show (readParams P G).kindPref = P.kindPref from rfl,
    show (readParams P G).refKind = P.refKind from rfl]
  split <;> simp_all
  split <;> simp_all
  cases minBy (readParams P G).pageLe ((readParams P G).declarers (activeGraph P G) (P.key s)) <;> rfl

theorem final_resolve_inputs (D : Dates K) (G : Graph) (s : String) :
    resolve P D G s = (finalDirect P D G s).getD (target P D G s) := by
  simp only [resolve, finalDirect]
  split <;> simp_all

theorem navigationCases_exact (D : Dates K) (G : Graph) (s : String) :
    anyDifference (navigationCases P D G s) ↔
      target P D G s ≠ genericTarget P G s ∨ finalDirect P D G s ≠ genericDirect P G s := by
  simp [anyDifference, navigationCases]

theorem navigation_comparison_generic (D : Dates K) (G : Graph) (s : String) :
    anyDifference (navigationCases P D G s) ∨ resolve P D G s = Consolidated13.resolve P G s := by
  classical
  by_cases h : anyDifference (navigationCases P D G s)
  · exact Or.inl h
  right
  have ht : target P D G s = genericTarget P G s :=
    Classical.byContradiction fun hn => h ((navigationCases_exact P D G s).mpr (Or.inl hn))
  have hd : finalDirect P D G s = genericDirect P G s :=
    Classical.byContradiction fun hn => h ((navigationCases_exact P D G s).mpr (Or.inr hn))
  rw [final_resolve_inputs, generic_resolve_inputs, ht, hd]

/-- No premise: all differences from R12 are accounted for, including unknown
names and empty graphs. A positive cause may be masked by another branch. -/
theorem navigation_comparison_exhaustive (D : Dates K) (G : Graph) (s : String) :
    activeGraph P G ≠ G ∨ readerNormalization P G ∨ emptyReal P G s = true ∨
      anyDifference (navigationCases P D G s) ∨ resolve P D G s = P.resolve G s := by
  let Q : QParams K Unit := ⟨fun _ _ => [], fun _ _ => false, fun _ _ _ => false, fun _ _ _ => false⟩
  rcases static_comparison_exhaustive P Q { favoritesPage := "" } G s with ha | hp | he | h
  · exact Or.inl ha
  · exact Or.inr (Or.inl hp)
  · exact Or.inr (Or.inr (Or.inl he))
  · right; right; right
    rcases navigation_comparison_generic P D G s with hn | hn
    · exact Or.inl hn
    · exact Or.inr (hn.trans h.2.2.2.2.2.2.2.2.2)

/-- Each raw page-creation input is compared before any list is assembled.
Its optional row exposes target interpretation, real suppression and Q9. -/
def rawSeeds (G : Graph) : List String :=
  (Consolidated13.blocks P G).flatMap (P.refNames (activeGraph P G)) ++
    (activeGraph P G).flatMap P.fileNsNames

def finalSeed (D : Dates K) (G : Graph) (s : String) : Option (Page K) :=
  let t := target P D G s
  if (realFiles P G t).isEmpty && !(edges P D G).any (fun e => decide (e.2 = t)) then some t else none

def genericSeed (G : Graph) (s : String) : Option (Page K) :=
  let R := readParams P G
  if !R.claimedKey (activeGraph P G) (P.key s) && !R.declaredKey (activeGraph P G) (P.key s)
  then some (P.refKind (P.key s), P.key s) else none

theorem final_pages_seed_rows (D : Dates K) (G : Graph) :
    pages P D G = (activeGraph P G).map P.claim ++ (rawSeeds P G).filterMap (finalSeed P D G) := by
  have hs : seedTargets P D G = (rawSeeds P G).map (target P D G) := by
    simp only [seedTargets, rawSeeds, List.map_append, List.map_flatMap]
    rfl
  rw [pages, hs]
  congr 1
  generalize rawSeeds P G = xs
  induction xs with
  | nil => rfl
  | cons a xs ih =>
    simp only [List.map_cons, List.filter_cons, List.filterMap_cons, finalSeed]
    split <;> simp_all

theorem generic_pages_seed_rows (G : Graph) :
    Consolidated13.pages P G = (activeGraph P G).map P.claim ++ (rawSeeds P G).filterMap (genericSeed P G) := by
  have hs : (readParams P G).pageSeedKeys (activeGraph P G) = (rawSeeds P G).map P.key := by
    simp only [Params.pageSeedKeys, Params.refKeys, Params.nameKeys, rawSeeds,
      List.map_append, List.map_flatMap, Function.comp_def]
    rfl
  change (activeGraph P G).map P.claim ++ _ = _
  congr 1
  unfold Params.refOnlyPagesR
  rw [hs]
  generalize rawSeeds P G = xs
  induction xs with
  | nil => rfl
  | cons a xs ih =>
    simp only [List.map_cons, List.filter_cons, List.filterMap_cons, genericSeed, Bool.true_and]
    cases hc : (readParams P G).claimedKey (activeGraph P G) (P.key a) <;>
      cases hd : (readParams P G).declaredKey (activeGraph P G) (P.key a) <;>
      simp [hc, hd, show (readParams P G).refKind = P.refKind from rfl] <;> exact ih

theorem pages_eq_of_seed_rows (D : Dates K) (G : Graph)
    (h : ∀ s ∈ rawSeeds P G, finalSeed P D G s = genericSeed P G s) :
    pages P D G = Consolidated13.pages P G := by
  rw [final_pages_seed_rows, generic_pages_seed_rows]
  congr 1
  generalize rawSeeds P G = xs at *
  induction xs with
  | nil => rfl
  | cons a xs ih =>
    rw [List.filterMap_cons, List.filterMap_cons, h a (by simp)]
    have ht := ih (fun s hs => h s (by simp [hs]))
    cases genericSeed P G a <;> simp [ht]

/-- Source incidence is deliberately a primitive diagnostic: duplicate physical
paths are representable, and fileAt versus owner-block enumeration can differ. -/
def finalBlockSource (G : Graph) (S : Page K) (b : BlockId) : Bool :=
  decide (b ∈ Consolidated13.blocks P G) && decide (P.pageOfBlock (activeGraph P G) b = some S)

def genericBlockSource (G : Graph) (S : Page K) (b : BlockId) : Bool :=
  decide (b ∈ (readParams P G).pageBlocks (activeGraph P G) S)

/-- Compare the immediate raw-parent lookup, before constructing any tree. -/
def finalParentLookup (D : Dates K) (G : Graph) (s : String) : Option (Page K) :=
  (Consolidated13.pages P G).find? (fun p => decide (p = target P D G s))

def genericParentLookup (G : Graph) (s : String) : Option (Page K) :=
  (Consolidated13.pages P G).eraseDups.find? (fun p => decide (p.2 = P.key s))

/-- Primitive witnesses, rather than inequality of complete reader outputs.
The reference clause compares a single closed-spelling path answer. -/
def readerDifference (D : Dates K) (Q : QParams K L) (favorites : Option String) (G : Graph) : Prop :=
  (∃ s ∈ rawSeeds P G, finalSeed P D G s ≠ genericSeed P G s) ∨
  (∃ b, ∃ s ∈ P.refNames (activeGraph P G) b, ∃ t,
    sameCls (edges P D G) (target P D G s) t ≠
      sameCls ((readParams P G).aliasEdges (activeGraph P G)) (P.key s) t.2) ∨
  (∃ a n, n ∈ univ P D G a ∧ ∃ l,
    leafAt P D Q G l n ≠ (readParams P G).leafAt3 Q (activeGraph P G) l n) ∨
  (∃ t b, b ∈ Consolidated13.blocks P G ∧
    panelSource P D favorites G t b ≠ Consolidated13.panelSource P favorites G t b) ∨
  (∃ S b, finalBlockSource P G S b ≠ genericBlockSource P G S b) ∨
  (∃ s, finalParentLookup P D G s ≠ genericParentLookup P G s)

theorem references_eq_of_paths (D : Dates K) (G : Graph)
    (h : ∀ b s, s ∈ P.refNames (activeGraph P G) b → ∀ t,
      sameCls (edges P D G) (target P D G s) t =
        sameCls ((readParams P G).aliasEdges (activeGraph P G)) (P.key s) t.2) :
    referencesB P D G = Consolidated13.referencesB P G := by
  funext b t
  apply Bool.eq_iff_iff.mpr
  simp only [referencesB, refTargets, Consolidated13.referencesB, keyReferences,
    List.any_map, List.any_eq_true]
  constructor <;> rintro ⟨s, hs, hv⟩
  · exact ⟨s, hs, (h b s hs t) ▸ hv⟩
  · refine ⟨s, hs, ?_⟩
    change sameCls (edges P D G) (target P D G s) t = true
    rw [h b s hs t]
    exact hv

theorem den_eq_of_inputs (D : Dates K) (Q : QParams K L) (G : Graph)
    (hp : pages P D G = Consolidated13.pages P G)
    (hl : ∀ a n, n ∈ univ P D G a → ∀ l,
      leafAt P D Q G l n = (readParams P G).leafAt3 Q (activeGraph P G) l n) :
    den P D Q G = Consolidated13.den P Q G := by
  funext a q
  have hu : univ P D G a = (readParams P G).univ true (activeGraph P G) a := by
    cases a <;> simp only [univ, Params.univ, hp, Consolidated13.pages, Decided.pages]; rfl
  unfold den Consolidated13.den Decided.den Params.den3 Params.denU3 Params.evalNode3
  rw [← hu]
  apply List.filter_congr
  intro n hm
  exact bval_congr_leaves q _ _ (fun l _ => hl a n hm l)

theorem links_eq_of_inputs (D : Dates K) (config : Decided.Config) (G : Graph)
    (hp : pages P D G = Consolidated13.pages P G)
    (hr : referencesB P D G = Consolidated13.referencesB P G)
    (hs : ∀ S b, finalBlockSource P G S b = genericBlockSource P G S b) :
    links P D config G = Consolidated13.links P config G := by
  have hi : ∀ S b, (b ∈ Consolidated13.blocks P G ∧ P.pageOfBlock (activeGraph P G) b = some S) ↔
      b ∈ (readParams P G).pageBlocks (activeGraph P G) S := by
    intro S b
    have hh := Bool.eq_iff_iff.mp (hs S b)
    simpa only [finalBlockSource, genericBlockSource, Bool.and_eq_true, decide_eq_true_eq] using hh
  funext S T
  apply Bool.eq_iff_iff.mpr
  rw [links_exact, Consolidated13.links_eq, Decided.linkB_iff, Decided.link_config_iff]
  simp only [hp, hr, Consolidated13.pages, (readParams P G).referencesB_iff]
  have hh : (∃ b ∈ Consolidated13.blocks P G,
      P.pageOfBlock (activeGraph P G) b = some S ∧ Consolidated13.referencesB P G b T = true) ↔
      ∃ b ∈ (readParams P G).pageBlocks (activeGraph P G) S,
        Consolidated13.referencesB P G b T = true := by
    constructor
    · rintro ⟨b, hb, hS, ht⟩; exact ⟨b, (hi S b).mp ⟨hb, hS⟩, ht⟩
    · rintro ⟨b, hb, ht⟩
      obtain ⟨hb, hS⟩ := (hi S b).mpr hb
      exact ⟨b, hb, hS, ht⟩
  apply and_congr_right; intro _
  apply and_congr_right; intro _
  apply and_congr_right; intro _
  apply and_congr_right; intro _
  simpa only [Consolidated13.referencesB, keyReferences_eq, ← (readParams P G).referencesB_iff] using hh

theorem near_eq_of_inputs (D : Dates K) (config : Decided.Config) (G : Graph)
    (hp : pages P D G = Consolidated13.pages P G)
    (hl : links P D config G = Consolidated13.links P config G) :
    near P D config G = Consolidated13.near P config G := by
  funext d n X
  cases d <;> simp only [near, Consolidated13.near, hp, hl]

theorem treeParent_eq_of_inputs (D : Dates K) (spell : K → String) (G : Graph)
    (hp : pages P D G = Consolidated13.pages P G)
    (hl : ∀ s, finalParentLookup P D G s = genericParentLookup P G s) :
    treeParent P D spell G = Rulings11.treeParent (readParams P G) spell (activeGraph P G) := by
  funext t
  simp only [treeParent, hp, Rulings11.treeParent, Rulings11.treeNodes,
    List.contains_iff_mem, List.mem_eraseDups]
  change (if t ∈ Consolidated13.pages P G then
      (Ref10.nsParent P.toParams (spell t.2)).bind (fun s =>
        (Consolidated13.pages P G).find? (fun p => decide (p = target P D G s))) else none) =
    (if t ∈ Consolidated13.pages P G then
      match Ref10.nsParent P.toParams (spell t.2) with
      | none => none
      | some s => genericParentLookup P G s
    else none)
  split
  · simp only [finalParentLookup, genericParentLookup] at hl
    cases he : Ref10.nsParent P.toParams (spell t.2) with
    | none => simp only [he, Option.bind_none]
    | some s =>
      change (Consolidated13.pages P G).find? _ = _
      exact hl s
  · rfl

theorem trees_eq_of_inputs (D : Dates K) (spell : K → String) (G : Graph)
    (hp : pages P D G = Consolidated13.pages P G)
    (hl : ∀ s, finalParentLookup P D G s = genericParentLookup P G s) :
    tree P D spell G = Consolidated13.tree P spell G ∧
      namespaceTree P D spell G = Consolidated13.namespaceTree P spell G := by
  have ht := treeParent_eq_of_inputs P D spell G hp hl
  have hc : children P D spell G = Rulings11.children (readParams P G) spell (activeGraph P G) := by
    funext t
    simp only [children, hp, ht, Rulings11.children, Rulings11.treeNodes]
    rfl
  have he : tree P D spell G = Consolidated13.tree P spell G := by
    simp only [tree, hp, hc, Consolidated13.tree, Rulings11.tree, Rulings11.treeNodes]
    rfl
  refine ⟨he, ?_⟩
  simp only [namespaceTree, he, ht, Consolidated13.namespaceTree, Consolidated13.tree, Rulings11.namespaceTree,
    Rulings11.namespaceNodes, Rulings11.tree, Rulings11.treeNodes, List.filter_map]
  rfl

/-- Equality certificate for the intermediate generic reader. These fields are
conclusions derived from primitive inputs, never premises of a headline. -/
structure GenericStaticAgreement (D : Dates K) (Q : QParams K L) (config : Decided.Config)
    (favorites : Option String) (spell : K → String) (G : Graph) : Prop where
  pageList : pages P D G = Consolidated13.pages P G
  references : referencesB P D G = Consolidated13.referencesB P G
  queries : den P D Q G = Consolidated13.den P Q G
  panels : linked P D favorites G = Consolidated13.linked P favorites G
  counts : referenceCount P D favorites G = Consolidated13.referenceCount P favorites G
  queryRows : ∀ R o a q, queryResults P D Q R o G a q = Consolidated13.queryResults P Q R o G a q
  linkRelation : links P D config G = Consolidated13.links P config G
  neighborhoods : near P D config G = Consolidated13.near P config G
  fullTree : tree P D spell G = Consolidated13.tree P spell G
  namespaces : namespaceTree P D spell G = Consolidated13.namespaceTree P spell G

theorem generic_static_of_no_difference (D : Dates K) (Q : QParams K L) (config : Decided.Config)
    (favorites : Option String) (spell : K → String) (G : Graph)
    (hn : ¬ readerDifference P D Q favorites G) :
    GenericStaticAgreement P D Q config favorites spell G := by
  classical
  have hs : ∀ s ∈ rawSeeds P G, finalSeed P D G s = genericSeed P G s := by
    intro s hm
    exact Classical.byContradiction fun he => hn (Or.inl ⟨s, hm, he⟩)
  have hp := pages_eq_of_seed_rows P D G hs
  have hr : referencesB P D G = Consolidated13.referencesB P G := by
    apply references_eq_of_paths
    intro b s hm t
    exact Classical.byContradiction fun he => hn (Or.inr (Or.inl ⟨b, s, hm, t, he⟩))
  have hleaf : ∀ a n, n ∈ univ P D G a → ∀ l,
      leafAt P D Q G l n = (readParams P G).leafAt3 Q (activeGraph P G) l n := by
    intro a n hm l
    exact Classical.byContradiction fun he => hn (Or.inr (Or.inr (Or.inl ⟨a, n, hm, l, he⟩)))
  have hpanel : ∀ t b, b ∈ Consolidated13.blocks P G →
      panelSource P D favorites G t b = Consolidated13.panelSource P favorites G t b := by
    intro t b hm
    exact Classical.byContradiction fun he => hn (Or.inr (Or.inr (Or.inr (Or.inl ⟨t, b, hm, he⟩))))
  have hsource : ∀ S b, finalBlockSource P G S b = genericBlockSource P G S b := by
    intro S b
    exact Classical.byContradiction fun he => hn (Or.inr (Or.inr (Or.inr (Or.inr (Or.inl ⟨S, b, he⟩)))))
  have hparent : ∀ s, finalParentLookup P D G s = genericParentLookup P G s := by
    intro s
    exact Classical.byContradiction fun he => hn (Or.inr (Or.inr (Or.inr (Or.inr (Or.inr ⟨s, he⟩)))))
  have hl := links_eq_of_inputs P D config G hp hr hsource
  have ht := trees_eq_of_inputs P D spell G hp hparent
  have hden := den_eq_of_inputs P D Q G hp hleaf
  have hlinked : linked P D favorites G = Consolidated13.linked P favorites G := by
    funext t
    unfold linked Consolidated13.linked relationHits
    rw [List.filter_filter]
    apply List.filter_congr
    intro b hb
    rw [hr, hpanel t b (List.mem_eraseDups.mp hb)]
    exact Bool.and_comm ..
  refine ⟨hp, hr, hden, hlinked, ?_, ?_,
    hl, near_eq_of_inputs P D config G hp hl, ht.1, ht.2⟩
  · funext t
    exact congrArg List.length (congrFun hlinked t)
  · intro R o a q
    simp only [queryResults, Consolidated13.queryResults, hden]

/-- R12 has raw reference hits and raw denotations. Panels/counts and displayed
queries compare the same current projections of those old raw readings. -/
structure StaticAgreement (D : Dates K) (Q : QParams K L) (config : Decided.Config)
    (favorites : Option String) (spell : K → String) (G : Graph) (s : String) : Prop where
  pageList : pages P D G = Decided.pages P.toParams G
  blockList : Consolidated13.blocks P G = P.blocks G
  references : referencesB P D G = P.referencesB G
  queries : den P D Q G = Decided.den P.toParams Q G
  panels : ∀ t, linked P D favorites G t =
    ((P.blocks G).eraseDups.filter (fun b => P.referencesB G b t)).filter (Consolidated13.panelSource P favorites G t)
  counts : ∀ t, referenceCount P D favorites G t =
    (((P.blocks G).eraseDups.filter (fun b => P.referencesB G b t)).filter (Consolidated13.panelSource P favorites G t)).length
  queryRows : ∀ R o a q, queryResults P D Q R o G a q = queryProjection P Q R o G (Decided.den P.toParams Q G a q)
  linkRelation : links P D config G = Decided.linkB P.toParams config G
  neighborhoods : near P D config G = Decided.nearL P.toParams config G
  fullTree : tree P D spell G = Rulings11.tree P.toParams spell G
  namespaces : namespaceTree P D spell G = Rulings11.namespaceTree P.toParams spell G
  navigation : resolve P D G s = P.resolve G s

/-- Every graph and every selected navigation request: either an independently
computed primitive diagnostic, or equality of the complete final reader. -/
theorem static_comparison_exhaustive_final (D : Dates K) (Q : QParams K L) (config : Decided.Config)
    (favorites : Option String) (spell : K → String) (G : Graph) (s : String) :
    activeGraph P G ≠ G ∨ readerNormalization P G ∨ emptyReal P G s = true ∨
      readerDifference P D Q favorites G ∨ anyDifference (navigationCases P D G s) ∨
      StaticAgreement P D Q config favorites spell G s := by
  classical
  by_cases ha : activeGraph P G = G
  case neg => exact Or.inl ha
  right
  by_cases hparams : readParams P G = P.toParams
  case neg => exact Or.inl hparams
  right
  by_cases hempty : emptyReal P G s = true
  case pos => exact Or.inl hempty
  right
  by_cases hd : readerDifference P D Q favorites G
  · exact Or.inl hd
  right
  rcases navigation_comparison_generic P D G s with hn | hn
  · exact Or.inl hn
  right
  have h := generic_static_of_no_difference P D Q config favorites spell G hd
  have ho : _ := Consolidated13.static_comparison_exhaustive P Q config G s
  rcases ho with hA | hP | hE | ho
  · exact False.elim (hA ha)
  · exact False.elim (hP hparams)
  · exact False.elim (hempty hE)
  obtain ⟨hp, hb, hr, hq, hl, _, hhits, _, hnear, hnav⟩ := ho
  refine ⟨h.pageList.trans hp, hb, h.references.trans hr, h.queries.trans hq, ?_, ?_, ?_,
    h.linkRelation.trans hl, h.neighborhoods.trans hnear, ?_, ?_, hn.trans hnav⟩
  · intro t; rw [h.panels, Consolidated13.linked, hhits]
  · intro t; rw [h.counts, Consolidated13.referenceCount, Consolidated13.linked, hhits]
  · intro R o a q
    simp only [queryResults, h.queries, hq]
  · simpa only [Consolidated13.tree, ha, hparams] using h.fullTree
  · simpa only [Consolidated13.namespaceTree, ha, hparams] using h.namespaces

/-- Direct comparison to historical raw panels/query rows additionally names
every suppressed input row. The old reader did not specify a presentation UI. -/
theorem presented_comparison_exhaustive_final (D : Dates K) (Q : QParams K L) (config : Decided.Config)
    (favorites : Option String) (spell : K → String) (R : QueryRowParser) (o : QueryPresentation)
    (G : Graph) (s : String) (t : Page K) (a : Anchor) (q : Query L) :
    activeGraph P G ≠ G ∨ readerNormalization P G ∨ emptyReal P G s = true ∨
      readerDifference P D Q favorites G ∨ anyDifference (navigationCases P D G s) ∨
      (∃ b ∈ (P.blocks G).eraseDups.filter (fun b => P.referencesB G b t),
        Consolidated13.panelSource P favorites G t b = false) ∨
      (∃ b, Node.block b ∈ Decided.den P.toParams Q G a q ∧
        queryRowVisible P Q R o G (Decided.den P.toParams Q G a q) (.block b) = false) ∨
      (linked P D favorites G t = (P.blocks G).eraseDups.filter (fun b => P.referencesB G b t) ∧
       referenceCount P D favorites G t = ((P.blocks G).eraseDups.filter (fun b => P.referencesB G b t)).length ∧
       queryResults P D Q R o G a q = Decided.den P.toParams Q G a q) := by
  classical
  rcases static_comparison_exhaustive_final P D Q config favorites spell G s with ha | hp | he | hd | hv | h
  · exact Or.inl ha
  · exact Or.inr (Or.inl hp)
  · exact Or.inr (Or.inr (Or.inl he))
  · exact Or.inr (Or.inr (Or.inr (Or.inl hd)))
  · exact Or.inr (Or.inr (Or.inr (Or.inr (Or.inl hv))))
  right; right; right; right; right
  by_cases hs : ∃ b ∈ (P.blocks G).eraseDups.filter (fun b => P.referencesB G b t),
      Consolidated13.panelSource P favorites G t b = false
  · exact Or.inl hs
  right
  by_cases hq : ∃ b, Node.block b ∈ Decided.den P.toParams Q G a q ∧
      queryRowVisible P Q R o G (Decided.den P.toParams Q G a q) (.block b) = false
  · exact Or.inl hq
  right
  have hpanel : (((P.blocks G).eraseDups.filter (fun b => P.referencesB G b t)).filter
      (Consolidated13.panelSource P favorites G t)) =
      (P.blocks G).eraseDups.filter (fun b => P.referencesB G b t) := by
    apply List.filter_eq_self.mpr
    intro b hb
    cases hv : Consolidated13.panelSource P favorites G t b with
    | true => rfl
    | false => exact False.elim (hs ⟨b, hb, hv⟩)
  have hquery : queryProjection P Q R o G (Decided.den P.toParams Q G a q) =
      Decided.den P.toParams Q G a q := by
    apply List.filter_eq_self.mpr
    intro n hn
    cases n with
    | page _ => rfl
    | block b =>
      cases hv : queryRowVisible P Q R o G (Decided.den P.toParams Q G a q) (.block b) with
      | true => rfl
      | false => exact False.elim (hq ⟨b, hn, hv⟩)
  exact ⟨(h.panels t).trans hpanel, (h.counts t).trans (congrArg List.length hpanel),
    (h.queryRows R o a q).trans hquery⟩

end Date13
end CM
