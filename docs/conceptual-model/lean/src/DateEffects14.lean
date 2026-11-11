
import AliasWitness14

set_option linter.unusedSectionVars false
namespace CM
namespace Date13
open Params Consolidated13 QueryRename6 Merge7
variable {K L : Type} [DecidableEq K] (P : Inputs K)

structure RefSignature (K : Type) where
  spellings : List String
  targets : List (Page K)
  openings : List (Page K)
  edges : List (Page K × Page K)
  deriving DecidableEq

def signature (D : Dates K) (G : Graph) (b : BlockId) : RefSignature K :=
  ⟨P.refNames (activeGraph P D G) b,refTargets P D G b,
    (P.refNames (activeGraph P D G) b).map (resolve P D G),edges P D G⟩

def images (dates : Dates K) (M : DatePolicy K) (E : Engine) (G : Graph) (old : String) (c : Candidate) (b : BlockId) : List BlockId :=
  match fileAt G b.1 with
  | none => []
  | some f => match source P dates M G old, c.survivor with
    | some S, some D => if f = S ∨ f = D then
        ((List.range (P.outline ((E.merger (subst P dates M old c)).apply (subst P dates M old c) S D).format
          ((E.merger (subst P dates M old c)).apply (subst P dates M old c) S D).text).length).filter (fun i =>
          ((E.merger (subst P dates M old c)).origin (subst P dates M old c) S D i).contains (decide (f = S), b.2))).map (D.path, ·)
      else [(((rewriter E (subst P dates M old c)).apply (subst P dates M old c) f).path, b.2)]
    | _, _ => [(((rewriter E (subst P dates M old c)).apply (subst P dates M old c) f).path, b.2)]

def preview (dates : Dates K) (M : DatePolicy K) (E : Engine) (G : Graph) (old : String) (c : Candidate) : List (BlockId × BlockId) :=
  (blocks P dates G).flatMap (fun b => ((images P dates M E G old c b).filter (fun b' =>
    decide (signature P dates G b ≠ signature P dates (execute P dates M E G old c) b'))).map (b, ·))

/-- Lost and newly activated blocks are separate preview rows. No absent
origin silently erases a reference from the dialog. -/
def lost (dates : Dates K) (M : DatePolicy K) (E : Engine) (G : Graph) (old : String) (c : Candidate) : List BlockId :=
  (blocks P dates G).filter (fun b => (images P dates M E G old c b).isEmpty)
def introduced (dates : Dates K) (M : DatePolicy K) (E : Engine) (G : Graph) (old : String) (c : Candidate) : List BlockId :=
  (blocks P dates (execute P dates M E G old c)).filter (fun b' =>
    !((blocks P dates G).flatMap (images P dates M E G old c)).contains b')

theorem signature_reference (D : Dates K) {G H : Graph} {b b' : BlockId}
    (h : signature P D G b = signature P D H b') (t : Page K) :
    referencesB P D G b t = referencesB P D H b' t := by
  have repr : ∀ A x, referencesB P D A x t =
    (signature P D A x).targets.any (fun v => sameCls (signature P D A x).edges v t) := by intro A x; rfl
  rw [repr G b, repr H b', h]

theorem preview_complete (D : Dates K) (M : DatePolicy K) (E : Engine) {G : Graph} {old : String} {c : Candidate}
    {b b' : BlockId} (hb : b ∈ blocks P D G) (hi : b' ∈ images P D M E G old c b)
    (t : Page K) (h : referencesB P D G b t ≠ referencesB P D (execute P D M E G old c) b' t) :
    (b,b') ∈ preview P D M E G old c := by
  have hn : signature P D G b ≠ signature P D (execute P D M E G old c) b' :=
    fun he => h (signature_reference P D he t)
  simp only [preview, List.mem_flatMap, List.mem_map, List.mem_filter, decide_eq_true_eq]
  exact ⟨b, hb, b', ⟨hi, hn⟩, rfl⟩

/-- Additional date diagnostics are individual path witnesses and primitive
query inputs, never inequality of complete operation outputs. -/

def favoritesAfter (D : Dates K) (M : DatePolicy K) (old : String) (c : Candidate)
    (favorites : Option String) : Option String := favorites.map (subst P D M old c)

def configAfter (D : Dates K) (M : DatePolicy K) (old : String) (c : Candidate)
    (config : Decided.Config) : Decided.Config :=
  { config with favoritesPage := subst P D M old c config.favoritesPage }

theorem favorites_follows (D : Dates K) (M : DatePolicy K) (old : String) (c : Candidate) (s : String) :
    favoritesAfter P D M old c (some s) = some (subst P D M old c s) := rfl

def assembled (D : Dates K) (M : DatePolicy K) (E : Engine) (G : Graph) (old : String) (c : Candidate) : Graph :=
  (kept P D M G old c).map (outputFile P D M E G old c)

theorem assembled_eq (D : Dates K) (M : DatePolicy K) (E : Engine) (G : Graph) (old : String) (c : Candidate) :
    assembled P D M E G old c = execute P D M E G old c := (execute_map P D M E G old c).symm

theorem after_reference_exact (D : Dates K) (M : DatePolicy K) (E : Engine)
    (G : Graph) (old : String) (c : Candidate) (b : BlockId) (t : Page K) :
    referencesB P D (execute P D M E G old c) b t = true ↔
      ∃ s ∈ P.refNames (activeGraph P D (assembled P D M E G old c)) b,
        Reach (edges P D (assembled P D M E G old c))
          (target P D (assembled P D M E G old c) s) t := by
  rw [assembled_eq]; exact reference_exact P D _ b t

theorem after_query_exact (D : Dates K) (M : DatePolicy K) (Q : QParams K L) (R : QueryRowParser)
    (o : QueryPresentation) (E : Engine) (G : Graph) (old : String) (c : Candidate)
    (a : Anchor) (q : Query L) (n : Node K) :
    n ∈ queryResults P D Q R o (execute P D M E G old c) a q ↔
      (n ∈ univ P D (assembled P D M E G old c) a ∧
        q.bval (fun l => leafAt P D Q (assembled P D M E G old c) l n) = true) ∧
      queryRowVisible P D Q R o (assembled P D M E G old c)
        (den P D Q (assembled P D M E G old c) a q) n = true := by
  rw [queryResults_exact,den_exact,assembled_eq]

/-- The actual after-state includes the renamed setting. Every displayed/read
surface is recomputed; no invariant query/alias/namespace answer is promised. -/
theorem offered_recomputed (D : Dates K) (M : DatePolicy K) (N : MemberNames P D) (SP : StoredParser L)
    (E : Engine) {G : Graph} {extras : List String} {old new : String} {c : Candidate}
    (h : c ∈ offers P D M N SP E G extras old new) :
    answer P D M N SP E G extras old new (some c) = some (assembled P D M E G old c) := by
  rw [assembled_eq]; exact offered_executes P D M N SP h

/-- Settings are part of the operation result, not an optional caller action. -/
structure State where
  graph : Graph
  config : Decided.Config

def executeState (D : Dates K) (M : DatePolicy K) (E : Engine) (S : State) (old : String) (c : Candidate) : State :=
  ⟨execute P D M E S.graph old c,configAfter P D M old c S.config⟩

def answerState (D : Dates K) (M : DatePolicy K) (N : MemberNames P D) (SP : StoredParser L) (E : Engine)
    (S : State) (extras : List String) (old new : String) (choice : Option Candidate) : Option State :=
  match choice with
  | none => some S
  | some c => if c ∈ offers P D M N SP E S.graph (extras ++ [S.config.favoritesPage]) old new
    then some (executeState P D M E S old c) else none

theorem state_setting_follows (D : Dates K) (M : DatePolicy K) (E : Engine) (S : State)
    (old : String) (c : Candidate) :
    (executeState P D M E S old c).config.favoritesPage = subst P D M old c S.config.favoritesPage := rfl

theorem state_cancel (D : Dates K) (M : DatePolicy K) (N : MemberNames P D) (SP : StoredParser L) (E : Engine)
    (S : State) (extras : List String) (old new : String) :
    answerState P D M N SP E S extras old new none = some S := rfl

theorem lost_complete (D : Dates K) (M : DatePolicy K) (E : Engine) {G : Graph} {old : String}
    {c : Candidate} {b : BlockId} (hb : b ∈ blocks P D G) (hi : images P D M E G old c b = []) :
    b ∈ lost P D M E G old c := by simp [lost,hb,hi]

theorem introduced_exact (D : Dates K) (M : DatePolicy K) (E : Engine) (G : Graph)
    (old : String) (c : Candidate) (b' : BlockId) :
    b' ∈ introduced P D M E G old c ↔ b' ∈ blocks P D (execute P D M E G old c) ∧
      ¬ ∃ b ∈ blocks P D G, b' ∈ images P D M E G old c b := by
  simp [introduced]

theorem after_count_partition (D : Dates K) (M : DatePolicy K) (favorites : Option String)
    (E : Engine) (G : Graph) (old : String) (c : Candidate) (t : Page K) :
    referenceCount P D (favoritesAfter P D M old c favorites) (execute P D M E G old c) t =
      ((linked P D (favoritesAfter P D M old c favorites) (execute P D M E G old c) t).filter
        (fun b => (linked P D favorites G t).contains b)).length +
      ((linked P D (favoritesAfter P D M old c favorites) (execute P D M E G old c) t).filter
        (fun b => !(linked P D favorites G t).contains b)).length := length_partition _ _

end Date13
end CM
