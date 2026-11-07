/-
Membership characterisations of the model's derived lists, and T1: resolution is well defined.
-/
import Model

set_option linter.unusedSectionVars false

namespace CM
namespace Params

variable {K : Type} [DecidableEq K] (P : Params K)

/-! ## Membership lemmas (no laws needed) -/

theorem mem_claimants {G : Graph} {p : Page K} {f : File} :
    f ∈ P.claimants G p ↔ f ∈ G ∧ P.claim f = p := by
  simp [claimants]

theorem real_iff {G : Graph} {p : Page K} : P.real G p = true ↔ ∃ f ∈ G, P.claim f = p := by
  simp [real]

/-- Round 9: a solid page is a real page one of whose claimants has content. -/
theorem solid_iff {G : Graph} {p : Page K} :
    P.solid G p = true ↔ ∃ f ∈ G, P.claim f = p ∧ P.contentFree f = false := by
  simp [solid]

theorem solid_real {G : Graph} {p : Page K} (h : P.solid G p = true) : P.real G p = true := by
  obtain ⟨f, hf, hc, _⟩ := P.solid_iff.mp h
  exact P.real_iff.mpr ⟨f, hf, hc⟩

theorem owner_spec {G : Graph} {p : Page K} {f : File} (h : P.owner G p = some f) :
    f ∈ G ∧ P.claim f = p :=
  (P.mem_claimants).mp (minBy_mem h)

theorem owner_eq_some_iff {G : Graph} {p : Page K} {f : File} :
    P.owner G p = some f ↔ f ∈ G ∧ P.isOwner G f = true ∧ P.claim f = p := by
  constructor
  · intro h
    obtain ⟨h1, h2⟩ := P.owner_spec h
    refine ⟨h1, ?_, h2⟩
    simp [isOwner, h2, h]
  · rintro ⟨_, h2, h3⟩
    simp [isOwner, h3] at h2
    exact h2

theorem mem_declaringOwners {G : Graph} {f : File} :
    f ∈ P.declaringOwners G ↔ f ∈ G ∧ P.isOwner G f = true ∧ P.aliasKinds (kindOf f) = true := by
  simp [declaringOwners]

theorem mem_aliasEdges {G : Graph} {a b : K} :
    (a, b) ∈ P.aliasEdges G ↔
      ∃ f ∈ P.declaringOwners G, (P.claim f).2 = a ∧ ∃ s ∈ P.aliasDecl f.format f.text, P.key s = b := by
  simp only [aliasEdges, List.mem_flatMap, List.mem_map, Prod.mk.injEq]
  constructor
  · rintro ⟨f, hf, s, hs, h1, h2⟩; exact ⟨f, hf, h1, s, hs, h2⟩
  · rintro ⟨f, hf, h1, s, hs, h2⟩; exact ⟨f, hf, s, hs, h1, h2⟩

theorem declaresKey_iff {f : File} {k : K} :
    P.declaresKey f k = true ↔ ∃ s ∈ P.aliasDecl f.format f.text, P.key s = k := by
  simp [declaresKey]

theorem mem_declarers {G : Graph} {k : K} {q : Page K} :
    q ∈ P.declarers G k ↔ ∃ f ∈ P.declaringOwners G, P.declaresKey f k = true ∧ P.claim f = q := by
  simp only [declarers, List.mem_map, List.mem_filter]
  constructor
  · rintro ⟨f, ⟨hf, hd⟩, hq⟩; exact ⟨f, hf, hd, hq⟩
  · rintro ⟨f, hf, hd, hq⟩; exact ⟨f, ⟨hf, hd⟩, hq⟩

/-- A direct declarer of `k` is joined to `k` by an alias edge. -/
theorem declarer_edge {G : Graph} {k : K} {q : Page K} (h : q ∈ P.declarers G k) :
    (q.2, k) ∈ P.aliasEdges G := by
  obtain ⟨f, hf, hd, hq⟩ := (P.mem_declarers).mp h
  obtain ⟨s, hs, hk⟩ := (P.declaresKey_iff).mp hd
  exact (P.mem_aliasEdges).mpr ⟨f, hf, by rw [hq], s, hs, hk⟩

/-- The resolved page's key is in the class of the spelling's key: `key s ∈ concept s`. -/
theorem key_mem_concept (G : Graph) (s : String) : P.concept G s (P.key s) := by
  unfold concept aliasClass cls resolve resolveKey
  split
  · exact Reach.refl _
  · split
    · exact Reach.refl _
    · split
      · next q hq => exact Reach.symm (Reach.edge' (P.declarer_edge (minBy_mem hq)))
      · exact Reach.refl _

/-- `concept s` is the alias class of `key s` itself. -/
theorem concept_eq_class (G : Graph) (s : String) : P.concept G s = P.aliasClass G (P.key s) :=
  (cls_eq_iff _).mpr (P.key_mem_concept G s)

theorem conceptB_iff (G : Graph) (s : String) (k : K) : P.conceptB G s k = true ↔ P.concept G s k :=
  sameCls_iff _ _ _

theorem referencesB_iff (G : Graph) (b : BlockId) (t : Page K) :
    P.referencesB G b t = true ↔ P.References G b t := by
  unfold referencesB References refs
  rw [List.any_eq_true]
  constructor
  · rintro ⟨s, hs, h⟩
    exact ⟨s, hs, ((cls_eq_iff _).mpr ((sameCls_iff _ _ _).mp h)).symm⟩
  · rintro ⟨s, hs, h⟩
    refine ⟨s, hs, (sameCls_iff _ _ _).mpr ?_⟩
    have : P.aliasClass G (P.resolve G s).2 = cls (P.aliasEdges G) t.2 := h.symm
    exact (cls_eq_iff _).mp this

/-! ## T1: dependence on the graph only as a set of files -/

/-- Two lists represent the same set of files. -/
def SameFiles (G₁ G₂ : Graph) : Prop := ∀ f, f ∈ G₁ ↔ f ∈ G₂

variable {P}

theorem owner_congr (hP : P.Lawful) {G₁ G₂ : Graph} (h : SameFiles G₁ G₂) (p : Page K) :
    P.owner G₁ p = P.owner G₂ p :=
  minBy_congr hP.file (fun f => by rw [P.mem_claimants, P.mem_claimants, h f])

theorem isOwner_congr (hP : P.Lawful) {G₁ G₂ : Graph} (h : SameFiles G₁ G₂) (f : File) :
    P.isOwner G₁ f = P.isOwner G₂ f := by
  simp only [isOwner, owner_congr hP h]

theorem real_congr {G₁ G₂ : Graph} (h : SameFiles G₁ G₂) (p : Page K) : P.real G₁ p = P.real G₂ p := by
  apply Bool.eq_iff_iff.mpr
  rw [P.real_iff, P.real_iff]
  exact ⟨fun ⟨f, hf, e⟩ => ⟨f, (h f).mp hf, e⟩, fun ⟨f, hf, e⟩ => ⟨f, (h f).mpr hf, e⟩⟩

theorem solid_congr {G₁ G₂ : Graph} (h : SameFiles G₁ G₂) (p : Page K) :
    P.solid G₁ p = P.solid G₂ p := by
  apply Bool.eq_iff_iff.mpr
  rw [P.solid_iff, P.solid_iff]
  exact ⟨fun ⟨f, hf, e⟩ => ⟨f, (h f).mp hf, e⟩, fun ⟨f, hf, e⟩ => ⟨f, (h f).mpr hf, e⟩⟩

theorem declaringOwners_congr (hP : P.Lawful) {G₁ G₂ : Graph} (h : SameFiles G₁ G₂) :
    SameFiles (P.declaringOwners G₁) (P.declaringOwners G₂) := fun f => by
  rw [P.mem_declaringOwners, P.mem_declaringOwners, h f, isOwner_congr hP h]

theorem aliasEdges_congr (hP : P.Lawful) {G₁ G₂ : Graph} (h : SameFiles G₁ G₂) (e : K × K) :
    e ∈ P.aliasEdges G₁ ↔ e ∈ P.aliasEdges G₂ := by
  obtain ⟨a, b⟩ := e
  rw [P.mem_aliasEdges, P.mem_aliasEdges]
  have hd := declaringOwners_congr hP h
  exact ⟨fun ⟨f, hf, r⟩ => ⟨f, (hd f).mp hf, r⟩, fun ⟨f, hf, r⟩ => ⟨f, (hd f).mpr hf, r⟩⟩

theorem declarers_congr (hP : P.Lawful) {G₁ G₂ : Graph} (h : SameFiles G₁ G₂) (k : K) (q : Page K) :
    q ∈ P.declarers G₁ k ↔ q ∈ P.declarers G₂ k := by
  rw [P.mem_declarers, P.mem_declarers]
  have hd := declaringOwners_congr hP h
  exact ⟨fun ⟨f, hf, r⟩ => ⟨f, (hd f).mp hf, r⟩, fun ⟨f, hf, r⟩ => ⟨f, (hd f).mpr hf, r⟩⟩

/-- **T1 (resolve, set).** `resolve` is a function of the graph as a set of files. -/
theorem resolve_sameFiles (hP : P.Lawful) {G₁ G₂ : Graph} (h : SameFiles G₁ G₂) (s : String) :
    P.resolve G₁ s = P.resolve G₂ s := by
  unfold resolve resolveKey
  rw [solid_congr h, solid_congr h, real_congr h, real_congr h,
    minBy_congr hP.page (declarers_congr hP h (P.key s))]

theorem aliasClass_congr (hP : P.Lawful) {G₁ G₂ : Graph} (h : SameFiles G₁ G₂) (k : K) :
    P.aliasClass G₁ k = P.aliasClass G₂ k := by
  funext y; apply propext
  exact Reach.congr (aliasEdges_congr hP h)

/-- **T1 (concept, set).** `concept` is a function of the graph as a set of files. -/
theorem concept_sameFiles (hP : P.Lawful) {G₁ G₂ : Graph} (h : SameFiles G₁ G₂) (s : String) :
    P.concept G₁ s = P.concept G₂ s := by
  unfold concept
  rw [resolve_sameFiles hP h, aliasClass_congr hP h]

/-- **T1 (order).** In particular neither depends on the listing order of the files. -/
theorem resolve_perm (hP : P.Lawful) {G₁ G₂ : Graph} (h : G₁.Perm G₂) (s : String) :
    P.resolve G₁ s = P.resolve G₂ s ∧ P.concept G₁ s = P.concept G₂ s :=
  ⟨resolve_sameFiles hP (fun _ => h.mem_iff) s, concept_sameFiles hP (fun _ => h.mem_iff) s⟩

/-- **T1 (spelling).** `resolve` and `concept` depend on a spelling only through `key`. (Immediate
from the definition, once "declares `s`" is read by key; see the notes.) -/
theorem resolve_key (G : Graph) {s t : String} (h : P.key s = P.key t) :
    P.resolve G s = P.resolve G t ∧ P.concept G s = P.concept G t := by
  unfold concept resolve; rw [h]; exact ⟨rfl, rfl⟩

/-- The executable forms agree too (they are Boolean, so this is an equality of values). -/
theorem conceptB_sameFiles (hP : P.Lawful) {G₁ G₂ : Graph} (h : SameFiles G₁ G₂) (s : String) (k : K) :
    P.conceptB G₁ s k = P.conceptB G₂ s k := by
  apply Bool.eq_iff_iff.mpr
  rw [P.conceptB_iff, P.conceptB_iff, concept_sameFiles hP h]

/-! ## Block lookups (need unique paths) -/

theorem eq_of_path_eq {G : Graph} (hw : WF G) {f g : File} (hf : f ∈ G) (hg : g ∈ G)
    (h : f.path = g.path) : f = g := by
  induction G with
  | nil => simp at hf
  | cons a t ih =>
    unfold WF at hw
    rw [List.map_cons, List.nodup_cons] at hw
    obtain ⟨hn, ht⟩ := hw
    rcases List.mem_cons.mp hf with hf' | hf' <;> rcases List.mem_cons.mp hg with hg' | hg'
    · rw [hf', hg']
    · rw [hf'] at h; exact absurd (h ▸ List.mem_map_of_mem hg') hn
    · rw [hg'] at h; exact absurd (h.symm ▸ List.mem_map_of_mem hf') hn
    · exact ih ht hf' hg'

theorem fileAt_some {G : Graph} {p : String} {f : File} (h : fileAt G p = some f) :
    f ∈ G ∧ f.path = p := by
  unfold fileAt at h
  exact ⟨List.mem_of_find?_eq_some h, by simpa using List.find?_some h⟩

theorem fileAt_none {G : Graph} {p : String} (h : fileAt G p = none) : ∀ f ∈ G, f.path ≠ p := by
  unfold fileAt at h
  intro f hf e
  have := List.find?_eq_none.mp h f hf
  simp [e] at this

theorem fileAt_eq_of_mem {G : Graph} (hw : WF G) {f : File} (hf : f ∈ G) : fileAt G f.path = some f := by
  cases h : fileAt G f.path with
  | none => exact absurd rfl (fileAt_none h f hf)
  | some g =>
    obtain ⟨hg, hp⟩ := fileAt_some h
    rw [eq_of_path_eq hw hg hf hp]

/-- `fileAt` agrees on two well-formed graphs with the same files at path `p`. -/
theorem fileAt_congr {G₁ G₂ : Graph} (hw₂ : WF G₂) {p : String}
    (h : ∀ f, f.path = p → (f ∈ G₁ ↔ f ∈ G₂)) : fileAt G₁ p = fileAt G₂ p := by
  cases h1 : fileAt G₁ p with
  | none =>
    cases h2 : fileAt G₂ p with
    | none => rfl
    | some g =>
      obtain ⟨hg, hp⟩ := fileAt_some h2
      exact absurd hp (fileAt_none h1 g ((h g hp).mpr hg))
  | some f =>
    obtain ⟨hf, hp⟩ := fileAt_some h1
    have := fileAt_eq_of_mem hw₂ ((h f hp).mp hf)
    rw [hp] at this; exact this.symm

theorem occAt_sameFiles {G₁ G₂ : Graph} (hw₂ : WF G₂) (h : SameFiles G₁ G₂) (b : BlockId) :
    P.occAt G₁ b = P.occAt G₂ b := by
  unfold occAt; rw [fileAt_congr hw₂ (fun f _ => h f)]

/-- **T1 (references, set).** The Linked References relation is a function of the set of files. -/
theorem references_sameFiles (hP : P.Lawful) {G₁ G₂ : Graph} (hw₂ : WF G₂)
    (h : SameFiles G₁ G₂) (b : BlockId) (t : Page K) : P.References G₁ b t ↔ P.References G₂ b t := by
  unfold References refs refNames
  rw [occAt_sameFiles hw₂ h, aliasClass_congr hP h]
  simp only [concept_sameFiles hP h]

end Params
end CM
