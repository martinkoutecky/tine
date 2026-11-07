/- O3/O4 decided instance, D-20261026-CM5 and FV2.
Date recognition is a given key predicate: round 4 had no date parser.
Neither real-file priority nor the unresolved owner order is changed. -/
import Headlines -- Scheduling: extend the existing two-compiler DAG serially.
import Witnesses
set_option linter.unusedSectionVars false

namespace CM
namespace Policy5
open Params

def params {K : Type} (P : Params K) (dateKey : K → Bool) : Params K :=
  { P with refKind := fun k => if dateKey k then .journal else P.refKind k,
           aliasKinds := fun _ => true }

variable {K : Type} [DecidableEq K] (P : Params K) (dateKey : K → Bool)

theorem lawful (hP : P.Lawful) : (params P dateKey).Lawful := ⟨hP.file, hP.page⟩

theorem date_kind {k : K} (h : dateKey k = true) :
    (params P dateKey).refKind k = .journal := by simp [params, h]

theorem ordinary_kind {k : K} (h : dateKey k = false) :
    (params P dateKey).refKind k = P.refKind k := by simp [params, h]

theorem aliases_enabled (kind : Kind) :
    (params P dateKey).aliasKinds kind = true := rfl

/-- The formerly opaque refKind-transport condition has this exact meaning.
It is not automatic when a rename crosses the date/non-date boundary. -/
theorem kind_transport_iff (k l : K)
    (hk : dateKey k = false → P.refKind k = .page)
    (hl : dateKey l = false → P.refKind l = .page) :
    (params P dateKey).refKind k = (params P dateKey).refKind l ↔
      dateKey k = dateKey l := by
  cases hd : dateKey k <;> cases he : dateKey l <;> simp_all [params]

theorem seed_kind_transport_iff (G : Graph) (σ : K → K)
    (ordinary : ∀ k, dateKey k = false → P.refKind k = .page) :
    (∀ k ∈ (params P dateKey).pageSeedKeys G,
      (params P dateKey).refKind (σ k) = (params P dateKey).refKind k) ↔
    (∀ k ∈ (params P dateKey).pageSeedKeys G, dateKey (σ k) = dateKey k) := by
  simp only [kind_transport_iff P dateKey _ _ (ordinary _) (ordinary _)]

theorem date_page_iff {G : Graph} {k : K} (hd : dateKey k = true) :
    (Kind.journal, k) ∈ Decided.pages (params P dateKey) G ↔
      (params P dateKey).real G (.journal, k) = true ∨
      (k ∈ (params P dateKey).pageSeedKeys G ∧
       (params P dateKey).claimedKey G k = false ∧
       (params P dateKey).declaredKey G k = false) := by
  unfold Decided.pages
  rw [(params P dateKey).mem_pagesR]
  simp [params, hd]

/-- O3 does not override a real ordinary file with content. No conflict
fallback preference is chosen: absence of a journal claim is explicit.
Round 9 (EMPTY-ALIAS, D-20261103-CMA): restated from `real` to `solid`. The
round-8 statement ("even an empty real ordinary file") is false when the
empty file's key is declared as an alias: the declaring owner then wins
(`Witness9.empty_alias_opens_owner`). The undeclared case is kept verbatim as
`real_page_wins_undeclared`. -/
theorem real_page_wins {G : Graph} {k : K}
    (hp : (params P dateKey).solid G (.page, k) = true)
    (hj : (params P dateKey).real G (.journal, k) = false) :
    (params P dateKey).resolveKey G k = (.page, k) := by
  have hj' : (params P dateKey).solid G (.journal, k) = false := by
    cases h : (params P dateKey).solid G (.journal, k)
    · rfl
    · rw [(params P dateKey).solid_real h] at hj; exact hj
  unfold resolveKey
  cases (params P dateKey).kindPref <;> simp [otherKind, hp, hj, hj']

/-- Round 9: the round-8 statement of `real_page_wins` (an empty real file
still wins), valid when no owner declares the key. -/
theorem real_page_wins_undeclared {G : Graph} {k : K}
    (hp : (params P dateKey).real G (.page, k) = true)
    (hj : (params P dateKey).real G (.journal, k) = false)
    (hd : (params P dateKey).declarers G k = []) :
    (params P dateKey).resolveKey G k = (.page, k) := by
  have hj' : (params P dateKey).solid G (.journal, k) = false := by
    cases h : (params P dateKey).solid G (.journal, k)
    · rfl
    · rw [(params P dateKey).solid_real h] at hj; exact hj
  unfold resolveKey
  rw [hd]
  cases (params P dateKey).kindPref <;> simp [otherKind, hp, hj, hj', minBy]

/-- At O4's decided value, every actual owner journal may contribute its
direct aliases, independently of AL2's navigation choice. -/
theorem journal_owns_iff {G : Graph} {k : K} :
    Decided.JournalOwns (params P dateKey) G k ↔
      (∃ f ∈ G, kindOf f = .journal ∧ (P.claim f).2 = k) ∨
      ∃ f ∈ G, (params P dateKey).isOwner G f = true ∧ kindOf f = .journal ∧
        ∃ s ∈ P.aliasDecl f.format f.text, P.key s = k := by
  simp [Decided.JournalOwns, declaringOwners, params, claim, name, and_assoc]

def pageOnly : Params K :=
  { params P dateKey with aliasKinds := fun k => k == .page }

theorem journal_owns_pageOnly_iff {G : Graph} {k : K} :
    Decided.JournalOwns (pageOnly P dateKey) G k ↔
      ∃ f ∈ G, kindOf f = .journal ∧ (P.claim f).2 = k := by
  simp [Decided.JournalOwns, pageOnly, declaringOwners, params, claim, name,
    and_assoc]
  intro f hf ho hk ha
  cases he : kindOf f <;> simp_all

/-- O3 alone has no effect on CM3-1's *additional* journal risk.
Reference-only journal-kind pages do not become real journal owners. -/
theorem risk_date_independent (otherDate : K → Bool) (G : Graph) (old new : String) :
    Decided.JournalRisk (params P dateKey) G old new ↔
      Decided.JournalRisk (params P otherDate) G old new := Iff.rfl

/-- Enabling journals cannot remove a CM3-1 spelling-loss risk. This does
not assert monotonicity of the entire guard, which also uses Q9 pages. -/
theorem risk_O4_monotone {G : Graph} {old new : String}
    (h : Decided.JournalRisk (pageOnly P dateKey) G old new) :
    Decided.JournalRisk (params P dateKey) G old new := by
  obtain ⟨b, hb, o, ho, k, hk, hj, hn⟩ := h
  exact ⟨b, hb, o, ho, k, hk,
    (journal_owns_iff P dateKey).mpr
      (Or.inl ((journal_owns_pageOnly_iff P dateKey).mp hj)), hn⟩

/-- The O4 enablement premise is discharged for ANY owner. In particular this
applies to journals without a page-only hypothesis or an AL2 owner choice. -/
theorem owner_alias_edge {G : Graph} {f : File} {s : String}
    (hf : f ∈ G) (ho : (params P dateKey).isOwner G f = true)
    (ha : s ∈ P.aliasDecl f.format f.text) :
    ((P.claim f).2, P.key s) ∈ (params P dateKey).aliasEdges G :=
  (params P dateKey).mem_aliasEdges.mpr
    ⟨f, (params P dateKey).mem_declaringOwners.mpr ⟨hf, ho, rfl⟩, rfl, s, ha, rfl⟩

theorem owner_alias_reference {G : Graph} {f : File} {s : String} {b : BlockId}
    (hf : f ∈ G) (ho : (params P dateKey).isOwner G f = true)
    (ha : s ∈ P.aliasDecl f.format f.text)
    (hs : s ∈ (params P dateKey).refNames G b) :
    (params P dateKey).References G b (P.claim f) :=
  (params P dateKey).references_iff_keys.mpr
    ⟨s, hs, Reach.edge' (owner_alias_edge P dateKey hf ho ha)⟩

end Policy5
end CM
