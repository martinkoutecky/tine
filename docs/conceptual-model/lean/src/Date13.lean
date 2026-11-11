/- Current reader: one day identity, independently of file location and label. -/
import PresentationWitness13

set_option linter.unusedSectionVars false
namespace CM
namespace Date13
open Params Consolidated13 QueryRename6
variable {K L : Type} [DecidableEq K] (P : Inputs K)

/-- Accepted literal keys map to calendar-day values, NOT rendered titles.
Journal filename extraction supplies a day independently of its displayed label.
These are parser boundaries; no unique-label or round-trip law is assumed. -/
structure Dates (K : Type) where
  parse : K → Option K
  journalDay : String → Option K

def target (D : Dates K) (_G : Graph) (s : String) : Page K :=
  match D.parse (P.key s) with
  | some d => (.journal, d)
  | none => (.page, P.key s)

def claim (D : Dates K) (f : File) : Page K :=
  if f.dir = .journals then
    match D.journalDay f.path with
    | some d => (.journal,d) | none => target P D [] (P.name f)
  else target P D [] (P.name f)

def pathLe : File → File → Bool :=
  Owner5.rankLe (fun f => Owner5.bytes f.path) Orders.fileLe

/-- Journal DIRECTORY wins. Semantic date files remain at their original paths. -/
def fileLe : File → File → Bool :=
  Owner5.rankLe (fun f => [if f.dir == .journals then 0 else 1]) pathLe

theorem fileLe_linear : Linear fileLe :=
  Owner5.rankLe_linear _ (Owner5.rankLe_linear _ Orders.fileLe_linear)

def claimants (D : Dates K) (G : Graph) (t : Page K) : Graph :=
  G.filter (fun f => decide (claim P D f = t))

def openingFile (D : Dates K) (G : Graph) (t : Page K) : Option File :=
  minBy fileLe (claimants P D G t)

def activeGraph (D : Dates K) (G : Graph) : Graph :=
  G.filter (fun f => decide (openingFile P D G (claim P D f) = some f))

def conflicted (D : Dates K) (G : Graph) (t : Page K) : Bool :=
  decide (∃ f ∈ G, ∃ g ∈ G, f ≠ g ∧ claim P D f = t ∧ claim P D g = t)

def blocks (D : Dates K) (G : Graph) : List BlockId :=
  (activeGraph P D G).flatMap (fun f =>
    (List.range (P.outline f.format f.text).length).map (f.path, ·))

def pageOfBlock (D : Dates K) (G : Graph) (b : BlockId) : Option (Page K) :=
  (fileAt (activeGraph P D G) b.1).map (claim P D)

def edges (D : Dates K) (G : Graph) : List (Page K × Page K) :=
  (activeGraph P D G).flatMap (fun f => (P.aliasDecl f.format f.text).map
    (fun s => (claim P D f, target P D G s)))

/-- Close the raw namespace spelling before interpreting each resulting name. -/
def refTargets (D : Dates K) (G : Graph) (b : BlockId) : List (Page K) :=
  (P.refNames (activeGraph P D G) b).map (target P D G)

def referencesB (D : Dates K) (G : Graph) (b : BlockId) (t : Page K) : Bool :=
  (refTargets P D G b).any (fun v => sameCls (edges P D G) v t)

def realFiles (D : Dates K) (G : Graph) (t : Page K) : Graph :=
  (activeGraph P D G).filter (fun f => decide (claim P D f = t))

def declarers (D : Dates K) (G : Graph) (t : Page K) : Graph :=
  (activeGraph P D G).filter (fun f => (P.aliasDecl f.format f.text).any
    (fun s => decide (target P D G s = t)))

/-- File-less names choose a direct declarer by path across both directories. -/
def openTarget (D : Dates K) (G : Graph) (t : Page K) : Page K :=
  if !(realFiles P D G t).isEmpty then t else
    ((minBy pathLe (declarers P D G t)).map (claim P D)).getD t

def resolve (D : Dates K) (G : Graph) (s : String) : Page K :=
  openTarget P D G (target P D G s)

def seedTargets (D : Dates K) (G : Graph) : List (Page K) :=
  ((blocks P D G).flatMap (refTargets P D G)) ++
    (activeGraph P D G).flatMap (fun f => (P.fileNsNames f).map (target P D G))

def pages (D : Dates K) (G : Graph) : List (Page K) :=
  (activeGraph P D G).map (claim P D) ++
    (seedTargets P D G).filter (fun t => (realFiles P D G t).isEmpty &&
      !(edges P D G).any (fun e => decide (e.2 = t)))

def panelSelf (D : Dates K) (G : Graph) (t : Page K) : Page K :=
  openTarget P D G t

def panelSource (D : Dates K) (favorites : Option String) (G : Graph)
    (t : Page K) (b : BlockId) : Bool :=
  match fileAt (activeGraph P D G) b.1 with
  | none => false
  | some f => decide (claim P D f ≠ panelSelf P D G t) &&
      !favorites.any (fun s => decide (P.key s ≠ P.key "") && decide (claim P D f = target P D G s))

def linked (D : Dates K) (favorites : Option String) (G : Graph) (t : Page K) : List BlockId :=
  (blocks P D G).eraseDups.filter (fun b =>
    referencesB P D G b t && panelSource P D favorites G t b)

def referenceCount (D : Dates K) (favorites : Option String) (G : Graph) (t : Page K) : Nat :=
  (linked P D favorites G t).length

def leafPage (D : Dates K) (Q : QParams K L) (G : Graph) : Leaf L → Page K → Bool
  | .refs _, _ => false
  | .name s, t => decide (target P D G s = t)
  | .page l, t => leafPage D Q G l t
  | .given l, t => Q.pageLeaf l (activeGraph P D G) t

def pathTargets (D : Dates K) (Q : QParams K L) (G : Graph) (b : BlockId) : List (Page K) :=
  refTargets P D G b ++
    (ancestors Q (activeGraph P D G) b).flatMap (refTargets P D G) ++
    ((pageOfBlock P D G b).toList)

def leafBlock (D : Dates K) (Q : QParams K L) (G : Graph) : Leaf L → BlockId → Bool
  | .refs s, b => (pathTargets P D Q G b).any (fun t => sameCls (edges P D G) (target P D G s) t)
  | .name _, _ => false
  | .page l, b => ((pageOfBlock P D G b).map (leafPage P D Q G l)).getD false
  | .given l, b => Q.blockLeaf l (activeGraph P D G) b

def leafAt (D : Dates K) (Q : QParams K L) (G : Graph) (l : Leaf L) : Node K → Bool
  | .page t => leafPage P D Q G l t
  | .block b => leafBlock P D Q G l b

def univ (D : Dates K) (G : Graph) : Anchor → List (Node K)
  | .page => (pages P D G).map Node.page
  | .block => (blocks P D G).map Node.block

def den (D : Dates K) (Q : QParams K L) (G : Graph) (a : Anchor) (q : Query L) : List (Node K) :=
  (univ P D G a).filter (fun n => q.bval (fun l => leafAt P D Q G l n))

def rowKind (D : Dates K) (R : QueryRowParser) (G : Graph) (b : BlockId) : QueryRowKind :=
  match fileAt (activeGraph P D G) b.1 with
  | some f => R.kind f.format f.text b.2
  | none => .body

/-- The last body ancestor is the immediate parent. Only that parent matters. -/
def queryParent (D : Dates K) (Q : QParams K L) (R : QueryRowParser) (G : Graph) (b : BlockId) : Option BlockId :=
  if rowKind P D R G b = .body then
    ((ancestors Q (activeGraph P D G) b).filter (fun a => rowKind P D R G a == .body)).getLast?
  else none

/-- Parent matching uses the entire raw hit list, before either UI filter. -/
def queryRowVisible (D : Dates K) (Q : QParams K L) (R : QueryRowParser) (o : QueryPresentation)
    (G : Graph) (raw : List (Node K)) : Node K → Bool
  | .page _ => true
  | .block b => (o.showHeaders || decide (rowKind P D R G b ≠ .header)) &&
      !(o.removeBlockChildren && (queryParent P D Q R G b).any (fun a => raw.contains (.block a)))

def queryProjection (D : Dates K) (Q : QParams K L) (R : QueryRowParser) (o : QueryPresentation)
    (G : Graph) (raw : List (Node K)) : List (Node K) :=
  raw.filter (queryRowVisible P D Q R o G raw)

def queryResults (D : Dates K) (Q : QParams K L) (R : QueryRowParser) (o : QueryPresentation)
    (G : Graph) (a : Anchor) (q : Query L) : List (Node K) :=
  queryProjection P D Q R o G (den P D Q G a q)

def links (D : Dates K) (config : Decided.Config) (G : Graph) (S T : Page K) : Bool :=
  decide (S ∈ pages P D G) && decide (T ∈ pages P D G) && decide (S ≠ T) &&
    decide (S ≠ target P D G config.favoritesPage) &&
    (blocks P D G).any (fun b =>
      decide (pageOfBlock P D G b = some S) && referencesB P D G b T)

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
  (activeGraph P D G).filter (fun f => sameCls (edges P D G) (target P D G s) (claim P D f))

theorem reference_exact (D : Dates K) (G : Graph) (b : BlockId) (t : Page K) :
    referencesB P D G b t = true ↔ ∃ s ∈ P.refNames (activeGraph P D G) b,
      Reach (edges P D G) (target P D G s) t := by
  simp [referencesB, refTargets, List.any_eq_true, sameCls_iff, cls]

theorem no_alias_reference (D : Dates K) {G : Graph} (h : edges P D G = []) (b : BlockId) (t : Page K) :
    referencesB P D G b t = true ↔ t ∈ refTargets P D G b := by
  simp only [referencesB, h, List.any_eq_true, sameCls_iff, cls, Reach.nil_iff]
  constructor
  · rintro ⟨v, hv, rfl⟩; exact hv
  · intro ht; exact ⟨t, ht, rfl⟩

theorem edge_exact (D : Dates K) (G : Graph) (a b : Page K) :
    (a,b) ∈ edges P D G ↔ ∃ f ∈ activeGraph P D G,
      claim P D f = a ∧ ∃ s ∈ P.aliasDecl f.format f.text, target P D G s = b := by
  simp [edges, List.mem_flatMap, List.mem_map]
  constructor
  · rintro ⟨f, hf, s, hs, ha, hb⟩; exact ⟨f, hf, ha, s, hs, hb⟩
  · rintro ⟨f, hf, ha, s, hs, hb⟩; exact ⟨f, hf, s, hs, ha, hb⟩

theorem linked_exact (D : Dates K) (favorites : Option String) (G : Graph) (t : Page K) (b : BlockId) :
    b ∈ linked P D favorites G t ↔ b ∈ blocks P D G ∧
      referencesB P D G b t = true ∧ panelSource P D favorites G t b = true := by simp [linked]

theorem count_panel (D : Dates K) (favorites : Option String) (G : Graph) (t : Page K) :
    referenceCount P D favorites G t = (linked P D favorites G t).length := rfl

theorem panelSource_exact (D : Dates K) (favorites : Option String) (G : Graph) (t : Page K) (b : BlockId) :
    panelSource P D favorites G t b = true ↔ ∃ f,
      fileAt (activeGraph P D G) b.1 = some f ∧ claim P D f ≠ panelSelf P D G t ∧
      ∀ s ∈ favorites, P.key s ≠ P.key "" → claim P D f ≠ target P D G s := by
  cases hf : fileAt (activeGraph P D G) b.1 with
  | none => simp [panelSource, hf]
  | some f =>
    cases favorites with
    | none => simp [panelSource, hf]
    | some s => by_cases he : P.key s = P.key "" <;> simp [panelSource, hf, he]

theorem pages_exact (D : Dates K) (G : Graph) (t : Page K) :
    t ∈ pages P D G ↔ (∃ f ∈ activeGraph P D G, claim P D f = t) ∨
      (t ∈ seedTargets P D G ∧ (realFiles P D G t).isEmpty = true ∧
        ∀ e ∈ edges P D G, e.2 ≠ t) := by simp [pages, Bool.and_eq_true]

theorem children_exact (D : Dates K) (spell : K → String) (G : Graph) (t c : Page K) :
    c ∈ children P D spell G t ↔ c ∈ pages P D G ∧ treeParent P D spell G c = some t := by simp [children]

theorem den_exact (D : Dates K) (Q : QParams K L) (G : Graph) (a : Anchor) (q : Query L) (n : Node K) :
    n ∈ den P D Q G a q ↔ n ∈ univ P D G a ∧ q.bval (fun l => leafAt P D Q G l n) = true := by simp [den]

theorem queryResults_exact (D : Dates K) (Q : QParams K L) (R : QueryRowParser) (o : QueryPresentation)
    (G : Graph) (a : Anchor) (q : Query L) (n : Node K) :
    n ∈ queryResults P D Q R o G a q ↔ n ∈ den P D Q G a q ∧
      queryRowVisible P D Q R o G (den P D Q G a q) n = true := by simp [queryResults, queryProjection]

theorem links_exact (D : Dates K) (config : Decided.Config) (G : Graph) (S T : Page K) :
    links P D config G S T = true ↔ S ∈ pages P D G ∧ T ∈ pages P D G ∧ S ≠ T ∧
      S ≠ target P D G config.favoritesPage ∧ ∃ b ∈ blocks P D G,
        pageOfBlock P D G b = some S ∧ referencesB P D G b T = true := by
  simp [links, Bool.and_eq_true, List.any_eq_true, and_assoc]



theorem target_exact (D : Dates K) (G : Graph) (s : String) :
    target P D G s = match D.parse (P.key s) with
      | some d => (.journal, d) | none => (.page, P.key s) := rfl

theorem target_graph_independent (D : Dates K) (G H : Graph) (s : String) :
    target P D G s = target P D H s := rfl

theorem target_date (D : Dates K) (G : Graph) {s : String} {d : K}
    (h : D.parse (P.key s) = some d) : target P D G s = (.journal,d) := by simp [target,h]

theorem same_day_target (D : Dates K) (G : Graph) {s r : String} {d : K}
    (hs : D.parse (P.key s) = some d) (hr : D.parse (P.key r) = some d) :
    target P D G s = target P D G r := by rw [target_date P D G hs, target_date P D G hr]

theorem date_not_ordinary (D : Dates K) (G : Graph) {s : String} {d k : K}
    (h : D.parse (P.key s) = some d) : target P D G s ≠ (.page,k) := by
  rw [target_date P D G h]; simp

theorem pages_date_claim (D : Dates K) {f : File} {d : K} (hf : f.dir = .pages)
    (hd : D.parse (P.key (P.name f)) = some d) : claim P D f = (.journal,d) := by
  simp [claim,hf,target,hd]

theorem journal_claim (D : Dates K) (f : File) (h : f.dir = .journals) {d : K}
    (hd : D.journalDay f.path = some d) : claim P D f = (.journal,d) := by simp [claim,h,hd]

theorem active_iff (D : Dates K) {G : Graph} {f : File} :
    f ∈ activeGraph P D G ↔ f ∈ G ∧ openingFile P D G (claim P D f) = some f := by
  simp [activeGraph]

theorem opening_member (D : Dates K) {G : Graph} {t : Page K} {f : File}
    (h : openingFile P D G t = some f) : f ∈ activeGraph P D G ∧ claim P D f = t := by
  obtain ⟨hf,hk⟩ := List.mem_filter.mp (minBy_mem h)
  have hk : claim P D f = t := of_decide_eq_true hk
  exact ⟨(active_iff P D).mpr ⟨hf,hk ▸ h⟩,hk⟩

theorem real_of_active (D : Dates K) {G : Graph} {f : File} (hf : f ∈ activeGraph P D G) :
    (realFiles P D G (claim P D f)).isEmpty = false := by
  have hm : f ∈ realFiles P D G (claim P D f) := by simp [realFiles,hf]
  cases he : realFiles P D G (claim P D f) with
  | nil => simp [he] at hm
  | cons a as => rfl

theorem active_handle_opening (D : Dates K) {G : Graph} {f : File}
    (hf : f ∈ activeGraph P D G) : openTarget P D G (claim P D f) = claim P D f := by
  simp [openTarget,real_of_active P D hf]

/-- Textual self-opening requires the displayed name to identify its day.
The approved year-less-label limitation is explicit here, not assumed globally. -/
theorem active_text_opening_when_identifying (D : Dates K) {G : Graph} {f : File}
    (hf : f ∈ activeGraph P D G) (hn : target P D G (P.name f) = claim P D f) :
    resolve P D G (P.name f) = claim P D f := by
  rw [resolve,hn]; exact active_handle_opening P D hf

theorem openTarget_reachable (D : Dates K) (G : Graph) (t : Page K) :
    Reach (edges P D G) t (openTarget P D G t) := by
  unfold openTarget
  split
  · exact Reach.refl _
  · cases hm : minBy pathLe (declarers P D G t) with
    | none => exact Reach.refl _
    | some f =>
      obtain ⟨hf,hd⟩ := List.mem_filter.mp (minBy_mem hm)
      obtain ⟨r,hr,ht⟩ := List.any_eq_true.mp hd
      exact Reach.symm (Reach.edge ((edge_exact P D G _ _).mpr
        ⟨f,hf,rfl,r,hr,of_decide_eq_true ht⟩))

theorem opening_idempotent (D : Dates K) (G : Graph) (t : Page K) :
    openTarget P D G (openTarget P D G t) = openTarget P D G t := by
  by_cases hr : (realFiles P D G t).isEmpty = false
  · have ht : openTarget P D G t = t := by simp [openTarget,hr]
    rw [ht]; exact ht
  · cases hm : minBy pathLe (declarers P D G t) with
    | none =>
      have ht : openTarget P D G t = t := by simp [openTarget,hr,hm]
      rw [ht]; exact ht
    | some f =>
      have ht : openTarget P D G t = claim P D f := by simp [openTarget,hr,hm]
      rw [ht]
      exact active_handle_opening P D (List.mem_filter.mp (minBy_mem hm)).1

theorem references_opening (D : Dates K) (G : Graph) (b : BlockId) (t : Page K) :
    referencesB P D G b (openTarget P D G t) = referencesB P D G b t := by
  apply Bool.eq_iff_iff.mpr
  rw [reference_exact,reference_exact]
  constructor
  · rintro ⟨s,hs,hr⟩
    exact ⟨s,hs,Reach.trans hr (Reach.symm (openTarget_reachable P D G t))⟩
  · rintro ⟨s,hs,hr⟩
    exact ⟨s,hs,Reach.trans hr (openTarget_reachable P D G t)⟩

theorem panel_opening (D : Dates K) (favorites : Option String) (G : Graph) (t : Page K) :
    linked P D favorites G t = linked P D favorites G (openTarget P D G t) := by
  unfold linked
  apply List.filter_congr
  intro b _
  rw [references_opening]
  simp only [panelSource,panelSelf,opening_idempotent]
  rfl

def linkedAt (D : Dates K) (favorites : Option String) (G : Graph) (s : String) : List BlockId :=
  linked P D favorites G (target P D G s)

theorem name_panel_is_opening_panel (D : Dates K) (favorites : Option String) (G : Graph) (s : String) :
    linkedAt P D favorites G s = linked P D favorites G (resolve P D G s) :=
  panel_opening P D favorites G _

theorem mention_references_opened (D : Dates K) {G : Graph} {b : BlockId} {s : String}
    (h : s ∈ P.refNames (activeGraph P D G) b) : referencesB P D G b (resolve P D G s) = true :=
  (reference_exact P D G b _).mpr ⟨s,h,openTarget_reachable P D G _⟩

end Date13
end CM
