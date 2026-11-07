/- Literal physical claims; kind-sensitive reference vertices. -/
import PresentationWitness13

set_option linter.unusedSectionVars false
namespace CM
namespace Date13
open Params Consolidated13 QueryRename6
variable {K L : Type} [DecidableEq K] (P : Inputs K)

/-- The date parser consumes a normalized literal spelling and returns the
literal key of the configured canonical journal title. -/
structure Dates (K : Type) where
  canonical : K → Option K := fun _ => none

def target (D : Dates K) (G : Graph) (s : String) : Page K :=
  match openingFile P G (P.key s) with
  | some f => P.claim f
  | none => match D.canonical (P.key s) with
    | some k => (.journal, k)
    | none => (.page, P.key s)

/-- Full (kind,key) vertices distinguish an ordinary canonical-title page from
an absent journal. Explicit aliases can join either kind. -/
def edges (D : Dates K) (G : Graph) : List (Page K × Page K) :=
  (activeGraph P G).flatMap (fun f => (P.aliasDecl f.format f.text).map
    (fun s => (P.claim f, target P D G s)))

/-- Close the raw namespace spelling before interpreting each resulting name. -/
def refTargets (D : Dates K) (G : Graph) (b : BlockId) : List (Page K) :=
  (P.refNames (activeGraph P G) b).map (target P D G)

def referencesB (D : Dates K) (G : Graph) (b : BlockId) (t : Page K) : Bool :=
  (refTargets P D G b).any (fun v => sameCls (edges P D G) v t)

def realFiles (G : Graph) (t : Page K) : Graph :=
  (activeGraph P G).filter (fun f => decide (P.claim f = t))

def declarers (D : Dates K) (G : Graph) (t : Page K) : Graph :=
  (activeGraph P G).filter (fun f => (P.aliasDecl f.format f.text).any
    (fun s => decide (target P D G s = t)))

/-- Direct declarers rank by raw path, independently of their kinds. -/
def resolve (D : Dates K) (G : Graph) (s : String) : Page K :=
  let t := target P D G s
  if !(realFiles P G t).isEmpty then t else
    ((minBy (Owner5.rankLe (fun f => Owner5.bytes f.path) Orders.fileLe)
      (declarers P D G t)).map P.claim).getD t

def seedTargets (D : Dates K) (G : Graph) : List (Page K) :=
  ((Consolidated13.blocks P G).flatMap (refTargets P D G)) ++
    (activeGraph P G).flatMap (fun f => (P.fileNsNames f).map (target P D G))

def pages (D : Dates K) (G : Graph) : List (Page K) :=
  (activeGraph P G).map P.claim ++
    (seedTargets P D G).filter (fun t => (realFiles P G t).isEmpty &&
      !(edges P D G).any (fun e => decide (e.2 = t)))

def panelSelf (D : Dates K) (G : Graph) (t : Page K) : Page K :=
  if !(realFiles P G t).isEmpty then t else
    ((minBy (fun f g => decide (P.name f ≤ P.name g))
      ((activeGraph P G).filter (fun f => sameCls (edges P D G) (P.claim f) t))).map P.claim).getD t

def panelSource (D : Dates K) (favorites : Option String) (G : Graph)
    (t : Page K) (b : BlockId) : Bool :=
  match fileAt (activeGraph P G) b.1 with
  | none => false
  | some f => decide (P.claim f ≠ panelSelf P D G t) &&
      !favorites.any (fun s => decide (P.key s ≠ P.key "") && decide ((P.claim f).2 = P.key s))

def linked (D : Dates K) (favorites : Option String) (G : Graph) (t : Page K) : List BlockId :=
  (Consolidated13.blocks P G).eraseDups.filter (fun b =>
    referencesB P D G b t && panelSource P D favorites G t b)

def referenceCount (D : Dates K) (favorites : Option String) (G : Graph) (t : Page K) : Nat :=
  (linked P D favorites G t).length

def leafPage (D : Dates K) (Q : QParams K L) (G : Graph) : Leaf L → Page K → Bool
  | .refs _, _ => false
  | .name s, t => decide (target P D G s = t)
  | .page l, t => leafPage D Q G l t
  | .given l, t => Q.pageLeaf l (activeGraph P G) t

def pathTargets (D : Dates K) (Q : QParams K L) (G : Graph) (b : BlockId) : List (Page K) :=
  refTargets P D G b ++
    (ancestors Q (activeGraph P G) b).flatMap (refTargets P D G) ++
    ((P.pageOfBlock (activeGraph P G) b).toList)

def leafBlock (D : Dates K) (Q : QParams K L) (G : Graph) : Leaf L → BlockId → Bool
  | .refs s, b => (pathTargets P D Q G b).any (fun t => sameCls (edges P D G) (target P D G s) t)
  | .name _, _ => false
  | .page l, b => ((P.pageOfBlock (activeGraph P G) b).map (leafPage P D Q G l)).getD false
  | .given l, b => Q.blockLeaf l (activeGraph P G) b

def leafAt (D : Dates K) (Q : QParams K L) (G : Graph) (l : Leaf L) : Node K → Bool
  | .page t => leafPage P D Q G l t
  | .block b => leafBlock P D Q G l b

def univ (D : Dates K) (G : Graph) : Anchor → List (Node K)
  | .page => (pages P D G).map Node.page
  | .block => (Consolidated13.blocks P G).map Node.block

def den (D : Dates K) (Q : QParams K L) (G : Graph) (a : Anchor) (q : Query L) : List (Node K) :=
  (univ P D G a).filter (fun n => q.bval (fun l => leafAt P D Q G l n))

def queryResults (D : Dates K) (Q : QParams K L) (R : QueryRowParser) (o : QueryPresentation)
    (G : Graph) (a : Anchor) (q : Query L) : List (Node K) :=
  queryProjection P Q R o G (den P D Q G a q)

def links (D : Dates K) (config : Decided.Config) (G : Graph) (S T : Page K) : Bool :=
  decide (S ∈ pages P D G) && decide (T ∈ pages P D G) && decide (S ≠ T) &&
    decide (S.2 ≠ P.key config.favoritesPage) &&
    (Consolidated13.blocks P G).any (fun b =>
      decide (P.pageOfBlock (activeGraph P G) b = some S) && referencesB P D G b T)

def near (D : Dates K) (config : Decided.Config) (G : Graph) : NearDir → Nat → List (Page K) → List (Page K)
  | .out, n, X => walk (pages P D G) (links P D config G) n X
  | .inn, n, X => walk (pages P D G) (fun S T => links P D config G T S) n X
  | .both, n, X => walk (pages P D G) (links P D config G) n X ++
      walk (pages P D G) (fun S T => links P D config G T S) n X

def treeParent (D : Dates K) (spell : K → String) (G : Graph) (t : Page K) : Option (Page K) :=
  if t ∈ pages P D G then
    (Ref10.nsParent P.toParams (spell t.2)).bind (fun s =>
      (pages P D G).find? (fun p => decide (p = target P D G s)))
  else none

def children (D : Dates K) (spell : K → String) (G : Graph) (t : Page K) : List (Page K) :=
  (pages P D G).eraseDups.filter (fun c => decide (treeParent P D spell G c = some t))

def tree (D : Dates K) (spell : K → String) (G : Graph) : List (Page K × List (Page K)) :=
  (pages P D G).eraseDups.map (fun t => (t, children P D spell G t))

def namespaceTree (D : Dates K) (spell : K → String) (G : Graph) : List (Page K × List (Page K)) :=
  (tree P D spell G).filter (fun row => (treeParent P D spell G row.1).isSome || !row.2.isEmpty)

def groupFiles (D : Dates K) (G : Graph) (s : String) : Graph :=
  (activeGraph P G).filter (fun f => sameCls (edges P D G) (target P D G s) (P.claim f))

def candidates (D : Dates K) (G : Graph) (old new : String) : List Candidate :=
  (literal P G old new :: ((groupFiles P D G new).filter (fun f =>
    decide ((P.claim f).2 ≠ P.key old))).map (fun f => literal P G old (P.name f))).eraseDups

def offers (D : Dates K) (SP : StoredParser L) (E : Engine) (G : Graph)
    (extras : List String) (old new : String) : List Candidate :=
  (candidates P D G old new).filter (safe P SP E G extras old)

def answer (D : Dates K) (SP : StoredParser L) (E : Engine) (G : Graph)
    (extras : List String) (old new : String) (choice : Option Candidate) : Option Graph :=
  match choice with
  | none => some G
  | some c => if c ∈ offers P D SP E G extras old new then some (execute P E G old c) else none

def automatic (D : Dates K) (SP : StoredParser L) (E : Engine) (G : Graph)
    (extras : List String) (old new : String) : Option Graph :=
  match offers P D SP E G extras old new with
  | [c] => some (execute P E G old c)
  | _ => none

/-- The preferred literal option and all physical checks retain their existing
definitions. Only semantic group enumeration and preview need the date layer. -/
abbrev defaultCandidate := @Consolidated13.defaultCandidate

def linkedAt (D : Dates K) (favorites : Option String) (G : Graph) (s : String) : List BlockId :=
  linked P D favorites G (target P D G s)

structure RefSignature (K : Type) where
  spellings : List String
  targets : List (Page K)
  openings : List (Page K)
  edges : List (Page K × Page K)
  deriving DecidableEq

def signature (D : Dates K) (G : Graph) (b : BlockId) : RefSignature K :=
  ⟨P.refNames (activeGraph P G) b, refTargets P D G b,
    (P.refNames (activeGraph P G) b).map (resolve P D G), edges P D G⟩

def preview (D : Dates K) (E : Engine) (G : Graph) (old : String) (c : Candidate) : List (BlockId × BlockId) :=
  (Consolidated13.blocks P G).flatMap (fun b => ((images P E G old c b).filter (fun b' =>
    decide (signature P D G b ≠ signature P D (execute P E G old c) b'))).map (b, ·))

theorem target_exact (D : Dates K) (G : Graph) (s : String) :
    target P D G s = match openingFile P G (P.key s) with
      | some f => P.claim f
      | none => match D.canonical (P.key s) with
        | some k => (.journal, k)
        | none => (.page, P.key s) := rfl

theorem literal_claim (f : File) : P.claim f = (kindOf f, P.key (P.name f)) := rfl

theorem conflict_exact (G : Graph) (k : K) : P.conflictedB G k = true ↔
    ∃ f ∈ G, ∃ g ∈ G, f ≠ g ∧ P.key (P.name f) = k ∧ P.key (P.name g) = k :=
  P.conflictedB_iff

theorem opening_member {G : Graph} {k : K} {f : File}
    (h : openingFile P G k = some f) : f ∈ activeGraph P G ∧ (P.claim f).2 = k := by
  have hm := List.mem_filter.mp (minBy_mem h)
  have hk : (P.claim f).2 = k := of_decide_eq_true hm.2
  exact ⟨(active_iff P).mpr ⟨hm.1, hk ▸ h⟩, hk⟩

theorem target_claimed (D : Dates K) {G : Graph} {s : String} {f : File}
    (h : openingFile P G (P.key s) = some f) : target P D G s = P.claim f := by simp [target, h]

theorem open_real (D : Dates K) {G : Graph} {s : String}
    (h : (realFiles P G (target P D G s)).isEmpty = false) :
    resolve P D G s = target P D G s := by simp [resolve, h]

theorem literal_opening (D : Dates K) {G : Graph} {s : String} {f : File}
    (h : openingFile P G (P.key s) = some f) : resolve P D G s = P.claim f := by
  have ht := target_claimed P D h
  have hm : f ∈ realFiles P G (target P D G s) := by
    simp [realFiles, ht, (opening_member P h).1]
  have hn : (realFiles P G (target P D G s)).isEmpty = false := by
    cases he : realFiles P G (target P D G s) with
    | nil => simp [he] at hm
    | cons a as => rfl
  rw [open_real P D hn, ht]

theorem active_self_opening (D : Dates K) {G : Graph} {f : File} (hf : f ∈ activeGraph P G) :
    resolve P D G (P.name f) = P.claim f :=
  literal_opening P D ((active_iff P).mp hf).2

theorem distinct_literal_coexist (f g : File) (h : P.key (P.name f) ≠ P.key (P.name g)) (k : K) :
    P.conflictedB [f,g] k = false := by
  cases hc : P.conflictedB [f,g] k with
  | false => rfl
  | true =>
    obtain ⟨a, ha, b, hb, hn, hak, hbk⟩ := P.conflictedB_iff.mp hc
    simp only [List.mem_cons, List.not_mem_nil, or_false] at ha hb
    rcases ha with rfl | rfl <;> rcases hb with rfl | rfl
    · exact False.elim (hn rfl)
    · exact False.elim (h (hak.trans hbk.symm))
    · exact False.elim (h (hbk.trans hak.symm))
    · exact False.elim (hn rfl)

theorem target_date (D : Dates K) {G : Graph} {s : String} {k : K}
    (h : openingFile P G (P.key s) = none) (hd : D.canonical (P.key s) = some k) :
    target P D G s = (.journal, k) := by simp [target, h, hd]

theorem date_not_ordinary (D : Dates K) {G : Graph} {s : String} {k q : K}
    (h : openingFile P G (P.key s) = none) (hd : D.canonical (P.key s) = some k) :
    target P D G s ≠ (.page, q) := by rw [target_date P D h hd]; simp

theorem same_literal_target (D : Dates K) (G : Graph) {s r : String} (h : P.key s = P.key r) :
    target P D G s = target P D G r := by simp [target, h]

theorem same_day_target (D : Dates K) {G : Graph} {s r : String} {k : K}
    (hs : openingFile P G (P.key s) = none) (hr : openingFile P G (P.key r) = none)
    (ds : D.canonical (P.key s) = some k) (dr : D.canonical (P.key r) = some k) :
    target P D G s = target P D G r := by rw [target_date P D hs ds, target_date P D hr dr]

theorem crosskind_conflict {G : Graph} {f g : File} (hf : f ∈ G) (hg : g ∈ G)
    (fd : f.dir = .pages) (gd : g.dir = .journals) (hk : P.key (P.name f) = P.key (P.name g)) :
    P.conflictedB G (P.key (P.name f)) = true := by
  apply P.conflictedB_iff.mpr
  refine ⟨f, hf, g, hg, ?_, rfl, hk.symm⟩
  intro he
  have hd := congrArg File.dir he
  rw [fd, gd] at hd
  contradiction

theorem date_name_query (D : Dates K) (Q : QParams K L) {G : Graph} {s : String} {k q : K}
    (h : openingFile P G (P.key s) = none) (hd : D.canonical (P.key s) = some k) :
    leafPage P D Q G (.name s) (.page, q) = false := by
  simp [leafPage, target_date P D h hd]

theorem reference_exact (D : Dates K) (G : Graph) (b : BlockId) (t : Page K) :
    referencesB P D G b t = true ↔ ∃ s ∈ P.refNames (activeGraph P G) b,
      Reach (edges P D G) (target P D G s) t := by
  simp [referencesB, refTargets, List.any_eq_true, sameCls_iff, cls]

theorem no_alias_reference (D : Dates K) {G : Graph} (h : edges P D G = []) (b : BlockId) (t : Page K) :
    referencesB P D G b t = true ↔ t ∈ refTargets P D G b := by
  simp only [referencesB, h, List.any_eq_true, sameCls_iff, cls, Reach.nil_iff]
  constructor
  · rintro ⟨v, hv, rfl⟩; exact hv
  · intro ht; exact ⟨t, ht, rfl⟩

theorem edge_exact (D : Dates K) (G : Graph) (a b : Page K) :
    (a,b) ∈ edges P D G ↔ ∃ f ∈ activeGraph P G,
      P.claim f = a ∧ ∃ s ∈ P.aliasDecl f.format f.text, target P D G s = b := by
  simp [edges, List.mem_flatMap, List.mem_map]
  constructor
  · rintro ⟨f, hf, s, hs, ha, hb⟩; exact ⟨f, hf, ha, s, hs, hb⟩
  · rintro ⟨f, hf, ha, s, hs, hb⟩; exact ⟨f, hf, s, hs, ha, hb⟩

theorem opening_reachable (D : Dates K) (G : Graph) (s : String) :
    Reach (edges P D G) (target P D G s) (resolve P D G s) := by
  unfold resolve
  dsimp only
  split
  · exact Reach.refl _
  · cases hm : minBy (Owner5.rankLe (fun f => Owner5.bytes f.path) Orders.fileLe)
        (declarers P D G (target P D G s)) with
    | none => exact Reach.refl _
    | some f =>
      obtain ⟨hf, hd⟩ := List.mem_filter.mp (minBy_mem hm)
      obtain ⟨r, hr, ht⟩ := List.any_eq_true.mp hd
      have he : (P.claim f, target P D G s) ∈ edges P D G :=
        (edge_exact P D G _ _).mpr ⟨f, hf, rfl, r, hr, of_decide_eq_true ht⟩
      exact Reach.symm (Reach.edge he)

theorem mention_references_opened (D : Dates K) {G : Graph} {b : BlockId} {s : String}
    (h : s ∈ P.refNames (activeGraph P G) b) :
    referencesB P D G b (resolve P D G s) = true :=
  (reference_exact P D G b _).mpr ⟨s, h, opening_reachable P D G s⟩

theorem open_virtual (D : Dates K) {G : Graph} {s : String}
    (hr : realFiles P G (target P D G s) = [])
    (ha : declarers P D G (target P D G s) = []) : resolve P D G s = target P D G s := by
  simp [resolve, hr, ha, minBy]

theorem panel_real (D : Dates K) (G : Graph) (t : Page K)
    (h : (realFiles P G t).isEmpty = false) : panelSelf P D G t = t := by simp [panelSelf, h]

theorem linked_exact (D : Dates K) (favorites : Option String) (G : Graph) (t : Page K) (b : BlockId) :
    b ∈ linked P D favorites G t ↔ b ∈ Consolidated13.blocks P G ∧
      referencesB P D G b t = true ∧ panelSource P D favorites G t b = true := by simp [linked]

theorem count_panel (D : Dates K) (favorites : Option String) (G : Graph) (t : Page K) :
    referenceCount P D favorites G t = (linked P D favorites G t).length := rfl

theorem panelSource_exact (D : Dates K) (favorites : Option String) (G : Graph) (t : Page K) (b : BlockId) :
    panelSource P D favorites G t b = true ↔ ∃ f,
      fileAt (activeGraph P G) b.1 = some f ∧ P.claim f ≠ panelSelf P D G t ∧
      ∀ s ∈ favorites, P.key s ≠ P.key "" → (P.claim f).2 ≠ P.key s := by
  cases hf : fileAt (activeGraph P G) b.1 with
  | none => simp [panelSource, hf]
  | some f =>
    cases favorites with
    | none => simp [panelSource, hf]
    | some s => by_cases he : P.key s = P.key "" <;> simp [panelSource, hf, he]

theorem pages_exact (D : Dates K) (G : Graph) (t : Page K) :
    t ∈ pages P D G ↔ (∃ f ∈ activeGraph P G, P.claim f = t) ∨
      (t ∈ seedTargets P D G ∧ (realFiles P G t).isEmpty = true ∧
        ∀ e ∈ edges P D G, e.2 ≠ t) := by simp [pages, Bool.and_eq_true]

theorem children_exact (D : Dates K) (spell : K → String) (G : Graph) (t c : Page K) :
    c ∈ children P D spell G t ↔ c ∈ pages P D G ∧ treeParent P D spell G c = some t := by simp [children]

theorem den_exact (D : Dates K) (Q : QParams K L) (G : Graph) (a : Anchor) (q : Query L) (n : Node K) :
    n ∈ den P D Q G a q ↔ n ∈ univ P D G a ∧ q.bval (fun l => leafAt P D Q G l n) = true := by simp [den]

theorem queryResults_exact (D : Dates K) (Q : QParams K L) (R : QueryRowParser) (o : QueryPresentation)
    (G : Graph) (a : Anchor) (q : Query L) (n : Node K) :
    n ∈ queryResults P D Q R o G a q ↔ n ∈ den P D Q G a q ∧
      queryRowVisible P Q R o G (den P D Q G a q) n = true := by simp [queryResults, queryProjection]

theorem links_exact (D : Dates K) (config : Decided.Config) (G : Graph) (S T : Page K) :
    links P D config G S T = true ↔ S ∈ pages P D G ∧ T ∈ pages P D G ∧ S ≠ T ∧
      S.2 ≠ P.key config.favoritesPage ∧ ∃ b ∈ Consolidated13.blocks P G,
        P.pageOfBlock (activeGraph P G) b = some S ∧ referencesB P D G b T = true := by
  simp [links, Bool.and_eq_true, List.any_eq_true, and_assoc]

theorem after_reference_exact (D : Dates K) (E : Engine) (G : Graph) (old : String) (c : Candidate)
    (b : BlockId) (t : Page K) : referencesB P D (execute P E G old c) b t = true ↔
      ∃ s ∈ P.refNames (activeGraph P (assembled P E G old c)) b,
        Reach (edges P D (assembled P E G old c)) (target P D (assembled P E G old c) s) t := by
  rw [assembled_eq]; exact reference_exact P D _ _ _

theorem after_count_exact (D : Dates K) (favorites : Option String) (E : Engine) (G : Graph)
    (old : String) (c : Candidate) (t : Page K) :
    referenceCount P D favorites (execute P E G old c) t =
      ((linked P D favorites (execute P E G old c) t).filter (fun b => (linked P D favorites G t).contains b)).length +
      ((linked P D favorites (execute P E G old c) t).filter (fun b => !(linked P D favorites G t).contains b)).length :=
  length_partition _ _

theorem after_queryResults_exact (D : Dates K) (Q : QParams K L) (R : QueryRowParser) (o : QueryPresentation)
    (E : Engine) (G : Graph) (old : String) (c : Candidate) (a : Anchor) (q : Query L) (n : Node K) :
    n ∈ queryResults P D Q R o (execute P E G old c) a q ↔
      (n ∈ univ P D (assembled P E G old c) a ∧
        q.bval (fun l => leafAt P D Q (assembled P E G old c) l n) = true) ∧
      queryRowVisible P Q R o (assembled P E G old c) (den P D Q (assembled P E G old c) a q) n = true := by
  rw [queryResults_exact, den_exact, assembled_eq]

theorem offered_safe (D : Dates K) (SP : StoredParser L) (E : Engine)
    {G : Graph} {extras : List String} {old new : String} {c : Candidate}
    (h : c ∈ offers P D SP E G extras old new) : safe P SP E G extras old c = true :=
  (List.mem_filter.mp h).2

theorem offers_exact (D : Dates K) (SP : StoredParser L) (E : Engine)
    (G : Graph) (extras : List String) (old new : String) (c : Candidate) :
    c ∈ offers P D SP E G extras old new ↔ c ∈ candidates P D G old new ∧ safe P SP E G extras old c = true := by simp [offers]

theorem offered_executes (D : Dates K) (SP : StoredParser L) (E : Engine)
    {G : Graph} {extras : List String} {old new : String} {c : Candidate}
    (h : c ∈ offers P D SP E G extras old new) :
    answer P D SP E G extras old new (some c) = some (execute P E G old c) := by simp [answer, h]

theorem default_offered (D : Dates K) (SP : StoredParser L) (E : Engine)
    {G : Graph} {extras : List String} {old new : String} {c : Candidate}
    (h : defaultCandidate P SP E G extras old new = some c) :
    c ∈ offers P D SP E G extras old new := by
  unfold defaultCandidate Consolidated13.defaultCandidate at h
  dsimp only at h
  split at h
  · next hs =>
    have hc := (Option.some.inj h).symm
    subst c
    apply List.mem_filter.mpr
    exact ⟨by simp [candidates], hs⟩
  · contradiction

theorem cancel_identity (D : Dates K) (SP : StoredParser L) (E : Engine)
    (G : Graph) (extras : List String) (old new : String) :
    answer P D SP E G extras old new none = some G := rfl

theorem offered_recomputed (D : Dates K) (SP : StoredParser L) (Q : QParams K L)
    (R : QueryRowParser) (o : QueryPresentation) (favorites : Option String) (config : Decided.Config)
    (E : Engine) {G : Graph} {extras : List String} {old new : String} {c : Candidate}
    (h : c ∈ offers P D SP E G extras old new) :
    ∃ H, answer P D SP E G extras old new (some c) = some H ∧ H = assembled P E G old c ∧
      edges P D H = edges P D (assembled P E G old c) ∧
      referencesB P D H = referencesB P D (assembled P E G old c) ∧
      linked P D favorites H = linked P D favorites (assembled P E G old c) ∧
      referenceCount P D favorites H = referenceCount P D favorites (assembled P E G old c) ∧
      den P D Q H = den P D Q (assembled P E G old c) ∧
      queryResults P D Q R o H = queryResults P D Q R o (assembled P E G old c) ∧
      links P D config H = links P D config (assembled P E G old c) ∧
      resolve P D H = resolve P D (assembled P E G old c) := by
  refine ⟨execute P E G old c, offered_executes P D SP E h, ?_⟩
  rw [assembled_eq]
  exact ⟨rfl, rfl, rfl, rfl, rfl, rfl, rfl, rfl, rfl⟩

theorem automatic_unique (D : Dates K) (SP : StoredParser L) (E : Engine)
    {G H : Graph} {extras : List String} {old new : String}
    (h : automatic P D SP E G extras old new = some H) :
    ∃ c, offers P D SP E G extras old new = [c] ∧ H = execute P E G old c := by
  unfold automatic at h
  split at h
  · next c hc => exact ⟨c, hc, (Option.some.inj h).symm⟩
  · contradiction

theorem offered_text_safety (D : Dates K) (SP : StoredParser L)
    {Q : QParams K L} {E : Engine} {G : Graph} {old new : String} {c : Candidate} {extras : List String}
    {input : File → List Piece} (_h : c ∈ offers P D SP E G extras old new)
    (F : Realizes P SP Q E G old c input) {f : File} (hf : f ∈ G) {p : Piece}
    (hp : p ∈ input f) (he : p.editable = false) :
    ∃ f' ∈ execute P E G old c, Placement.original p ∈ F.trace.output f' ∧
      Placement.render (subst P old c) (.original p) = p.text ∧
      String.join ((F.trace.output f').map (Placement.render (subst P old c))) = f'.text :=
  trace_protected_bytes F.text hf (by rw [F.fixedInput]; exact hp) he

theorem offered_twins (D : Dates K) (SP : StoredParser L) {Q : QParams K L} {E : Engine} {G : Graph}
    {extras : List String} {old new : String} {c : Candidate} {input : File → List Piece}
    (h : c ∈ offers P D SP E G extras old new) (F : Realizes P SP Q E G old c input)
    {k : K} (hc : P.conflictedB G k = true) :
    (openingFile P (execute P E G old c) k).map File.path = (openingFile P G k).map File.path :=
  stationary_twins_opening P SP (offered_safe P D SP E h) F hc

theorem signature_reference (D : Dates K) {G H : Graph} {b b' : BlockId}
    (h : signature P D G b = signature P D H b') (t : Page K) :
    referencesB P D G b t = referencesB P D H b' t := by
  have repr : ∀ A x, referencesB P D A x t =
    (signature P D A x).targets.any (fun v => sameCls (signature P D A x).edges v t) := by intro A x; rfl
  rw [repr G b, repr H b', h]

theorem preview_complete (D : Dates K) (E : Engine) {G : Graph} {old : String} {c : Candidate}
    {b b' : BlockId} (hb : b ∈ Consolidated13.blocks P G) (hi : b' ∈ images P E G old c b)
    (t : Page K) (h : referencesB P D G b t ≠ referencesB P D (execute P E G old c) b' t) :
    (b,b') ∈ preview P D E G old c := by
  have hn : signature P D G b ≠ signature P D (execute P E G old c) b' :=
    fun he => h (signature_reference P D he t)
  simp only [preview, List.mem_flatMap, List.mem_map, List.mem_filter, decide_eq_true_eq]
  exact ⟨b, hb, b', ⟨hi, hn⟩, rfl⟩

/-- Additional date diagnostics are individual path witnesses and primitive
query inputs, never inequality of complete operation outputs. -/
theorem generic_reference_exact (G : Graph) (b : BlockId) (t : Page K) :
    Consolidated13.referencesB P G b t = true ↔
      ∃ s ∈ P.refNames (activeGraph P G) b,
        Reach ((readParams P G).aliasEdges (activeGraph P G)) (P.key s) t.2 := by
  change (P.refNames (activeGraph P G) b).any (fun s =>
    sameCls ((readParams P G).aliasEdges (activeGraph P G)) (P.key s) t.2) = true ↔ _
  simp [List.any_eq_true, sameCls_iff, cls]

theorem reference_comparison_exhaustive (D : Dates K) (G : Graph) (b : BlockId) (t : Page K) :
    ((∃ s ∈ P.refNames (activeGraph P G) b, Reach (edges P D G) (target P D G s) t) ∧
      ∀ s ∈ P.refNames (activeGraph P G) b,
        ¬ Reach ((readParams P G).aliasEdges (activeGraph P G)) (P.key s) t.2) ∨
    ((∃ s ∈ P.refNames (activeGraph P G) b,
        Reach ((readParams P G).aliasEdges (activeGraph P G)) (P.key s) t.2) ∧
      ∀ s ∈ P.refNames (activeGraph P G) b, ¬ Reach (edges P D G) (target P D G s) t) ∨
    referencesB P D G b t = Consolidated13.referencesB P G b t := by
  have hn := reference_exact P D G b t
  have ho := generic_reference_exact P G b t
  cases a : referencesB P D G b t <;> cases z : Consolidated13.referencesB P G b t
  · exact Or.inr (Or.inr rfl)
  · right; left
    refine ⟨ho.mp z, ?_⟩
    intro s hs hr
    have he := hn.mpr ⟨s, hs, hr⟩
    rw [a] at he
    contradiction
  · left
    refine ⟨hn.mp a, ?_⟩
    intro s hs hr
    have he := ho.mpr ⟨s, hs, hr⟩
    rw [z] at he
    contradiction
  · exact Or.inr (Or.inr rfl)

theorem bval_congr_leaves {L : Type} (q : Query L) (a b : Leaf L → Bool)
    (h : ∀ l ∈ q.leaves, a l = b l) : q.bval a = q.bval b := by
  induction q with
  | leaf l => exact h l (by simp [Query.leaves])
  | and q r ihq ihr =>
    simp only [Query.bval, ihq (fun l hl => h l (List.mem_append_left _ hl)),
      ihr (fun l hl => h l (List.mem_append_right _ hl))]
  | or q r ihq ihr =>
    simp only [Query.bval, ihq (fun l hl => h l (List.mem_append_left _ hl)),
      ihr (fun l hl => h l (List.mem_append_right _ hl))]
  | not q ih => exact congrArg Bool.not (ih h)

theorem query_comparison_exhaustive (D : Dates K) (Q : QParams K L) (G : Graph)
    (a : Anchor) (q : Query L) (n : Node K) :
    (¬ (n ∈ univ P D G a ↔ n ∈ (readParams P G).univ true (activeGraph P G) a)) ∨
    (∃ l ∈ q.leaves, leafAt P D Q G l n ≠ (readParams P G).leafAt3 Q (activeGraph P G) l n) ∨
    (n ∈ den P D Q G a q ↔ n ∈ Consolidated13.den P Q G a q) := by
  classical
  by_cases hu : n ∈ univ P D G a ↔ n ∈ (readParams P G).univ true (activeGraph P G) a
  · right
    by_cases hl : ∃ l ∈ q.leaves, leafAt P D Q G l n ≠ (readParams P G).leafAt3 Q (activeGraph P G) l n
    · exact Or.inl hl
    right
    have he := bval_congr_leaves q (fun l => leafAt P D Q G l n)
      (fun l => (readParams P G).leafAt3 Q (activeGraph P G) l n)
      (fun l hm => Classical.byContradiction fun hn => hl ⟨l, hm, hn⟩)
    rw [den_exact, he, hu]
    exact (Params.mem_denU3 (readParams P G) Q).symm
  · exact Or.inl hu

theorem panel_comparison_exhaustive (D : Dates K) (favorites : Option String) (G : Graph) (t : Page K) :
    (∃ b ∈ Consolidated13.blocks P G, referencesB P D G b t ≠ Consolidated13.referencesB P G b t) ∨
    (∃ b ∈ Consolidated13.blocks P G, panelSource P D favorites G t b ≠ Consolidated13.panelSource P favorites G t b) ∨
    (linked P D favorites G t = Consolidated13.linked P favorites G t ∧
      referenceCount P D favorites G t = Consolidated13.referenceCount P favorites G t) := by
  classical
  by_cases hr : ∃ b ∈ Consolidated13.blocks P G, referencesB P D G b t ≠ Consolidated13.referencesB P G b t
  · exact Or.inl hr
  right
  by_cases hs : ∃ b ∈ Consolidated13.blocks P G, panelSource P D favorites G t b ≠ Consolidated13.panelSource P favorites G t b
  · exact Or.inl hs
  right
  have he : linked P D favorites G t = Consolidated13.linked P favorites G t := by
    unfold linked Consolidated13.linked Consolidated13.relationHits
    rw [List.filter_filter]
    apply List.filter_congr
    intro b hb
    have hb' : b ∈ Consolidated13.blocks P G := List.mem_eraseDups.mp hb
    have h1 : referencesB P D G b t = Consolidated13.referencesB P G b t :=
      Classical.byContradiction fun hn => hr ⟨b, hb', hn⟩
    have h2 : panelSource P D favorites G t b = Consolidated13.panelSource P favorites G t b :=
      Classical.byContradiction fun hn => hs ⟨b, hb', hn⟩
    simp only [h1, h2, Bool.and_comm]
  exact ⟨he, congrArg List.length he⟩

end Date13
end CM
