/- Universal structural comparison via primitive reader inputs. Baselines are
comparison data; no obsolete date policy is selectable by the current API. -/

import DateOperationComparison14

set_option linter.unusedSectionVars false
namespace CM
namespace Date13
open Params Consolidated13 QueryRename6
variable {K L : Type} [DecidableEq K] (P : Inputs K)

/-- Every field is a primitive inventory row, name/path/leaf answer or finite
selection. No complete reference/query/tree/panel result is a field. A baseline
may supply the accepted r13 inputs or any other reader's inputs. -/
structure ReaderInputs (K L : Type) where
  realRows : List (Page K)
  blocks : List BlockId
  seeds : List String
  virtualRow : String → Option (Page K)
  refNames : BlockId → List String
  pathAnswer : String → Page K → Bool
  leaf : Leaf L → Node K → Bool
  sourceEligible : Page K → BlockId → Bool
  blockSource : BlockId → Option (Page K)
  linkSource : Page K → Bool
  parentSpelling : Page K → Option String
  parentLookup : String → Option (Page K)
  header : BlockId → Bool
  parentBlock : BlockId → Option BlockId
  initial : String → Page K
  direct : String → Option (Page K)

def ReaderInputs.pages (r : ReaderInputs K L) := r.realRows ++ r.seeds.filterMap r.virtualRow
def ReaderInputs.references (r : ReaderInputs K L) (b : BlockId) (t : Page K) :=
  (r.refNames b).any (fun s => r.pathAnswer s t)
def ReaderInputs.univ (r : ReaderInputs K L) : Anchor → List (Node K)
  | .page => r.pages.map Node.page | .block => r.blocks.map Node.block
def ReaderInputs.den (r : ReaderInputs K L) (a : Anchor) (q : Query L) :=
  (r.univ a).filter (fun n => q.bval (fun l => r.leaf l n))
def ReaderInputs.linked (r : ReaderInputs K L) (t : Page K) :=
  r.blocks.eraseDups.filter (fun b => r.references b t && r.sourceEligible t b)
def ReaderInputs.visible (r : ReaderInputs K L) (o : QueryPresentation) (raw : List (Node K)) : Node K → Bool
  | .page _ => true
  | .block b => (o.showHeaders || !r.header b) &&
      !(o.removeBlockChildren && (r.parentBlock b).any (fun a => raw.contains (.block a)))
def ReaderInputs.queryRows (r : ReaderInputs K L) (o : QueryPresentation) (a : Anchor) (q : Query L) :=
  let raw := r.den a q
  raw.filter (r.visible o raw)
def ReaderInputs.links (r : ReaderInputs K L) (S T : Page K) :=
  decide (S ∈ r.pages) && decide (T ∈ r.pages) && decide (S ≠ T) && r.linkSource S &&
    r.blocks.any (fun b => decide (r.blockSource b = some S) && r.references b T)
def ReaderInputs.near (r : ReaderInputs K L) : NearDir → Nat → List (Page K) → List (Page K)
  | .out,n,X => walk r.pages r.links n X
  | .inn,n,X => walk r.pages (fun S T => r.links T S) n X
  | .both,n,X => walk r.pages r.links n X ++ walk r.pages (fun S T => r.links T S) n X
def ReaderInputs.treeParent (r : ReaderInputs K L) (t : Page K) :=
  if t ∈ r.pages then (r.parentSpelling t).bind r.parentLookup else none
def ReaderInputs.children (r : ReaderInputs K L) (t : Page K) :=
  r.pages.eraseDups.filter (fun c => decide (r.treeParent c = some t))
def ReaderInputs.tree (r : ReaderInputs K L) := r.pages.eraseDups.map (fun t => (t,r.children t))
def ReaderInputs.namespaceTree (r : ReaderInputs K L) :=
  r.tree.filter (fun row => (r.treeParent row.1).isSome || !row.2.isEmpty)
def ReaderInputs.opening (r : ReaderInputs K L) (s : String) := (r.direct s).getD (r.initial s)

/-- Indexed row differences keep order and multiplicity in the comparison. -/
def rowDifference {α : Type} (a b : List α) : Prop := ∃ i : Nat, a[i]? ≠ b[i]?
def functionDifference {α β : Type} (a b : α → β) : Prop := ∃ x, a x ≠ b x

def primitiveDifference (a b : ReaderInputs K L) : Prop :=
  rowDifference a.realRows b.realRows ∨ rowDifference a.blocks b.blocks ∨ rowDifference a.seeds b.seeds ∨
  functionDifference a.virtualRow b.virtualRow ∨ functionDifference a.refNames b.refNames ∨
  (∃ s t, a.pathAnswer s t ≠ b.pathAnswer s t) ∨
  (∃ l n, a.leaf l n ≠ b.leaf l n) ∨
  (∃ t x, a.sourceEligible t x ≠ b.sourceEligible t x) ∨
  functionDifference a.blockSource b.blockSource ∨ functionDifference a.linkSource b.linkSource ∨
  functionDifference a.parentSpelling b.parentSpelling ∨ functionDifference a.parentLookup b.parentLookup ∨
  functionDifference a.header b.header ∨ functionDifference a.parentBlock b.parentBlock ∨
  functionDifference a.initial b.initial ∨ functionDifference a.direct b.direct

theorem rows_eq_of_no_difference {α : Type} (a b : List α) (h : ¬ rowDifference a b) : a = b := by
  apply List.ext_getElem?
  intro i
  exact Classical.byContradiction (fun he => h ⟨i,he⟩)

theorem functions_eq_of_no_difference {α β : Type} (a b : α → β) (h : ¬ functionDifference a b) : a = b := by
  funext x
  exact Classical.byContradiction (fun he => h ⟨x,he⟩)

theorem inputs_eq_of_no_difference (a b : ReaderInputs K L) (h : ¬ primitiveDifference a b) : a = b := by
  classical
  simp only [primitiveDifference,not_or] at h
  have h1 := rows_eq_of_no_difference a.realRows b.realRows h.1
  have h2 := rows_eq_of_no_difference a.blocks b.blocks h.2.1
  have h3 := rows_eq_of_no_difference a.seeds b.seeds h.2.2.1
  have h4 := functions_eq_of_no_difference a.virtualRow b.virtualRow h.2.2.2.1
  have h5 := functions_eq_of_no_difference a.refNames b.refNames h.2.2.2.2.1
  have h6 : a.pathAnswer = b.pathAnswer := by
    funext s t; exact Classical.byContradiction (fun he => h.2.2.2.2.2.1 ⟨s,t,he⟩)
  have h7 : a.leaf = b.leaf := by
    funext l n; exact Classical.byContradiction (fun he => h.2.2.2.2.2.2.1 ⟨l,n,he⟩)
  have h8 : a.sourceEligible = b.sourceEligible := by
    funext t x; exact Classical.byContradiction (fun he => h.2.2.2.2.2.2.2.1 ⟨t,x,he⟩)
  have h9 := functions_eq_of_no_difference a.blockSource b.blockSource h.2.2.2.2.2.2.2.2.1
  have h10 := functions_eq_of_no_difference a.linkSource b.linkSource h.2.2.2.2.2.2.2.2.2.1
  have h11 := functions_eq_of_no_difference a.parentSpelling b.parentSpelling h.2.2.2.2.2.2.2.2.2.2.1
  have h12 := functions_eq_of_no_difference a.parentLookup b.parentLookup h.2.2.2.2.2.2.2.2.2.2.2.1
  have h13 := functions_eq_of_no_difference a.header b.header h.2.2.2.2.2.2.2.2.2.2.2.2.1
  have h14 := functions_eq_of_no_difference a.parentBlock b.parentBlock h.2.2.2.2.2.2.2.2.2.2.2.2.2.1
  have h15 := functions_eq_of_no_difference a.initial b.initial h.2.2.2.2.2.2.2.2.2.2.2.2.2.2.1
  have h16 := functions_eq_of_no_difference a.direct b.direct h.2.2.2.2.2.2.2.2.2.2.2.2.2.2.2
  cases a; cases b
  simp_all

/-- All ordered output surfaces, including arbitrary Boolean queries, every
neighborhood radius/direction, full and namespace trees, and navigation. -/
structure ReaderAgreement (a b : ReaderInputs K L) : Prop where
  pages : a.pages = b.pages
  blocks : a.blocks = b.blocks
  references : a.references = b.references
  den : a.den = b.den
  linked : a.linked = b.linked
  counts : ∀ t, (a.linked t).length = (b.linked t).length
  queryRows : a.queryRows = b.queryRows
  links : a.links = b.links
  near : a.near = b.near
  tree : a.tree = b.tree
  namespaceTree : a.namespaceTree = b.namespaceTree
  opening : a.opening = b.opening

theorem structural_comparison_exhaustive (a b : ReaderInputs K L) :
    primitiveDifference a b ∨ ReaderAgreement a b := by
  classical
  by_cases h : primitiveDifference a b
  · exact Or.inl h
  · right
    have he := inputs_eq_of_no_difference a b h
    subst b
    exact ⟨rfl,rfl,rfl,rfl,rfl,fun _ => rfl,rfl,rfl,rfl,rfl,rfl,rfl⟩

def rawSeeds (D : Dates K) (G : Graph) : List String :=
  (blocks P D G).flatMap (P.refNames (activeGraph P D G)) ++ (activeGraph P D G).flatMap P.fileNsNames

def seedRow (D : Dates K) (G : Graph) (s : String) : Option (Page K) :=
  let t := target P D G s
  if (realFiles P D G t).isEmpty && !(edges P D G).any (fun e => decide (e.2 = t)) then some t else none

def directTarget (D : Dates K) (G : Graph) (s : String) : Option (Page K) :=
  if !(realFiles P D G (target P D G s)).isEmpty then none else
    (minBy pathLe (declarers P D G (target P D G s))).map (claim P D)

def readerInputs (D : Dates K) (Q : QParams K L) (R : QueryRowParser) (config : Decided.Config)
    (favorites : Option String) (spell : K → String) (G : Graph) : ReaderInputs K L where
  realRows := (activeGraph P D G).map (claim P D)
  blocks := blocks P D G
  seeds := rawSeeds P D G
  virtualRow := seedRow P D G
  refNames := P.refNames (activeGraph P D G)
  pathAnswer := fun s t => sameCls (edges P D G) (target P D G s) t
  leaf := leafAt P D Q G
  sourceEligible := panelSource P D favorites G
  blockSource := pageOfBlock P D G
  linkSource := fun S => decide (S ≠ target P D G config.favoritesPage)
  parentSpelling := fun t => Ref10.nsParent P.toParams (spell t.2)
  parentLookup := fun s => (pages P D G).find? (fun p => decide (p = target P D G s))
  header := fun b => decide (rowKind P D R G b = .header)
  parentBlock := queryParent P D Q R G
  initial := target P D G
  direct := directTarget P D G

theorem pages_seed_rows (D : Dates K) (G : Graph) :
    pages P D G = (activeGraph P D G).map (claim P D) ++ (rawSeeds P D G).filterMap (seedRow P D G) := by
  have hs : seedTargets P D G = (rawSeeds P D G).map (target P D G) := by
    simp only [seedTargets,rawSeeds,List.map_append,List.map_flatMap]; rfl
  rw [pages,hs]
  congr 1
  generalize rawSeeds P D G = xs
  induction xs with
  | nil => rfl
  | cons a xs ih =>
    simp only [List.map_cons,List.filter_cons,List.filterMap_cons,seedRow]
    split <;> simp_all

theorem reader_inputs_realize (D : Dates K) (Q : QParams K L) (R : QueryRowParser)
    (config : Decided.Config) (favorites : Option String) (spell : K → String) (G : Graph) :
    let r := readerInputs P D Q R config favorites spell G
    r.pages = pages P D G ∧ r.references = referencesB P D G ∧
    r.den = den P D Q G ∧ r.linked = linked P D favorites G ∧
    r.queryRows = (fun o a q => queryResults P D Q R o G a q) ∧
    r.links = links P D config G ∧ r.near = near P D config G ∧
    r.tree = tree P D spell G ∧ r.namespaceTree = namespaceTree P D spell G ∧
    r.opening = resolve P D G := by
  let r := readerInputs P D Q R config favorites spell G
  have hp : r.pages = pages P D G := (pages_seed_rows P D G).symm
  have hr : r.references = referencesB P D G := by
    funext b t
    simp only [r,ReaderInputs.references,readerInputs,referencesB,refTargets,List.any_map]
    rfl
  have hu : r.univ = univ P D G := by
    funext a; cases a <;> simp only [ReaderInputs.univ,univ,hp]; rfl
  have hd : r.den = den P D Q G := by
    funext a q; simp only [ReaderInputs.den,den,hu]; rfl
  have hlinked : r.linked = linked P D favorites G := by
    funext t; simp only [ReaderInputs.linked,linked,hr]; rfl
  have hv : ∀ o raw, r.visible o raw = queryRowVisible P D Q R o G raw := by
    intro o raw; funext n
    cases n with
    | page t => rfl
    | block b =>
      change ((o.showHeaders || !decide (rowKind P D R G b = .header)) && _) = _
      cases he : rowKind P D R G b <;> simp [queryRowVisible,he,ReaderInputs.visible,r,readerInputs]
  have hq : r.queryRows = fun o a q => queryResults P D Q R o G a q := by
    funext o a q; simp only [ReaderInputs.queryRows,hd,hv,queryResults,queryProjection]
  have hl : r.links = links P D config G := by
    funext S T; simp only [ReaderInputs.links,links,hp,hr]; rfl
  have hn : r.near = near P D config G := by
    funext d n X; cases d <;> simp only [ReaderInputs.near,near,hp,hl]
  have htparent : r.treeParent = treeParent P D spell G := by
    funext t; simp only [ReaderInputs.treeParent,treeParent,hp]; rfl
  have hchildren : r.children = children P D spell G := by
    funext t; simp only [ReaderInputs.children,children,hp,htparent]
  have ht : r.tree = tree P D spell G := by simp only [ReaderInputs.tree,tree,hp,hchildren]
  have hns : r.namespaceTree = namespaceTree P D spell G := by
    simp only [ReaderInputs.namespaceTree,namespaceTree,ht,htparent]
  have ho : r.opening = resolve P D G := by
    funext s
    change (directTarget P D G s).getD (target P D G s) = _
    unfold directTarget resolve openTarget
    split <;> rfl
  exact ⟨hp,hr,hd,hlinked,hq,hl,hn,ht,hns,ho⟩

/-- The baseline argument is arbitrary. In particular no graph-validity,
unique-path, date-preservation, parser-equivalence or realization premise. -/
theorem static_comparison_exhaustive_final (D : Dates K) (Q : QParams K L) (R : QueryRowParser)
    (config : Decided.Config) (favorites : Option String) (spell : K → String) (G : Graph)
    (baseline : ReaderInputs K L) :
    primitiveDifference (readerInputs P D Q R config favorites spell G) baseline ∨
      ReaderAgreement (readerInputs P D Q R config favorites spell G) baseline :=
  structural_comparison_exhaustive _ _

/-- Comparing projected rows directly to raw readings additionally identifies
an excluded reference source or a header/immediate-parent-suppressed query row. -/
theorem presented_comparison_exhaustive_final (r : ReaderInputs K L) (t : Page K)
    (a : Anchor) (q : Query L) (o : QueryPresentation) :
    (∃ b ∈ r.blocks.eraseDups.filter (fun b => r.references b t),r.sourceEligible t b = false) ∨
    (∃ n ∈ r.den a q,r.visible o (r.den a q) n = false) ∨
    (r.linked t = r.blocks.eraseDups.filter (fun b => r.references b t) ∧
      (r.linked t).length = (r.blocks.eraseDups.filter (fun b => r.references b t)).length ∧
      r.queryRows o a q = r.den a q) := by
  classical
  by_cases hs : ∃ b ∈ r.blocks.eraseDups.filter (fun b => r.references b t),r.sourceEligible t b = false
  · exact Or.inl hs
  right
  by_cases hq : ∃ n ∈ r.den a q,r.visible o (r.den a q) n = false
  · exact Or.inl hq
  right
  have hp : r.linked t = r.blocks.eraseDups.filter (fun b => r.references b t) := by
    unfold ReaderInputs.linked
    have he : (fun b => r.references b t && r.sourceEligible t b) =
        (fun b => r.sourceEligible t b && r.references b t) := funext (fun _ => Bool.and_comm ..)
    rw [he,← List.filter_filter]
    apply List.filter_eq_self.mpr
    intro b hb
    cases he : r.sourceEligible t b with
    | true => rfl
    | false => exact False.elim (hs ⟨b,hb,he⟩)
  have hrows : r.queryRows o a q = r.den a q := by
    apply List.filter_eq_self.mpr
    intro n hn
    cases he : r.visible o (r.den a q) n with
    | true => rfl
    | false => exact False.elim (hq ⟨n,hn,he⟩)
  exact ⟨hp,congrArg List.length hp,hrows⟩

end Date13
end CM
