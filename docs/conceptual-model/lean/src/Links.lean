/-
Links between pages (SPEC §6.2, D-20261018-T1) and T6.

`Link r fav G S T`: `S` and `T` are pages of `G` (reading `r` of P1), some block of `S` references `T`
(SPEC §4.4, through alias classes), `S ≠ T`, and `S` is not the Favorites page `fav` (a parameter: the
model does not fix which page that is). `out`, `in`, `near` are the `k`-step neighbourhoods (at most
`k` steps, seeds included at `k = 0`); `near … both = out ∪ in`, not the undirected ball.
-/
import Query -- Build ordering: at most two concurrent module compilers.

set_option linter.unusedSectionVars false

namespace CM
namespace Params

variable {K : Type} [DecidableEq K] (P : Params K)

/-- SPEC §6.2. -/
def Link (r : Bool) (fav : Page K) (G : Graph) (S T : Page K) : Prop :=
  S ∈ P.pagesR r G ∧ T ∈ P.pagesR r G ∧ S ≠ T ∧ S ≠ fav ∧ ∃ b ∈ P.pageBlocks G S, P.References G b T

def linkB (r : Bool) (fav : Page K) (G : Graph) (S T : Page K) : Bool :=
  decide (S ∈ P.pagesR r G) && decide (T ∈ P.pagesR r G) && decide (S ≠ T) && decide (S ≠ fav) &&
    (P.pageBlocks G S).any (fun b => P.referencesB G b T)

theorem linkB_iff {r : Bool} {fav : Page K} {G : Graph} {S T : Page K} :
    P.linkB r fav G S T = true ↔ P.Link r fav G S T := by
  unfold linkB Link
  simp only [Bool.and_eq_true, decide_eq_true_eq, List.any_eq_true, P.referencesB_iff]
  constructor
  · rintro ⟨⟨⟨⟨a, b⟩, c⟩, d⟩, e⟩; exact ⟨a, b, c, d, e⟩
  · rintro ⟨a, b, c, d, e⟩; exact ⟨⟨⟨⟨a, b⟩, c⟩, d⟩, e⟩

/-! ## Neighbourhoods -/

/-- At most `k` steps along `link` from a seed. -/
def OutP (r : Bool) (fav : Page K) (G : Graph) : Nat → List (Page K) → Page K → Prop
  | 0, X, T => T ∈ X
  | k + 1, X, T => OutP r fav G k X T ∨ ∃ S, OutP r fav G k X S ∧ P.Link r fav G S T

/-- At most `k` steps along the converse of `link`. -/
def InP (r : Bool) (fav : Page K) (G : Graph) : Nat → List (Page K) → Page K → Prop
  | 0, X, T => T ∈ X
  | k + 1, X, T => InP r fav G k X T ∨ ∃ S, InP r fav G k X S ∧ P.Link r fav G T S

inductive NearDir | out | inn | both
  deriving DecidableEq, Repr

def NearP (r : Bool) (fav : Page K) (G : Graph) : NearDir → Nat → List (Page K) → Page K → Prop
  | .out, k, X, T => P.OutP r fav G k X T
  | .inn, k, X, T => P.InP r fav G k X T
  | .both, k, X, T => P.OutP r fav G k X T ∨ P.InP r fav G k X T

/-- The undirected ball (for contrast only: `near … both` is *not* this). -/
def BallP (r : Bool) (fav : Page K) (G : Graph) : Nat → List (Page K) → Page K → Prop
  | 0, X, T => T ∈ X
  | k + 1, X, T => BallP r fav G k X T ∨ ∃ S, BallP r fav G k X S ∧ (P.Link r fav G S T ∨ P.Link r fav G T S)

/-- Executable neighbourhoods. -/
def outL (r : Bool) (fav : Page K) (G : Graph) : Nat → List (Page K) → List (Page K)
  | 0, X => X
  | k + 1, X => let Y := outL r fav G k X; Y ++ (P.pagesR r G).filter (fun T => Y.any (fun S => P.linkB r fav G S T))

def inL (r : Bool) (fav : Page K) (G : Graph) : Nat → List (Page K) → List (Page K)
  | 0, X => X
  | k + 1, X => let Y := inL r fav G k X; Y ++ (P.pagesR r G).filter (fun T => Y.any (fun S => P.linkB r fav G T S))

def ballL (r : Bool) (fav : Page K) (G : Graph) : Nat → List (Page K) → List (Page K)
  | 0, X => X
  | k + 1, X => let Y := ballL r fav G k X;
      Y ++ (P.pagesR r G).filter (fun T => Y.any (fun S => P.linkB r fav G S T || P.linkB r fav G T S))

def nearL (r : Bool) (fav : Page K) (G : Graph) : NearDir → Nat → List (Page K) → List (Page K)
  | .out, k, X => P.outL r fav G k X
  | .inn, k, X => P.inL r fav G k X
  | .both, k, X => P.outL r fav G k X ++ P.inL r fav G k X

variable {P}

theorem mem_outL {r : Bool} {fav : Page K} {G : Graph} (k : Nat) (X : List (Page K)) (T : Page K) :
    T ∈ P.outL r fav G k X ↔ P.OutP r fav G k X T := by
  induction k generalizing T with
  | zero => rfl
  | succ k ih =>
    simp only [outL, OutP, List.mem_append, List.mem_filter, List.any_eq_true, P.linkB_iff, ih]
    constructor
    · rintro (h | ⟨_, S, hS, hl⟩)
      · exact Or.inl h
      · exact Or.inr ⟨S, hS, hl⟩
    · rintro (h | ⟨S, hS, hl⟩)
      · exact Or.inl h
      · exact Or.inr ⟨hl.2.1, S, hS, hl⟩

theorem mem_inL {r : Bool} {fav : Page K} {G : Graph} (k : Nat) (X : List (Page K)) (T : Page K) :
    T ∈ P.inL r fav G k X ↔ P.InP r fav G k X T := by
  induction k generalizing T with
  | zero => rfl
  | succ k ih =>
    simp only [inL, InP, List.mem_append, List.mem_filter, List.any_eq_true, P.linkB_iff, ih]
    constructor
    · rintro (h | ⟨_, S, hS, hl⟩)
      · exact Or.inl h
      · exact Or.inr ⟨S, hS, hl⟩
    · rintro (h | ⟨S, hS, hl⟩)
      · exact Or.inl h
      · exact Or.inr ⟨hl.1, S, hS, hl⟩

theorem mem_ballL {r : Bool} {fav : Page K} {G : Graph} (k : Nat) (X : List (Page K)) (T : Page K) :
    T ∈ P.ballL r fav G k X ↔ P.BallP r fav G k X T := by
  induction k generalizing T with
  | zero => rfl
  | succ k ih =>
    simp only [ballL, BallP, List.mem_append, List.mem_filter, List.any_eq_true, Bool.or_eq_true,
      P.linkB_iff, ih]
    constructor
    · rintro (h | ⟨_, S, hS, hl⟩)
      · exact Or.inl h
      · exact Or.inr ⟨S, hS, hl⟩
    · rintro (h | ⟨S, hS, hl⟩)
      · exact Or.inl h
      · refine Or.inr ⟨?_, S, hS, hl⟩
        rcases hl with hl | hl
        · exact hl.2.1
        · exact hl.1

theorem mem_nearL {r : Bool} {fav : Page K} {G : Graph} (d : NearDir) (k : Nat) (X : List (Page K)) (T : Page K) :
    T ∈ P.nearL r fav G d k X ↔ P.NearP r fav G d k X T := by
  cases d with
  | out => exact mem_outL k X T
  | inn => exact mem_inL k X T
  | both => simp only [nearL, NearP, List.mem_append, mem_outL, mem_inL]

/-- **T6 (near both).** `near X k both = out k X ∪ in k X`. -/
theorem near_both {r : Bool} {fav : Page K} {G : Graph} (k : Nat) (X : List (Page K)) (T : Page K) :
    P.NearP r fav G .both k X T ↔ P.OutP r fav G k X T ∨ P.InP r fav G k X T := Iff.rfl

theorem outP_succ {r : Bool} {fav : Page K} {G : Graph} {k : Nat} {X : List (Page K)} {T : Page K}
    (h : P.OutP r fav G k X T) : P.OutP r fav G (k + 1) X T := Or.inl h

theorem inP_succ {r : Bool} {fav : Page K} {G : Graph} {k : Nat} {X : List (Page K)} {T : Page K}
    (h : P.InP r fav G k X T) : P.InP r fav G (k + 1) X T := Or.inl h

/-- **T6 (monotone).** `near` is monotone in `k`, in every direction. -/
theorem near_mono {r : Bool} {fav : Page K} {G : Graph} (d : NearDir) {k k' : Nat} (hk : k ≤ k')
    (X : List (Page K)) (T : Page K) : P.NearP r fav G d k X T → P.NearP r fav G d k' X T := by
  induction hk with
  | refl => exact id
  | step _ ih =>
    intro h
    have := ih h
    cases d with
    | out => exact outP_succ this
    | inn => exact inP_succ this
    | both => rcases this with h | h; exact Or.inl (outP_succ h); exact Or.inr (inP_succ h)

/-! ## T6: links depend on the target only through its class (and its own page) -/

/-- **T6 (class).** Two target pages of one alias class are linked from the same pagesLegacyA, except for the
`S ≠ T` clause, which reads the page itself. -/
theorem link_class {r : Bool} {fav : Page K} {G : Graph} {S T T' : Page K} (hT : T ∈ P.pagesR r G)
    (hT' : T' ∈ P.pagesR r G) (hc : P.aliasClass G T.2 = P.aliasClass G T'.2) (hs : S = T ↔ S = T') :
    P.Link r fav G S T ↔ P.Link r fav G S T' := by
  unfold Link References
  rw [hc]
  constructor
  · rintro ⟨a, _, c, d, e⟩; exact ⟨a, hT', fun h => c (hs.mpr h), d, e⟩
  · rintro ⟨a, _, c, d, e⟩; exact ⟨a, hT, fun h => c (hs.mp h), d, e⟩

/-- **T6 (spelling).** Naming the target by any spelling of one key gives the same links. -/
theorem link_key {r : Bool} {fav : Page K} {G : Graph} {S : Page K} {s t : String} (h : P.key s = P.key t) :
    P.Link r fav G S (P.resolve G s) ↔ P.Link r fav G S (P.resolve G t) := by
  rw [(P.resolve_key G h).1]

/-! ## T6: links under rename -/

variable {R : Rewriter} {m : String → String} {σ : K → K}

/-- **T6 (rename), replacement claim.** Under T3's hypotheses, with the whole graph exception-free and
the Favorites page neither moved nor a rename target, `link` is carried by `σp`. -/
theorem link_rename {r : Bool} {fav : Page K} {G : Graph} (H : P.RenameHyps R m σ G)
    (hx : P.GraphExceptionFree R σ G) (hfav : ∀ p ∈ P.pagesR r G, σp σ p = fav ↔ p = fav)
    {S T : Page K} (hS : S ∈ P.pagesR r G) (hT : T ∈ P.pagesR r G) :
    P.Link r fav G S T ↔ P.Link r fav (renameBy R m G) (σp σ S) (σp σ T) := by
  have hSp := P.inPlay_of_pagesR hS
  have hTp := P.inPlay_of_pagesR hT
  unfold Link
  rw [pageBlocks_rename H hSp]
  have hne : S ≠ T ↔ σp σ S ≠ σp σ T :=
    ⟨fun h e => h (pagesR_rename_inj H hS hT e), fun h e => h (e ▸ rfl)⟩
  have hf : S ≠ fav ↔ σp σ S ≠ fav := ⟨fun h e => h ((hfav S hS).mp e), fun h e => h ((hfav S hS).mpr e)⟩
  have hr : (∃ b ∈ P.pageBlocks G S, P.References G b T) ↔
      ∃ b' ∈ (P.pageBlocks G S).map (blockMap R m G), P.References (renameBy R m G) b' (σp σ T) := by
    constructor
    · rintro ⟨b, hb, hrf⟩
      have hbB := P.pageBlocks_sub_blocks hb
      exact ⟨_, List.mem_map_of_mem hb, (references_rename H hbB (hx b hbB) hTp).mp hrf⟩
    · rintro ⟨b', hb', hrf⟩
      obtain ⟨b, hb, rfl⟩ := List.mem_map.mp hb'
      have hbB := P.pageBlocks_sub_blocks hb
      exact ⟨b, hb, (references_rename H hbB (hx b hbB) hTp).mpr hrf⟩
  rw [hne, hf, hr]
  constructor
  · rintro ⟨_, _, c, d, e⟩
    exact ⟨pagesR_rename_fwd H hx hS, pagesR_rename_fwd H hx hT, c, d, e⟩
  · rintro ⟨_, _, c, d, e⟩
    exact ⟨hS, hT, c, d, e⟩

/-- Every link of the renamed graph is the image of a link of `G`. -/
theorem link_rename_onto {r : Bool} {fav : Page K} {G : Graph} (H : P.RenameHyps R m σ G)
    (hx : P.GraphExceptionFree R σ G) (hfav : ∀ p ∈ P.pagesR r G, σp σ p = fav ↔ p = fav)
    {S' T' : Page K} (h : P.Link r fav (renameBy R m G) S' T') :
    ∃ S ∈ P.pagesR r G, ∃ T ∈ P.pagesR r G, σp σ S = S' ∧ σp σ T = T' ∧ P.Link r fav G S T := by
  obtain ⟨S, hS, rfl⟩ := pagesR_rename_onto H hx h.1
  obtain ⟨T, hT, rfl⟩ := pagesR_rename_onto H hx h.2.1
  exact ⟨S, hS, T, hT, rfl, rfl, (link_rename H hx hfav hS hT).mpr h⟩

/-- **T6 (near under rename).** For seeds and targets among the pages of `G`, the neighbourhoods are
carried by `σp`, in every direction. -/
theorem near_rename {r : Bool} {fav : Page K} {G : Graph} (H : P.RenameHyps R m σ G)
    (hx : P.GraphExceptionFree R σ G) (hfav : ∀ p ∈ P.pagesR r G, σp σ p = fav ↔ p = fav)
    {X : List (Page K)} (hX : ∀ x ∈ X, x ∈ P.pagesR r G) (d : NearDir) (k : Nat) {T : Page K}
    (hT : T ∈ P.pagesR r G) :
    P.NearP r fav G d k X T ↔ P.NearP r fav (renameBy R m G) d k (X.map (σp σ)) (σp σ T) := by
  have base : ∀ T ∈ P.pagesR r G, (T ∈ X ↔ σp σ T ∈ X.map (σp σ)) := fun T hT => by
    constructor
    · exact List.mem_map_of_mem
    · intro h
      obtain ⟨x, hx', e⟩ := List.mem_map.mp h
      rw [← pagesR_rename_inj H (hX x hx') hT e]; exact hx'
  have hout : ∀ k, ∀ T ∈ P.pagesR r G,
      (P.OutP r fav G k X T ↔ P.OutP r fav (renameBy R m G) k (X.map (σp σ)) (σp σ T)) := by
    intro k
    induction k with
    | zero => exact base
    | succ k ih =>
      intro T hT
      simp only [OutP]
      rw [ih T hT]
      constructor
      · rintro (h | ⟨S, hS, hl⟩)
        · exact Or.inl h
        · exact Or.inr ⟨σp σ S, (ih S hl.1).mp hS, (link_rename H hx hfav hl.1 hT).mp hl⟩
      · rintro (h | ⟨S', hS', hl⟩)
        · exact Or.inl h
        · obtain ⟨S, hS, rfl⟩ := pagesR_rename_onto H hx hl.1
          exact Or.inr ⟨S, (ih S hS).mpr hS', (link_rename H hx hfav hS hT).mpr hl⟩
  have hin : ∀ k, ∀ T ∈ P.pagesR r G,
      (P.InP r fav G k X T ↔ P.InP r fav (renameBy R m G) k (X.map (σp σ)) (σp σ T)) := by
    intro k
    induction k with
    | zero => exact base
    | succ k ih =>
      intro T hT
      simp only [InP]
      rw [ih T hT]
      constructor
      · rintro (h | ⟨S, hS, hl⟩)
        · exact Or.inl h
        · exact Or.inr ⟨σp σ S, (ih S hl.2.1).mp hS, (link_rename H hx hfav hT hl.2.1).mp hl⟩
      · rintro (h | ⟨S', hS', hl⟩)
        · exact Or.inl h
        · obtain ⟨S, hS, rfl⟩ := pagesR_rename_onto H hx hl.2.1
          exact Or.inr ⟨S, (ih S hS).mpr hS', (link_rename H hx hfav hT hS).mpr hl⟩
  cases d with
  | out => exact hout k T hT
  | inn => exact hin k T hT
  | both => simp only [NearP]; rw [hout k T hT, hin k T hT]

end Params
end CM
