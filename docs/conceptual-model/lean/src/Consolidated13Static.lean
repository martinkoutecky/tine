/- Current semantic input: one visible file per claimed key. Historical modules
remain immutable comparison evidence. No duplicate file supplies namespace seeds. -/
import Claimant13

namespace CM
namespace Consolidated13
open Params
variable {K : Type} [DecidableEq K] (P : Inputs K)

def physicalParams : CM.Params K := { P.toParams with fileLe := claimantLe P }

/-- Physical selection uses kind preference, then unsigned UTF-8 path bytes. -/
def openingFile (G : Graph) (k : K) : Option File :=
  minBy (claimantLe P) (G.filter (fun f => decide ((P.claim f).2 = k)))

def activeGraph (G : Graph) : Graph :=
  G.filter (fun f => decide (openingFile P G (P.claim f).2 = some f))

/-- Both kinds declare aliases; direct-declarer navigation ranks current owner
paths. refKind already includes the configured date recognizer. -/
def readParams (G : Graph) : CM.Params K :=
  { physicalParams P with
    aliasKinds := fun _ => true
    pageLe := Owner5.rankLe (Owner5.pathRank (physicalParams P) (activeGraph P G)) P.pageLe }

def pages (G : Graph) := Decided.pages (readParams P G) (activeGraph P G)
def blocks (G : Graph) := (readParams P G).blocks (activeGraph P G)
def resolve (G : Graph) (s : String) :=
  Rulings9.resolveKeyR8 (readParams P G) (activeGraph P G) (P.key s)
def keyReferences (P : CM.Params K) (G : Graph) (b : BlockId) (t : Page K) : Bool :=
  (P.refNames G b).any (fun s => sameCls (P.aliasEdges G) (P.key s) t.2)

theorem keyReferences_eq (P : CM.Params K) (G : Graph) (b : BlockId) (t : Page K) :
    keyReferences P G b t = P.referencesB G b t := by
  apply Bool.eq_iff_iff.mpr
  rw [P.referencesB_iff, P.references_iff_keys]
  simp [keyReferences, List.any_eq_true, sameCls_iff, aliasClass]

def referencesB (G : Graph) := keyReferences (readParams P G) (activeGraph P G)
def den {L : Type} (Q : QParams K L) (G : Graph) :=
  Decided.den (readParams P G) Q (activeGraph P G)
def tree (spell : K → String) (G : Graph) :=
  Rulings11.tree (readParams P G) spell (activeGraph P G)
def namespaceTree (spell : K → String) (G : Graph) :=
  Rulings11.namespaceTree (readParams P G) spell (activeGraph P G)
def links (config : Decided.Config) (G : Graph) (S T : Page K) :=
  decide (S ∈ pages P G) && decide (T ∈ pages P G) && decide (S ≠ T) &&
    decide (S.2 ≠ P.key config.favoritesPage) &&
    ((readParams P G).pageBlocks (activeGraph P G) S).any (fun b => referencesB P G b T)

def walk (U : List (Page K)) (step : Page K → Page K → Bool) : Nat → List (Page K) → List (Page K)
  | 0, X => X
  | k + 1, X => let Y := walk U step k X; Y ++ U.filter (fun T => Y.any (fun S => step S T))

def near (config : Decided.Config) (G : Graph) : NearDir → Nat → List (Page K) → List (Page K)
  | .out, k, X => walk (pages P G) (links P config G) k X
  | .inn, k, X => walk (pages P G) (fun S T => links P config G T S) k X
  | .both, k, X => walk (pages P G) (links P config G) k X ++
      walk (pages P G) (fun S T => links P config G T S) k X

theorem links_eq (config : Decided.Config) (G : Graph) (S T : Page K) :
    links P config G S T = Decided.linkB (readParams P G) config (activeGraph P G) S T := by
  apply Bool.eq_iff_iff.mpr
  simp only [links, Bool.and_eq_true, decide_eq_true_eq, List.any_eq_true,
    referencesB, keyReferences_eq, (readParams P G).referencesB_iff,
    Decided.linkB_iff, Decided.link_config_iff, and_assoc]
  rfl

theorem near_eq (config : Decided.Config) (G : Graph) (d : NearDir) (k : Nat) (X : List (Page K)) :
    near P config G d k X = Decided.nearL (readParams P G) config (activeGraph P G) d k X := by
  have ho : ∀ n Y, walk (pages P G) (links P config G) n Y =
      Decided.outL (readParams P G) config (activeGraph P G) n Y := by
    intro n; induction n with
    | zero => intro Y; rfl
    | succ n ih =>
      intro Y
      simp only [walk, Decided.outL, ih]
      simp only [links_eq, pages]
  have hi : ∀ n Y, walk (pages P G) (fun S T => links P config G T S) n Y =
      Decided.inL (readParams P G) config (activeGraph P G) n Y := by
    intro n; induction n with
    | zero => intro Y; rfl
    | succ n ih =>
      intro Y
      simp only [walk, Decided.inL, ih]
      simp only [links_eq, pages]
  cases d <;> simp [near, Decided.nearL, ho, hi]

theorem active_iff {G : Graph} {f : File} :
    f ∈ activeGraph P G ↔ f ∈ G ∧ openingFile P G (P.claim f).2 = some f := by
  simp [activeGraph]

theorem twin_excluded {G : Graph} {f : File}
    (h : openingFile P G (P.claim f).2 ≠ some f) : f ∉ activeGraph P G := by
  simp [activeGraph, h]

/-- Every meaning operation above uses only activeGraph. In particular the
old all-files name-parent exception is absent. -/
theorem namespace_seed_active {G : Graph} {k : K}
    (h : k ∈ (readParams P G).nameKeys (activeGraph P G)) :
    ∃ f ∈ G, openingFile P G (P.claim f).2 = some f ∧
      k ∈ ((readParams P G).fileNsNames f).map P.key := by
  obtain ⟨f, hf, s, hs, e⟩ := (Params.mem_nameKeys _).mp h
  obtain ⟨hf, ha⟩ := (active_iff P).mp hf
  exact ⟨f, hf, ha, List.mem_map.mpr ⟨s, hs, e⟩⟩

/-- All non-navigation readings agree on this domain. -/
theorem static_overlap {G : Graph} (ha : activeGraph P G = G)
    (hp : readParams P G = P.toParams) :
    pages P G = Decided.pages P.toParams G ∧ blocks P G = P.blocks G ∧
    referencesB P G = P.referencesB G := by
  refine ⟨by simp [pages, ha, hp], by simp [blocks, ha, hp], ?_⟩
  funext b t
  simpa only [referencesB, ha, hp] using keyReferences_eq P G b t

theorem navigation_real {G : Graph} {s : String}
    (h : (readParams P G).real (activeGraph P G) (P.kindPref, P.key s) = true) :
    resolve P G s = (P.kindPref, P.key s) := by
  change Rulings9.resolveKeyR8 (readParams P G) (activeGraph P G) (P.key s) =
    ((readParams P G).kindPref, P.key s)
  unfold Rulings9.resolveKeyR8
  exact if_pos h

theorem navigation_overlap {G : Graph} {s : String}
    (h : (readParams P G).declarers (activeGraph P G) (P.key s) = []) :
    resolve P G s = (readParams P G).resolve (activeGraph P G) s := by
  exact (Rulings9.resolve_eq_r8_undeclared _ h).symm

end Consolidated13
end CM
