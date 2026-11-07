/- Public round-4 entry points. The old modules remain available for round-1/2
proofs; use THIS namespace for the decided model. All lists denote membership sets. -/
import Demo -- Build ordering: at most two concurrent module compilers.
import Conflicts
import Query3
import Links

namespace CM
namespace Decided
open Params
variable {K : Type} [DecidableEq K] (P : Params K)

def pages (G : Graph) : List (Page K) := P.pagesR true G
def nodes (G : Graph) : List (Node K) :=
  (pages P G).map .page ++ (P.blocks G).map .block

/-- Navigation is total even for unknown keys. Q9 removes alias-only keys from the
finite page universe, not from the domain of navigation. AL2 is still pageLe. -/
def resolve (G : Graph) (s : String) : Page K := P.resolve G s

/-- CM3-2: the source exclusion follows this configured spelling's KEY, across
both kinds. Renaming a file never edits this configuration. -/
structure Config where
  favoritesPage : String
  deriving DecidableEq, Repr

def Link (config : Config) (G : Graph) (S T : Page K) :=
  P.Link true (S.1, P.key config.favoritesPage) G S T
def linkB (config : Config) (G : Graph) (S T : Page K) :=
  P.linkB true (S.1, P.key config.favoritesPage) G S T

theorem link_config_iff (config : Config) (G : Graph) (S T : Page K) :
    Link P config G S T ↔ S ∈ pages P G ∧ T ∈ pages P G ∧ S ≠ T ∧
      S.2 ≠ P.key config.favoritesPage ∧
      ∃ b ∈ P.pageBlocks G S, P.References G b T := by
  simp [Link, Params.Link, pages, Prod.ext_iff]

theorem linkB_iff (config : Config) (G : Graph) (S T : Page K) :
    linkB P config G S T = true ↔ Link P config G S T := P.linkB_iff

/-- At most `k` steps along `link` from a seed. -/
def OutP (fav : Config) (G : Graph) : Nat → List (Page K) → Page K → Prop
  | 0, X, T => T ∈ X
  | k + 1, X, T => OutP fav G k X T ∨ ∃ S, OutP fav G k X S ∧ Link P fav G S T

/-- At most `k` steps along the converse of `link`. -/
def InP (fav : Config) (G : Graph) : Nat → List (Page K) → Page K → Prop
  | 0, X, T => T ∈ X
  | k + 1, X, T => InP fav G k X T ∨ ∃ S, InP fav G k X S ∧ Link P fav G T S

def NearP (fav : Config) (G : Graph) : Params.NearDir → Nat → List (Page K) → Page K → Prop
  | .out, k, X, T => OutP P fav G k X T
  | .inn, k, X, T => InP P fav G k X T
  | .both, k, X, T => OutP P fav G k X T ∨ InP P fav G k X T

/-- Executable neighbourhoods. -/
def outL (fav : Config) (G : Graph) : Nat → List (Page K) → List (Page K)
  | 0, X => X
  | k + 1, X => let Y := outL fav G k X; Y ++ (pages P G).filter (fun T => Y.any (fun S => linkB P fav G S T))

def inL (fav : Config) (G : Graph) : Nat → List (Page K) → List (Page K)
  | 0, X => X
  | k + 1, X => let Y := inL fav G k X; Y ++ (pages P G).filter (fun T => Y.any (fun S => linkB P fav G T S))

def nearL (fav : Config) (G : Graph) : Params.NearDir → Nat → List (Page K) → List (Page K)
  | .out, k, X => outL P fav G k X
  | .inn, k, X => inL P fav G k X
  | .both, k, X => outL P fav G k X ++ inL P fav G k X



theorem mem_outL {fav : Config} {G : Graph} (k : Nat) (X : List (Page K)) (T : Page K) :
    T ∈ outL P fav G k X ↔ OutP P fav G k X T := by
  induction k generalizing T with
  | zero => rfl
  | succ k ih =>
    simp only [outL, OutP, List.mem_append, List.mem_filter, List.any_eq_true, linkB_iff, ih]
    constructor
    · rintro (h | ⟨_, S, hS, hl⟩)
      · exact Or.inl h
      · exact Or.inr ⟨S, hS, hl⟩
    · rintro (h | ⟨S, hS, hl⟩)
      · exact Or.inl h
      · exact Or.inr ⟨hl.2.1, S, hS, hl⟩

theorem mem_inL {fav : Config} {G : Graph} (k : Nat) (X : List (Page K)) (T : Page K) :
    T ∈ inL P fav G k X ↔ InP P fav G k X T := by
  induction k generalizing T with
  | zero => rfl
  | succ k ih =>
    simp only [inL, InP, List.mem_append, List.mem_filter, List.any_eq_true, linkB_iff, ih]
    constructor
    · rintro (h | ⟨_, S, hS, hl⟩)
      · exact Or.inl h
      · exact Or.inr ⟨S, hS, hl⟩
    · rintro (h | ⟨S, hS, hl⟩)
      · exact Or.inl h
      · exact Or.inr ⟨hl.1, S, hS, hl⟩

theorem mem_nearL {fav : Config} {G : Graph} (d : Params.NearDir) (k : Nat) (X : List (Page K)) (T : Page K) :
    T ∈ nearL P fav G d k X ↔ NearP P fav G d k X T := by
  cases d with
  | out => exact mem_outL P k X T
  | inn => exact mem_inL P k X T
  | both => simp only [nearL, NearP, List.mem_append, mem_outL, mem_inL]

/-- **T6 (near both).** `near X k both = out k X ∪ in k X`. -/
theorem near_both {fav : Config} {G : Graph} (k : Nat) (X : List (Page K)) (T : Page K) :
    NearP P fav G .both k X T ↔ OutP P fav G k X T ∨ InP P fav G k X T := Iff.rfl

theorem outP_succ {fav : Config} {G : Graph} {k : Nat} {X : List (Page K)} {T : Page K}
    (h : OutP P fav G k X T) : OutP P fav G (k + 1) X T := Or.inl h

theorem inP_succ {fav : Config} {G : Graph} {k : Nat} {X : List (Page K)} {T : Page K}
    (h : InP P fav G k X T) : InP P fav G (k + 1) X T := Or.inl h

/-- **T6 (monotone).** `near` is monotone in `k`, in every direction. -/
theorem near_mono {fav : Config} {G : Graph} (d : Params.NearDir) {k k' : Nat} (hk : k ≤ k')
    (X : List (Page K)) (T : Page K) : NearP P fav G d k X T → NearP P fav G d k' X T := by
  induction hk with
  | refl => exact id
  | step _ ih =>
    intro h
    have := ih h
    cases d with
    | out => exact outP_succ P this
    | inn => exact inP_succ P this
    | both => rcases this with h | h; exact Or.inl (outP_succ P h); exact Or.inr (inP_succ P h)


structure Meaning where
  pageSet : List (Page K)
  blockSet : List BlockId
  aliases : List (K × K)
  conflicts : List K
  owner : Page K → Option File
  navigate : String → Page K
  occurrences : BlockId → List (OccKind × String)
  references : BlockId → Page K → Bool
  links : Page K → Page K → Bool

def meaning (config : Config) (G : Graph) : Meaning (K := K) :=
  ⟨pages P G, P.blocks G, P.aliasEdges G, P.conflictKeys G,
   P.owner G, P.resolve G, P.occAt G, P.referencesB G, linkB P config G⟩

def den {L : Type} (Q : QParams K L) (G : Graph) (a : Anchor) (q : Query L) :=
  P.den3 Q true G a q

/-- Permanent exceptions M2(4). Stored query positions are rewritten under the
separate round-6 contract (D-20261031-CMQ); block references remain outside OccKind. -/
def covers : OccKind → Bool
  | .taskMarker | .priority => false
  | _ => true

/-- Text rewriting is still given, as are the parser-facing extractors. Unlike
the round-2 Rewriter, its occurrence coverage is fixed by the decisions. -/
structure Rewriter where
  apply : (String → String) → File → File

def Rewriter.base (R : Rewriter) : CM.Rewriter := ⟨covers, R.apply⟩

/-- Parser-facing contract, with journals explicitly stationary. A text rewriter
is given for the same reason the occurrence extractor is given. These are laws
to check at the fixture boundary, never assumptions about reference preservation. -/
structure FaithfulOn (R : Rewriter) (m : String → String) (G : Graph) : Prop where
  dir : ∀ f ∈ G, (R.apply m f).dir = f.dir
  format : ∀ f ∈ G, (R.apply m f).format = f.format
  name : ∀ f ∈ G, P.key (P.name (R.apply m f)) =
    if kindOf f = .page then P.key (m (P.name f)) else P.key (P.name f)
  path : ∀ f ∈ G, (kindOf f = .journal ∨ P.key (m (P.name f)) = P.key (P.name f)) →
    (R.apply m f).path = f.path
  occ : ∀ f ∈ G, ((P.outline f.format f.text).map (P.occ f.format)).map
    (List.map (rwOcc covers m)) =
      (P.outline (R.apply m f).format (R.apply m f).text).map (P.occ (R.apply m f).format)
  alias : ∀ f ∈ G, P.aliasDecl (R.apply m f).format (R.apply m f).text =
    (P.aliasDecl f.format f.text).map m

/-- On clean graphs owner order stability follows, rather than being assumed.
The explicit journal condition is NOT implied by conflict freedom: a journal
may live strictly below old without sharing a key with any page. -/
theorem faithful_rwLaws (hP : P.Lawful) {R : Rewriter} {m : String → String}
    {σ : K → K} {G : Graph} (hc : P.ConflictFree G) (F : FaithfulOn P R m G)
    (hn : ∀ f ∈ G, P.key (m (P.name f)) = σ (P.claim f).2)
    (hj : ∀ f ∈ G, kindOf f = .journal → σ (P.claim f).2 = (P.claim f).2) :
    P.RwLaws R.base m σ G where
  dir := F.dir
  format := F.format
  name := by
    intro f hf
    change P.key (P.name (R.apply m f)) = σ (P.claim f).2
    rw [F.name f hf]
    split
    · exact hn f hf
    · next h =>
      have hk : kindOf f = .journal := by cases he : kindOf f <;> simp_all
      exact (hj f hf hk).symm
  path := by
    intro f hf hfix
    exact F.path f hf (Or.inr ((hn f hf).trans hfix))
  occ := F.occ
  alias := F.alias
  order := P.conflictFree_order hP hc R.base m

/-- Every name in play, including namespace prefixes and alias-only keys.
The old and new roots are checked separately even when never referenced. -/
def usedNames (G : Graph) : List String :=
  G.map P.name ++ G.flatMap (fun f => P.aliasDecl f.format f.text) ++
    (P.blocks G).flatMap (P.refNames G) ++ G.flatMap P.fileNsNames

def movedNames (G : Graph) (old : String) : List String :=
  old :: (usedNames P G).filter (fun s => (nsMatch P.key P.urlHasOrigin old s).isSome)

/-- Alias-only metadata is rewritten but is not a moved PAGE under Q9. In
particular, rewriting an alias member onto an existing real key is permitted by
AL1. Real-target collision refusal applies to actual moved pages only. -/
def movedPageNames (G : Graph) (old : String) : List String :=
  (movedNames P G old).filter (fun s =>
    (pages P G).any (fun p => decide (p.2 = P.key s)))

/-- Round 7 (D-20261102-CM4): rule 6.4's alias-class clause. It refuses only when
`new` names a real page OTHER than the source, i.e. a key different from `old`'s
that some file claims and that lies in `old`'s alias class. A same-key rename
(`Foo` → `foo`) is never refused by this clause. For a different key it is the
round-6 clause verbatim (`CaseRename7.core_eq_legacy`). -/
def classClash (G : Graph) (old new : String) : Bool :=
  decide (P.key new ≠ P.key old) && P.claimedKey G (P.key new) &&
    sameCls (P.aliasEdges G) (P.key old) (P.key new)

/-- `renameAllowedLegacy` with the round-7 class clause: the source is a real
page-kind root; no file outside the moved subtree claims a moved file's
destination; `new` is no OTHER real page of the source's alias class. -/
def renameCoreAllowed (G : Graph) (old new : String) : Bool :=
  P.real G (.page, P.key old) &&
  G.all (fun g => (decide (kindOf g = .page) && (nsMatch P.key P.urlHasOrigin old (P.name g)).isSome) ||
    (P.movedFiles G old).all (fun f =>
      decide ((P.claim g).2 ≠ P.key (nsSubst P.key P.urlHasOrigin old new (P.name f))))) &&
  !classClash P G old new

/-- Round 7: in a same-key rename, a moved spelling whose key the substitution
leaves unchanged still names exactly the page it named; it cannot land on a
page it did not already name, so the real-target clause does not count it. -/
def keepsKey (old new s : String) : Bool :=
  decide (P.key new = P.key old) && decide (P.key (nsSubst P.key P.urlHasOrigin old new s) = P.key s)

/-- Check all moved keys and destinations, including reference-only children.
Reference-only targets are allowed. Real targets outside the subtree are refused.
We also refuse collisions between DISTINCT moved real keys after substitution;
K1 alone does not make namespace substitution injective on keys.
Round 7: the first conjunct is `renameCoreAllowed` (was `renameAllowedLegacy`)
and the last clause skips `keepsKey` spellings; for `key new ≠ key old` the
guard is the round-6 guard verbatim (`CaseRename7.base_eq_r6`). -/
def renameBaseAllowed (G : Graph) (old new : String) : Bool :=
  renameCoreAllowed P G old new &&
  (movedNames P G old).all (fun s =>
    !P.conflictedB G (P.key s) &&
    !P.conflictedB G (P.key (nsSubst P.key P.urlHasOrigin old new s))) &&
  !P.conflictedB G (P.key new) &&
  (P.movedFiles G old).all (fun f => (P.movedFiles G old).all (fun g =>
    decide (P.key (nsSubst P.key P.urlHasOrigin old new (P.name f)) =
      P.key (nsSubst P.key P.urlHasOrigin old new (P.name g)) → (P.claim f).2 = (P.claim g).2))) &&
  G.all (fun g => decide (g ∈ P.movedFiles G old) ||
    (movedPageNames P G old).all (fun s =>
      decide ((P.claim g).2 ≠ P.key (nsSubst P.key P.urlHasOrigin old new s)) || keepsKey P old new s))

/-- Ownership here is a real journal claim OR a direct alias declaration of a
journal owner enabled by O4. Alias reachability alone is not ownership; a
non-owner's ignored alias metadata is not ownership. -/
def journalOwnedKeys (G : Graph) : List K :=
  (G.filter (fun f => kindOf f == .journal)).map (fun f => (P.claim f).2) ++
  ((P.declaringOwners G).filter (fun f => kindOf f == .journal)).flatMap
    (fun f => (P.aliasDecl f.format f.text).map P.key)

def JournalOwns (G : Graph) (k : K) : Prop :=
  (∃ f ∈ G, kindOf f = .journal ∧ (P.claim f).2 = k) ∨
  ∃ f ∈ P.declaringOwners G, kindOf f = .journal ∧
    ∃ s ∈ P.aliasDecl f.format f.text, P.key s = k

theorem mem_journalOwnedKeys {G : Graph} {k : K} :
    k ∈ journalOwnedKeys P G ↔ JournalOwns P G k := by
  simp [journalOwnedKeys, JournalOwns, and_assoc]

/-- Per occurrence, keys of old namespace spellings absent from its rewritten
closure. Uncovered occurrences lose no key. Prefix loss is included even when
the prefix is outside the renamed subtree; retention as a new prefix is safe.
This is a syntactic spelling check, not an alias-class preservation claim. -/
def journalRewriteRisks (G : Graph) (old new : String) : List K :=
  (P.blocks G).flatMap (fun b => (P.occAt G b).flatMap (fun o =>
    ((P.occClosure o.2).map P.key).filter (fun k =>
      decide (k ∈ journalOwnedKeys P G) &&
      !decide (k ∈ (P.occClosure (rwOcc covers (nsSubst P.key P.urlHasOrigin old new) o).2).map P.key))))

def JournalRisk (G : Graph) (old new : String) : Prop :=
  ∃ b ∈ P.blocks G, ∃ o ∈ P.occAt G b, ∃ k,
    k ∈ (P.occClosure o.2).map P.key ∧ JournalOwns P G k ∧
    k ∉ (P.occClosure (rwOcc covers (nsSubst P.key P.urlHasOrigin old new) o).2).map P.key

theorem journalRewriteRisks_nonempty {G : Graph} {old new : String} :
    (journalRewriteRisks P G old new).isEmpty = false ↔ JournalRisk P G old new := by
  simp only [List.isEmpty_eq_false_iff_exists_mem, journalRewriteRisks,
    List.mem_flatMap, List.mem_filter, Bool.and_eq_true, decide_eq_true_eq,
    Bool.not_eq_true', decide_eq_false_iff_not, mem_journalOwnedKeys]
  constructor
  · rintro ⟨k, b, hb, o, ho, hk, hj, hn⟩
    exact ⟨b, hb, o, ho, k, hk, hj, hn⟩
  · rintro ⟨b, hb, o, ho, k, hk, hj, hn⟩
    exact ⟨k, b, hb, o, ho, hk, hj, hn⟩

/-- Round 9, CM-OWN-CHILD (D-20261103-CMA): `new` lies strictly below `old`
in the namespace (`nsMatch` finds `old` as a prefix and the keys differ) and
`new` is already used: claimed by a file of either kind (a real page) or
declared as an alias by some enabled owner. Reference-only use does not count
(round-9 call C9-1). -/
def ownChildUsed (G : Graph) (old new : String) : Bool :=
  (nsMatch P.key P.urlHasOrigin old new).isSome && decide (P.key new ≠ P.key old) &&
    (P.claimedKey G (P.key new) || P.declaredKey G (P.key new))

/-- The round-8 guard, kept verbatim for historical witnesses. -/
def renameAllowedR8 (G : Graph) (old new : String) : Bool :=
  renameBaseAllowed P G old new && (journalRewriteRisks P G old new).isEmpty

/-- Round 9: the round-8 guard plus the CM-OWN-CHILD refusal. -/
def renameAllowed (G : Graph) (old new : String) : Bool :=
  renameBaseAllowed P G old new && (journalRewriteRisks P G old new).isEmpty &&
    !ownChildUsed P G old new

/-- An executable, refusing rename. Journal paths/names must be kept by the
given rewriter's laws; its occurrences are still rewritten. -/
def rename (R : Rewriter) (G : Graph) (old new : String) : Option Graph :=
  if renameAllowed P G old new then some (P.rename R.base G old new) else none

/-- A favorite is a page-name entry, not a file claimant. Its subtree follows the
same substitution; raw entries need not already be in the page universe. -/
def renameFavorites (old new : String) (favorites : List String) : List String :=
  favorites.map (nsSubst P.key P.urlHasOrigin old new)

def renameState (R : Rewriter) (G : Graph) (favorites : List String) (old new : String) :
    Option (Graph × List String) :=
  (rename P R G old new).map (fun H => (H, renameFavorites P old new favorites))

structure State where
  graph : Graph
  config : Config
  favorites : List String
  deriving DecidableEq, Repr

def renameConfiguredState (R : Rewriter) (state : State) (old new : String) : Option State :=
  (rename P R state.graph old new).map (fun H =>
    ⟨H, state.config, renameFavorites P old new state.favorites⟩)

theorem rename_config_fixed {R : Rewriter} {s t : State} {old new : String}
    (h : renameConfiguredState P R s old new = some t) : t.config = s.config := by
  obtain ⟨H, _, ht⟩ := Option.map_eq_some_iff.mp h
  cases ht
  rfl

/-- Exact additional refusal beyond the pre-existing guard.
Round 9: a third disjunct, the CM-OWN-CHILD refusal (ruling-driven). -/
theorem rename_none_iff (R : Rewriter) (G : Graph) (old new : String) :
    rename P R G old new = none ↔
      renameBaseAllowed P G old new = false ∨ JournalRisk P G old new ∨
        ownChildUsed P G old new = true := by
  have hn : rename P R G old new = none ↔ renameAllowed P G old new = false := by
    unfold rename
    split <;> simp_all
  rw [hn, renameAllowed, Bool.and_eq_false_iff, Bool.and_eq_false_iff,
    journalRewriteRisks_nonempty P, Bool.not_eq_false', or_assoc]

/-- Round 9: the journal refusal is exact once the base guard passes and the
CM-OWN-CHILD clause does not fire (`hc`, new hypothesis, ruling-driven). -/
theorem rename_journal_exact (R : Rewriter) {G : Graph} {old new : String}
    (hb : renameBaseAllowed P G old new = true)
    (hc : ownChildUsed P G old new = false) :
    rename P R G old new = none ↔ JournalRisk P G old new := by
  rw [rename_none_iff]; simp [hb, hc]

/-- Round 9: the decided guard is the round-8 guard with CM-OWN-CHILD added. -/
theorem renameAllowed_eq_r8 (G : Graph) (old new : String) :
    renameAllowed P G old new = (renameAllowedR8 P G old new && !ownChildUsed P G old new) := rfl

/-- Round 9, CM-OWN-CHILD for rename: renaming onto a used strict namespace
descendant is refused. -/
theorem rename_own_child_refused (R : Rewriter) {G : Graph} {old new : String}
    (h : ownChildUsed P G old new = true) : rename P R G old new = none :=
  (rename_none_iff P R G old new).mpr (Or.inr (Or.inr h))

/-- Every accepted rename retains every directly journal-owned closure key in
the predicted rewrite of each individual occurrence. Actual text follows this
prediction under FaithfulOn; alias-class preservation is a separate question. -/
theorem rename_success_journal_support {R : Rewriter} {G H : Graph} {old new : String}
    (h : rename P R G old new = some H) {b : BlockId} (hb : b ∈ P.blocks G)
    {o : OccKind × String} (ho : o ∈ P.occAt G b) {k : K}
    (hk : k ∈ (P.occClosure o.2).map P.key) (hj : JournalOwns P G k) :
    k ∈ (P.occClosure (rwOcc covers (nsSubst P.key P.urlHasOrigin old new) o).2).map P.key := by
  by_cases hn : k ∈ (P.occClosure (rwOcc covers (nsSubst P.key P.urlHasOrigin old new) o).2).map P.key
  · exact hn
  · have hr : JournalRisk P G old new := ⟨b, hb, o, ho, k, hk, hj, hn⟩
    have he := (rename_none_iff P R G old new).mpr (Or.inr (Or.inl hr))
    rw [h] at he
    cases he

theorem rename_eq_some {R : Rewriter} {G : Graph} {old new : String} :
    rename P R G old new = some (P.rename R.base G old new) ↔
      renameAllowed P G old new = true := by
  simp [rename]

theorem rename_success_graph {R : Rewriter} {G H : Graph} {old new : String}
    (h : rename P R G old new = some H) : H = P.rename R.base G old new := by
  unfold rename at h
  split at h
  · exact (Option.some.inj h).symm
  · cases h

theorem rename_conflict_refused {G : Graph} {old new s : String}
    (hs : s ∈ movedNames P G old)
    (hc : P.Conflicted G (P.key s) ∨
      P.Conflicted G (P.key (nsSubst P.key P.urlHasOrigin old new s))) (R : Rewriter) :
    rename P R G old new = none := by
  have hn : renameAllowed P G old new ≠ true := by
    intro h
    have ha := h
    simp only [renameAllowed, Bool.and_eq_true] at ha
    have ha := ha.1.1
    simp only [renameBaseAllowed, Bool.and_eq_true] at ha
    have hcheck := List.all_eq_true.mp ha.1.1.1.2 s hs
    simp only [Bool.and_eq_true, Bool.not_eq_true'] at hcheck
    rcases hc with hc | hc
    · have := P.conflictedB_iff.mpr hc; simp_all
    · have := P.conflictedB_iff.mpr hc; simp_all
  simp [rename, hn]

/-- Q9 set dependence, for the decided page universe. -/
theorem pages_sameFiles (hP : P.Lawful) {G H : Graph} (hw : WF H)
    (h : SameFiles G H) (p : Page K) : p ∈ pages P G ↔ p ∈ pages P H := by
  simp only [pages, P.mem_pagesR]
  have hc : P.claimedKey G p.2 = P.claimedKey H p.2 := by
    simp only [claimedKey, real_congr h]
  have hd : P.declaredKey G p.2 = P.declaredKey H p.2 := by
    apply Bool.eq_iff_iff.mpr
    simp only [P.declaredKey_iff, aliasEdges_congr hP h]
  rw [real_congr h, pageSeedKeys_sameFiles P hP hw h, hc, hd]

end Decided
end CM
