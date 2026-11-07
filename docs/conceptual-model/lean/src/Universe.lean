/-
Round 2 vocabulary shared by rename (T3), queries (T5) and links (T6).

* The page universe under both readings of earlier **P1**, enlarged in round 4 with real-name parents.
  Reading A (`r = false`,
  SPEC §3.2 literally, and round 1's `pages G`): a referenced key that no file claims is a reference-only
  page, even when some page declares it as an alias. Reading B (`r = true`): such a key is not a page of
  its own when some declaring owner declares it (navigation sends it to the declarer anyway). Every
  generic theorem is stated for an arbitrary `r`, so it holds under both readings.
* `InPlay G k`: the keys a graph mentions (claimed, referenced from an owner block, or an end of an
  alias edge), with an executable list form.
* `pageBlocks G p`: the blocks of page `p` (the blocks of its owner file, in outline order).
-/
import Pages

set_option linter.unusedSectionVars false

namespace CM
namespace Params

variable {K : Type} [DecidableEq K] (P : Params K)

/-! ## The page universe, both readings of P1 -/

/-- Some declaring owner declares a spelling of key `k` (an alias edge ends at `k`). -/
def declaredKey (G : Graph) (k : K) : Bool := (P.aliasEdges G).any (fun e => decide (e.2 = k))

/-- Namespace parents contributed by real file names (CM3-3). -/
def nameKeys (G : Graph) : List K := G.flatMap (fun f => (P.fileNsNames f).map P.key)

/-- Page-creation seeds; this does not add occurrences or Linked References. -/
def pageSeedKeys (G : Graph) : List K := P.refKeys G ++ P.nameKeys G

theorem mem_nameKeys {G : Graph} {k : K} : k ∈ P.nameKeys G ↔
    ∃ f ∈ G, ∃ s ∈ P.fileNsNames f, P.key s = k := by simp [nameKeys]

theorem nameKeys_sameFiles {G H : Graph} (h : SameFiles G H) (k : K) :
    k ∈ P.nameKeys G ↔ k ∈ P.nameKeys H := by
  simp only [P.mem_nameKeys]
  constructor <;> rintro ⟨f, hf, hs⟩
  · exact ⟨f, (h f).mp hf, hs⟩
  · exact ⟨f, (h f).mpr hf, hs⟩

theorem pageSeedKeys_sameFiles (hP : P.Lawful) {G H : Graph} (hw : WF H)
    (h : SameFiles G H) (k : K) : k ∈ P.pageSeedKeys G ↔ k ∈ P.pageSeedKeys H := by
  simp only [pageSeedKeys, List.mem_append, refKeys_sameFiles hP hw h, nameKeys_sameFiles P h]

/-- Reference-only pages under reading `r`, with CM3-3 file-name parents. -/
def refOnlyPagesR (r : Bool) (G : Graph) : List (Page K) :=
  ((P.pageSeedKeys G).filter (fun k => !P.claimedKey G k && !(r && P.declaredKey G k))).map
    (fun k => (P.refKind k, k))

/-- `pages G` under reading `r` of P1. -/
def pagesR (r : Bool) (G : Graph) : List (Page K) := P.realPages G ++ P.refOnlyPagesR r G

/-- The old reading-A list is recovered when no file contributes parents.
The unrestricted round-2 equality is false after CM3-3. -/
theorem pagesR_false_no_names (G : Graph) (h : P.nameKeys G = []) :
    P.pagesR false G = P.pagesLegacyA G := by
  simp [pagesR, pagesLegacyA, refOnlyPagesR, refOnlyPagesLegacyA, pageSeedKeys, h]

theorem declaredKey_iff {G : Graph} {k : K} :
    P.declaredKey G k = true ↔ ∃ e ∈ P.aliasEdges G, e.2 = k := by
  simp [declaredKey]

theorem mem_refOnlyPagesR {r : Bool} {G : Graph} {p : Page K} :
    p ∈ P.refOnlyPagesR r G ↔
      p = (P.refKind p.2, p.2) ∧ p.2 ∈ P.pageSeedKeys G ∧ P.claimedKey G p.2 = false ∧
        (r && P.declaredKey G p.2) = false := by
  obtain ⟨κ, k⟩ := p
  simp only [refOnlyPagesR, List.mem_map, List.mem_filter, Bool.and_eq_true, Bool.not_eq_true',
    Prod.mk.injEq]
  constructor
  · rintro ⟨k', ⟨h1, h2, h3⟩, e1, e2⟩; subst e2; exact ⟨⟨e1.symm, trivial⟩, h1, h2, h3⟩
  · rintro ⟨⟨e1, _⟩, h1, h2, h3⟩; exact ⟨k, ⟨h1, h2, h3⟩, e1.symm, rfl⟩

theorem mem_pagesR {r : Bool} {G : Graph} {p : Page K} :
    p ∈ P.pagesR r G ↔ P.real G p = true ∨
      (p = (P.refKind p.2, p.2) ∧ p.2 ∈ P.pageSeedKeys G ∧ P.claimedKey G p.2 = false ∧
        (r && P.declaredKey G p.2) = false) := by
  rw [pagesR, List.mem_append, P.mem_realPages, P.mem_refOnlyPagesR]

/-! ## Keys in play -/

/-- The keys graph `G` mentions: claimed by a file, referenced (namespace-closed) from an owner block,
or an end of an alias edge. -/
def InPlay (G : Graph) (k : K) : Prop :=
  P.claimedKey G k = true ∨ k ∈ P.pageSeedKeys G ∨ ∃ e ∈ P.aliasEdges G, e.1 = k ∨ e.2 = k

/-- The keys in play, listed (with repetitions). -/
def inPlayKeys (G : Graph) : List K :=
  G.map (fun f => (P.claim f).2) ++ P.pageSeedKeys G ++ (P.aliasEdges G).flatMap (fun e => [e.1, e.2])

theorem inPlay_iff {G : Graph} {k : K} : P.InPlay G k ↔ k ∈ P.inPlayKeys G := by
  unfold InPlay inPlayKeys
  rw [P.claimedKey_iff]
  simp only [List.mem_append, List.mem_map, List.mem_flatMap, List.mem_cons, List.not_mem_nil, or_false]
  constructor
  · rintro (⟨f, hf, e⟩ | h | ⟨e, he, h1 | h2⟩)
    · exact Or.inl (Or.inl ⟨f, hf, e⟩)
    · exact Or.inl (Or.inr h)
    · exact Or.inr ⟨e, he, Or.inl h1.symm⟩
    · exact Or.inr ⟨e, he, Or.inr h2.symm⟩
  · rintro ((⟨f, hf, e⟩ | h) | ⟨e, he, h1 | h2⟩)
    · exact Or.inl ⟨f, hf, e⟩
    · exact Or.inr (Or.inl h)
    · exact Or.inr (Or.inr ⟨e, he, Or.inl h1.symm⟩)
    · exact Or.inr (Or.inr ⟨e, he, Or.inr h2.symm⟩)

theorem inPlay_claim {G : Graph} {f : File} (hf : f ∈ G) : P.InPlay G (P.claim f).2 :=
  Or.inl ((P.claimedKey_iff).mpr ⟨f, hf, rfl⟩)

theorem inPlay_of_pagesR {r : Bool} {G : Graph} {p : Page K} (h : p ∈ P.pagesR r G) : P.InPlay G p.2 := by
  rcases (P.mem_pagesR).mp h with h | ⟨_, h, _⟩
  · obtain ⟨f, hf, e⟩ := (P.real_iff).mp h
    exact e ▸ P.inPlay_claim hf
  · exact Or.inr (Or.inl h)

/-! ## The blocks of a page -/

/-- The blocks of page `p`: those of its owner file, in outline order (none for a reference-only page). -/
def pageBlocks (G : Graph) (p : Page K) : List BlockId :=
  match P.owner G p with
  | none => []
  | some f => (List.range (P.outline f.format f.text).length).map (f.path, ·)

theorem mem_pageBlocks {G : Graph} {p : Page K} {b : BlockId} :
    b ∈ P.pageBlocks G p ↔ ∃ f, P.owner G p = some f ∧ b.1 = f.path ∧ b.2 < (P.outline f.format f.text).length := by
  unfold pageBlocks
  cases h : P.owner G p with
  | none => simp
  | some f =>
    obtain ⟨π, i⟩ := b
    simp only [List.mem_map, List.mem_range, Prod.mk.injEq, Option.some.injEq]
    constructor
    · rintro ⟨j, hj, e1, e2⟩; exact ⟨f, rfl, e1.symm, e2 ▸ hj⟩
    · rintro ⟨g, rfl, e1, hi⟩; exact ⟨i, hi, e1.symm, rfl⟩

/-- The blocks of a page are blocks of the graph. -/
theorem pageBlocks_sub_blocks {G : Graph} {p : Page K} {b : BlockId} (h : b ∈ P.pageBlocks G p) :
    b ∈ P.blocks G := by
  obtain ⟨f, ho, e1, hi⟩ := (P.mem_pageBlocks).mp h
  obtain ⟨hf, hio, _⟩ := (P.owner_eq_some_iff).mp ho
  exact (P.mem_blocks).mpr ⟨f, hf, hio, e1.symm, hi⟩

/-! ## References through keys -/

/-- References restricted to the occurrences whose kind passes `keep`. With `keep = fun _ => true`
this is `References` (`references_iff_refsBy`). -/
def RefsBy (keep : OccKind → Bool) (G : Graph) (b : BlockId) (t : Page K) : Prop :=
  ∃ o ∈ P.occAt G b, keep o.1 = true ∧ ∃ u ∈ P.occClosure o.2, P.aliasClass G (P.key u) t.2

theorem references_iff_keys {G : Graph} {b : BlockId} {t : Page K} :
    P.References G b t ↔ ∃ s ∈ P.refNames G b, P.aliasClass G (P.key s) t.2 := by
  unfold References refs
  constructor
  · rintro ⟨s, hs, e⟩
    refine ⟨s, hs, ?_⟩
    rw [P.concept_eq_class] at e
    exact (cls_eq_iff _).mp e.symm
  · rintro ⟨s, hs, h⟩
    refine ⟨s, hs, ?_⟩
    rw [P.concept_eq_class]
    exact ((cls_eq_iff _).mpr h).symm

theorem mem_refNames {G : Graph} {b : BlockId} {s : String} :
    s ∈ P.refNames G b ↔ ∃ o ∈ P.occAt G b, s ∈ P.occClosure o.2 := by
  simp [refNames]

theorem references_iff_refsBy {G : Graph} {b : BlockId} {t : Page K} :
    P.References G b t ↔ P.RefsBy (fun _ => true) G b t := by
  rw [P.references_iff_keys]
  simp only [RefsBy, P.mem_refNames, true_and]
  constructor
  · rintro ⟨s, ⟨o, ho, hs⟩, h⟩; exact ⟨o, ho, s, hs, h⟩
  · rintro ⟨o, ho, s, hs, h⟩; exact ⟨s, ⟨o, ho, hs⟩, h⟩

/-- References split into those through covered and through uncovered occurrence kinds. -/
theorem references_split (c : OccKind → Bool) {G : Graph} {b : BlockId} {t : Page K} :
    P.References G b t ↔ P.RefsBy c G b t ∨ P.RefsBy (fun k => !c k) G b t := by
  rw [P.references_iff_refsBy]
  unfold RefsBy
  constructor
  · rintro ⟨o, ho, _, h⟩
    cases hc : c o.1
    · exact Or.inr ⟨o, ho, by simp [hc], h⟩
    · exact Or.inl ⟨o, ho, hc, h⟩
  · rintro (⟨o, ho, _, h⟩ | ⟨o, ho, _, h⟩) <;> exact ⟨o, ho, rfl, h⟩

end Params
end CM
