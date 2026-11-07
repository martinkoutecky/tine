/- Conflicts are about distinct files claiming a KEY, across both kinds.
Aliases never contribute claims. Duplicate list entries do not create conflicts. -/
import Links -- Build ordering: at most two concurrent module compilers.
set_option linter.unusedSectionVars false

namespace CM
namespace Params
variable {K : Type} [DecidableEq K] (P : Params K)

def Conflicted (G : Graph) (k : K) : Prop :=
  ∃ f ∈ G, ∃ g ∈ G, f ≠ g ∧ (P.claim f).2 = k ∧ (P.claim g).2 = k

def conflictedB (G : Graph) (k : K) : Bool :=
  G.any (fun f => G.any (fun g =>
    decide (f ≠ g ∧ (P.claim f).2 = k ∧ (P.claim g).2 = k)))

theorem conflictedB_iff {G : Graph} {k : K} :
    P.conflictedB G k = true ↔ P.Conflicted G k := by
  simp [conflictedB, Conflicted]

instance {G : Graph} {k : K} : Decidable (P.Conflicted G k) :=
  decidable_of_iff (P.conflictedB G k = true) P.conflictedB_iff

def conflictKeys (G : Graph) : List K :=
  ((G.map (fun f => (P.claim f).2)).eraseDups).filter (P.conflictedB G)

def ConflictFree (G : Graph) : Prop := ∀ k, ¬ P.Conflicted G k

def conflictFreeB (G : Graph) : Bool := (P.conflictKeys G).isEmpty

theorem mem_conflictKeys {G : Graph} {k : K} :
    k ∈ P.conflictKeys G ↔ P.Conflicted G k := by
  simp only [conflictKeys, List.mem_filter, List.mem_eraseDups, List.mem_map,
    P.conflictedB_iff]
  exact ⟨And.right, fun h => ⟨by obtain ⟨f, hf, _, _, _, e, _⟩ := h; exact ⟨f, hf, e⟩, h⟩⟩

theorem conflictFreeB_iff {G : Graph} : P.conflictFreeB G = true ↔ P.ConflictFree G := by
  simp only [conflictFreeB, ConflictFree, ← P.mem_conflictKeys, List.isEmpty_iff]
  constructor
  · intro h k; rw [h]; simp
  · intro h
    cases he : P.conflictKeys G with
    | nil => rfl
    | cons k ks => exact False.elim (h k (by rw [he]; simp))

theorem conflict_sameFiles {G H : Graph} (h : SameFiles G H) (k : K) :
    P.Conflicted G k ↔ P.Conflicted H k := by
  simp only [Conflicted, show ∀ f, f ∈ G ↔ f ∈ H from h]

theorem conflictKeys_sameFiles {G H : Graph} (h : SameFiles G H) (k : K) :
    k ∈ P.conflictKeys G ↔ k ∈ P.conflictKeys H := by
  rw [P.mem_conflictKeys, P.mem_conflictKeys, P.conflict_sameFiles h]

/-- At a clean key the file order is irrelevant: there is at most one distinct claimant. -/
theorem clean_claim_unique {G : Graph} {f g : File} (hc : ¬ P.Conflicted G (P.claim f).2)
    (hf : f ∈ G) (hg : g ∈ G) (he : (P.claim f).2 = (P.claim g).2) : f = g := by
  by_cases hn : f = g
  · exact hn
  · exact False.elim (hc ⟨f, hf, g, hg, hn, rfl, he.symm⟩)

/-- Thus the legacy rename order law follows from conflict freedom, without assuming any
stability of the order under rewriting. This implication is used only where needed. -/
theorem conflictFree_order (hP : P.Lawful) {G : Graph} (hc : P.ConflictFree G) (R : Rewriter)
    (m : String → String) :
    ∀ f ∈ G, ∀ g ∈ G, P.claim f = P.claim g →
      P.fileLe (R.apply m f) (R.apply m g) = P.fileLe f g := by
  intro f hf g hg he
  rw [P.clean_claim_unique (hc _) hf hg (congrArg Prod.snd he)]
  have hr : ∀ f, P.fileLe f f = true := fun f => (hP.file.total f f).elim id id
  rw [hr, hr]

end Params
end CM
