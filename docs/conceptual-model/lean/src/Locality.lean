/-
T4: locality. Two graphs `G`, `G'` that differ only in the file at path `π` (an edit, a creation or a
deletion of that one file: `DiffAt π G G'`). The *touched* files are those at path `π` in either graph
(the file before and after); the *affected keys* are the keys of the pages they claim.
-/
import WellDefined

set_option linter.unusedSectionVars false

namespace CM
namespace Params

variable {K : Type} [DecidableEq K] {P : Params K}

/-- `G` and `G'` have the same files at every path other than `π`. -/
def DiffAt (π : String) (G G' : Graph) : Prop := ∀ g, g.path ≠ π → (g ∈ G ↔ g ∈ G')

theorem DiffAt.symm {π : String} {G G' : Graph} (h : DiffAt π G G') : DiffAt π G' G :=
  fun g hg => (h g hg).symm

variable (P) in
/-- Key `a` is claimed by the file at `π`, before or after. -/
def Affected (π : String) (G G' : Graph) (a : K) : Prop :=
  ∃ h, (h ∈ G ∨ h ∈ G') ∧ h.path = π ∧ (P.claim h).2 = a

variable (P) in
/-- Key `a` is claimed or declared (as an alias) by the file at `π`, before or after: the spec's
literal condition in T4. -/
def ClaimedOrDeclared (π : String) (G G' : Graph) (a : K) : Prop :=
  ∃ h, (h ∈ G ∨ h ∈ G') ∧ h.path = π ∧ ((P.claim h).2 = a ∨ P.declaresKey h a = true)

theorem Affected.symm {π : String} {G G' : Graph} {a : K} (h : P.Affected π G G' a) :
    P.Affected π G' G a := by
  obtain ⟨f, hm, hp, hc⟩ := h; exact ⟨f, hm.symm, hp, hc⟩

/-! ## Owners, blocks and occurrences -/

/-- The owner of a page no touched file claims is the same in both graphs. -/
theorem owner_local (hP : P.Lawful) {π : String} {G G' : Graph} (hD : DiffAt π G G') {p : Page K}
    (hp : ∀ h, (h ∈ G ∨ h ∈ G') → h.path = π → P.claim h ≠ p) : P.owner G p = P.owner G' p := by
  apply minBy_congr hP.file
  intro f
  rw [P.mem_claimants, P.mem_claimants]
  by_cases hf : f.path = π
  · constructor
    · rintro ⟨m, c⟩; exact absurd c (hp f (Or.inl m) hf)
    · rintro ⟨m, c⟩; exact absurd c (hp f (Or.inr m) hf)
  · rw [hD f hf]

/-- **T4 (occurrences).** `occ` changes only on blocks of the file at `π`. -/
theorem occAt_local {π : String} {G G' : Graph} (hw' : WF G') (hD : DiffAt π G G')
    (b : BlockId) (hb : b.1 ≠ π) : P.occAt G b = P.occAt G' b := by
  unfold occAt
  rw [fileAt_congr hw' (fun f hf => hD f (hf ▸ hb))]

/-- **T4 (blocks).** Whether a file other than the one at `π` contributes blocks (is an owner)
changes only if it claims a page that a touched file claims. So the block set can change outside
`π`: a touched file that newly claims (or stops claiming) a page can displace (or restore) another
owner. -/
theorem isOwner_local (hP : P.Lawful) {π : String} {G G' : Graph} (hD : DiffAt π G G') {g : File}
    (hp : ∀ h, (h ∈ G ∨ h ∈ G') → h.path = π → P.claim h ≠ P.claim g) :
    P.isOwner G g = P.isOwner G' g := by
  simp only [isOwner, owner_local hP hD hp]

theorem mem_blocks {G : Graph} {b : BlockId} :
    b ∈ P.blocks G ↔ ∃ f ∈ G, P.isOwner G f = true ∧ f.path = b.1 ∧ b.2 < (P.outline f.format f.text).length := by
  obtain ⟨p, i⟩ := b
  simp only [blocks, List.mem_flatMap]
  constructor
  · rintro ⟨f, hf, hb⟩
    by_cases ho : P.isOwner G f = true
    · rw [if_pos ho] at hb
      simp only [List.mem_map, List.mem_range, Prod.mk.injEq] at hb
      obtain ⟨j, hj, e1, e2⟩ := hb
      exact ⟨f, hf, ho, e1, e2 ▸ hj⟩
    · rw [if_neg ho] at hb; simp at hb
  · rintro ⟨f, hf, ho, e1, hi⟩
    refine ⟨f, hf, ?_⟩
    rw [if_pos ho]
    simp only [List.mem_map, List.mem_range, Prod.mk.injEq]
    exact ⟨i, hi, e1, rfl⟩

theorem blocks_local (hP : P.Lawful) {π : String} {G G' : Graph} (hD : DiffAt π G G') {b : BlockId}
    (hb : b.1 ≠ π)
    (hp : ∀ g, (g ∈ G ∨ g ∈ G') → g.path = b.1 →
      ∀ h, (h ∈ G ∨ h ∈ G') → h.path = π → P.claim h ≠ P.claim g) :
    b ∈ P.blocks G ↔ b ∈ P.blocks G' := by
  rw [mem_blocks, mem_blocks]
  constructor
  · rintro ⟨f, hf, ho, e, hi⟩
    have hfp : f.path ≠ π := e ▸ hb
    refine ⟨f, (hD f hfp).mp hf, ?_, e, hi⟩
    rw [← isOwner_local hP hD (hp f (Or.inl hf) e)]; exact ho
  · rintro ⟨f, hf, ho, e, hi⟩
    have hfp : f.path ≠ π := e ▸ hb
    refine ⟨f, (hD f hfp).mpr hf, ?_, e, hi⟩
    rw [isOwner_local hP hD (hp f (Or.inr hf) e)]; exact ho

/-! ## Alias edges -/

/-- A declaring owner whose page key is not affected stays a declaring owner. -/
theorem declaringOwner_local (hP : P.Lawful) {π : String} {G G' : Graph} (hD : DiffAt π G G')
    {f : File} (hf : f ∈ P.declaringOwners G) (ha : ¬ P.Affected π G G' (P.claim f).2) :
    f ∈ P.declaringOwners G' := by
  obtain ⟨hm, ho, hk⟩ := (P.mem_declaringOwners).mp hf
  have hfp : f.path ≠ π := fun e => ha ⟨f, Or.inl hm, e, rfl⟩
  have hc : ∀ h, (h ∈ G ∨ h ∈ G') → h.path = π → P.claim h ≠ P.claim f :=
    fun h hm' e c => ha ⟨h, hm', e, by rw [c]⟩
  refine (P.mem_declaringOwners).mpr ⟨(hD f hfp).mp hm, ?_, hk⟩
  rw [← isOwner_local hP hD hc]; exact ho

/-- Alias edges whose declaring key is not affected are the same in both graphs. -/
theorem aliasEdges_local (hP : P.Lawful) {π : String} {G G' : Graph} (hD : DiffAt π G G') {a b : K}
    (ha : ¬ P.Affected π G G' a) : (a, b) ∈ P.aliasEdges G ↔ (a, b) ∈ P.aliasEdges G' := by
  rw [P.mem_aliasEdges, P.mem_aliasEdges]
  constructor
  · rintro ⟨f, hf, h1, r⟩
    exact ⟨f, declaringOwner_local hP hD hf (h1 ▸ ha), h1, r⟩
  · rintro ⟨f, hf, h1, r⟩
    exact ⟨f, declaringOwner_local hP hD.symm hf (fun x => ha (h1 ▸ x).symm), h1, r⟩

/-- Generic graph fact: if every edge leaving a key outside `A` is also an edge of `E'`, and the
component of `k` in `E` avoids `A`, then that component is inside the component of `k` in `E'`. -/
theorem reach_transfer {E E' : List (K × K)} {A : K → Prop}
    (hE : ∀ a b, ¬ A a → (a, b) ∈ E → (a, b) ∈ E') {k : K} (hk : ∀ y, Reach E k y → ¬ A y)
    {y : K} (h : Reach E k y) : Reach E' k y := by
  induction h with
  | refl => exact Reach.refl _
  | @fwd u v r e ih => exact Reach.fwd ih (hE _ _ (hk u r) e)
  | @bwd u v r e ih => exact Reach.bwd ih (hE _ _ (hk v (Reach.bwd r e)) e)

/-- **T4 (classes).** If the class of `k` avoids the affected keys before and after, it is the same
class before and after. -/
theorem aliasClass_local (hP : P.Lawful) {π : String} {G G' : Graph} (hD : DiffAt π G G') {k : K}
    (h1 : ∀ y, P.aliasClass G k y → ¬ P.Affected π G G' y)
    (h2 : ∀ y, P.aliasClass G' k y → ¬ P.Affected π G G' y) :
    P.aliasClass G k = P.aliasClass G' k := by
  funext y; apply propext
  constructor
  · exact reach_transfer (fun a b ha e => (aliasEdges_local hP hD ha).mp e) h1
  · exact reach_transfer (fun a b ha e => (aliasEdges_local hP hD ha).mpr e) h2

/-! ## Resolution -/

theorem real_local {π : String} {G G' : Graph} (hD : DiffAt π G G') {k : K}
    (hk : ¬ P.Affected π G G' k) (κ : Kind) : P.real G (κ, k) = P.real G' (κ, k) := by
  apply Bool.eq_iff_iff.mpr
  rw [P.real_iff, P.real_iff]
  constructor
  · rintro ⟨f, hf, c⟩
    have hfp : f.path ≠ π := fun e => hk ⟨f, Or.inl hf, e, by rw [c]⟩
    exact ⟨f, (hD f hfp).mp hf, c⟩
  · rintro ⟨f, hf, c⟩
    have hfp : f.path ≠ π := fun e => hk ⟨f, Or.inr hf, e, by rw [c]⟩
    exact ⟨f, (hD f hfp).mpr hf, c⟩

theorem solid_local {π : String} {G G' : Graph} (hD : DiffAt π G G') {k : K}
    (hk : ¬ P.Affected π G G' k) (κ : Kind) : P.solid G (κ, k) = P.solid G' (κ, k) := by
  apply Bool.eq_iff_iff.mpr
  rw [P.solid_iff, P.solid_iff]
  constructor
  · rintro ⟨f, hf, c, e⟩
    have hfp : f.path ≠ π := fun e' => hk ⟨f, Or.inl hf, e', by rw [c]⟩
    exact ⟨f, (hD f hfp).mp hf, c, e⟩
  · rintro ⟨f, hf, c, e⟩
    have hfp : f.path ≠ π := fun e' => hk ⟨f, Or.inr hf, e', by rw [c]⟩
    exact ⟨f, (hD f hfp).mpr hf, c, e⟩

theorem declarers_local (hP : P.Lawful) {π : String} {G G' : Graph} (hD : DiffAt π G G') {k : K}
    (h1 : ∀ y, P.aliasClass G k y → ¬ P.Affected π G G' y) (q : Page K) :
    q ∈ P.declarers G k → q ∈ P.declarers G' k := by
  intro hq
  have hedge := P.declarer_edge hq
  obtain ⟨f, hf, hd, hfq⟩ := (P.mem_declarers).mp hq
  have hcls : P.aliasClass G k q.2 := Reach.edge' hedge
  have ha : ¬ P.Affected π G G' (P.claim f).2 := by rw [hfq]; exact h1 _ hcls
  exact (P.mem_declarers).mpr ⟨f, declaringOwner_local hP hD hf ha, hd, hfq⟩

/-- **T4 (resolve and concept), sharp form.** If the alias class of `key s` contains no key claimed
by the file at `π`, before or after (in either graph's alias graph), then `resolve s` and
`concept s` are unchanged. -/
theorem resolve_local (hP : P.Lawful) {π : String} {G G' : Graph} (hD : DiffAt π G G') (s : String)
    (h1 : ∀ y, P.aliasClass G (P.key s) y → ¬ P.Affected π G G' y)
    (h2 : ∀ y, P.aliasClass G' (P.key s) y → ¬ P.Affected π G G' y) :
    P.resolve G s = P.resolve G' s ∧ P.concept G s = P.concept G' s := by
  have hk : ¬ P.Affected π G G' (P.key s) := h1 _ (cls_self _ _)
  have hdecl : ∀ q, q ∈ P.declarers G (P.key s) ↔ q ∈ P.declarers G' (P.key s) := fun q =>
    ⟨declarers_local hP hD h1 q,
     declarers_local hP hD.symm (fun y hy => fun a => h2 y hy a.symm) q⟩
  refine ⟨?_, ?_⟩
  · unfold resolve resolveKey
    rw [solid_local hD hk, solid_local hD hk, real_local hD hk, real_local hD hk,
      minBy_congr hP.page hdecl]
  · rw [P.concept_eq_class, P.concept_eq_class]
    exact aliasClass_local hP hD h1 h2

/-- **T4 (resolve and concept), the spec's literal form** (SPEC §7): `resolve s` and `concept s`
change only if `key s` is claimed by the file at `π`, or lies in a class (before or after) containing
a key that file claims or declares. (The first disjunct is subsumed: `key s` is in its own class.) -/
theorem resolve_local_spec (hP : P.Lawful) {π : String} {G G' : Graph} (hD : DiffAt π G G')
    (s : String)
    (h1 : ∀ y, P.aliasClass G (P.key s) y → ¬ P.ClaimedOrDeclared π G G' y)
    (h2 : ∀ y, P.aliasClass G' (P.key s) y → ¬ P.ClaimedOrDeclared π G G' y) :
    P.resolve G s = P.resolve G' s ∧ P.concept G s = P.concept G' s := by
  apply resolve_local hP hD s
  · intro y hy ⟨f, hm, hp, hc⟩; exact h1 y hy ⟨f, hm, hp, Or.inl hc⟩
  · intro y hy ⟨f, hm, hp, hc⟩; exact h2 y hy ⟨f, hm, hp, Or.inl hc⟩

/-! ## The alias-adjacency delta (incremental = rebuild) -/

/-- The affected keys as a list, computed from the touched files only. -/
def affectedKeys (P : Params K) (π : String) (G G' : Graph) : List K :=
  ((G ++ G').filter (fun f => decide (f.path = π))).map (fun f => (P.claim f).2)

theorem mem_affectedKeys {π : String} {G G' : Graph} {a : K} :
    a ∈ affectedKeys P π G G' ↔ P.Affected π G G' a := by
  simp only [affectedKeys, List.mem_map, List.mem_filter, List.mem_append, decide_eq_true_eq, Affected]
  constructor
  · rintro ⟨f, ⟨hm, hp⟩, hc⟩; exact ⟨f, hm, hp, hc⟩
  · rintro ⟨f, hm, hp, hc⟩; exact ⟨f, ⟨hm, hp⟩, hc⟩

/-- Edges contributed by owners of affected keys, for every enabled O4 kind.
It reads G only through owner lookups at those kind/key pairs. -/
def contribEdges (P : Params K) (G : Graph) (ks : List K) : List (K × K) :=
  ks.flatMap (fun a => ([Kind.page, Kind.journal].filter P.aliasKinds).flatMap (fun κ =>
    match P.owner G (κ, a) with
    | some g => (P.aliasDecl g.format g.text).map (fun s => (a, P.key s))
    | none => []))

/-- **The delta.** Given the old adjacency `E` and the affected keys `ks`, drop every edge declared
from an affected key and add the edges the new owners of the affected pages declare. -/
def deltaEdges (P : Params K) (E : List (K × K)) (ks : List K) (G' : Graph) : List (K × K) :=
  E.filter (fun e => !decide (e.1 ∈ ks)) ++ contribEdges P G' ks

theorem mem_contribEdges {G : Graph} {ks : List K} {a b : K} :
    (a, b) ∈ contribEdges P G ks ↔ a ∈ ks ∧ (a, b) ∈ P.aliasEdges G := by
  simp only [contribEdges, List.mem_flatMap]
  constructor
  · rintro ⟨a', ha', κ, hκ, hm⟩
    have hpolicy := (List.mem_filter.mp hκ).2
    cases ho : P.owner G (κ, a') with
    | none => rw [ho] at hm; simp at hm
    | some g =>
      rw [ho] at hm
      simp only [List.mem_map, Prod.mk.injEq] at hm
      obtain ⟨s, hs, e1, e2⟩ := hm
      subst e1
      obtain ⟨hg, ho', hc⟩ := (P.owner_eq_some_iff).mp ho
      refine ⟨ha', (P.mem_aliasEdges).mpr ⟨g, ?_, by rw [hc], s, hs, e2⟩⟩
      apply P.mem_declaringOwners.mpr
      refine ⟨hg, ho', ?_⟩
      have hk : kindOf g = κ := congrArg Prod.fst hc
      rw [hk]; exact hpolicy
  · rintro ⟨ha, he⟩
    obtain ⟨g, hg, h1, s, hs, h2⟩ := (P.mem_aliasEdges).mp he
    obtain ⟨hm, ho, hk⟩ := (P.mem_declaringOwners).mp hg
    refine ⟨a, ha, kindOf g, List.mem_filter.mpr ⟨?_, hk⟩, ?_⟩
    · cases kindOf g <;> simp
    · have hc : P.claim g = (kindOf g, a) := by
        have : P.claim g = (kindOf g, (P.claim g).2) := rfl
        rw [this, h1]
      have hown : P.owner G (kindOf g, a) = some g :=
        (P.owner_eq_some_iff).mpr ⟨hm, ho, hc⟩
      rw [hown]
      simp only [List.mem_map, Prod.mk.injEq]
      exact ⟨s, hs, trivial, h2⟩

/-- **T4 corollary: incremental = rebuild for the alias adjacency.** Updating the old adjacency by
the delta gives exactly the edges of the new graph. -/
theorem delta_eq_rebuild (hP : P.Lawful) {π : String} {G G' : Graph} (hD : DiffAt π G G') (e : K × K) :
    e ∈ deltaEdges P (P.aliasEdges G) (affectedKeys P π G G') G' ↔ e ∈ P.aliasEdges G' := by
  obtain ⟨a, b⟩ := e
  simp only [deltaEdges, List.mem_append, List.mem_filter, Bool.not_eq_true', decide_eq_false_iff_not,
    mem_contribEdges, mem_affectedKeys]
  by_cases ha : P.Affected π G G' a
  · constructor
    · rintro (⟨_, hn⟩ | ⟨_, h⟩)
      · exact absurd ha hn
      · exact h
    · intro h; exact Or.inr ⟨ha, h⟩
  · rw [aliasEdges_local hP hD ha]
    constructor
    · rintro (⟨h, _⟩ | ⟨h, _⟩)
      · exact h
      · exact absurd h ha
    · intro h; exact Or.inl ⟨h, ha⟩

/-- Hence every alias class (and so every concept) computed from the delta-updated adjacency equals
the one rebuilt from `G'`. -/
theorem delta_class_eq_rebuild (hP : P.Lawful) {π : String} {G G' : Graph} (hD : DiffAt π G G') (k : K) :
    cls (deltaEdges P (P.aliasEdges G) (affectedKeys P π G G') G') k = P.aliasClass G' k := by
  funext y; apply propext
  exact Reach.congr (delta_eq_rebuild hP hD)

end Params
end CM
