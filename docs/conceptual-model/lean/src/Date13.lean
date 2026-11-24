import Claimant13
set_option linter.unusedSectionVars false
namespace CM
namespace Date13
open Params Consolidated13
variable {K L : Type} [DecidableEq K] (P : Inputs K)

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

def fileLe : File → File → Bool :=
  Owner5.rankLe (fun f => [if f.dir == .journals then 0 else 1]) pathLe

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

def refTargets (D : Dates K) (G : Graph) (b : BlockId) : List (Page K) :=
  (P.refNames (activeGraph P D G) b).map (target P D G)

def referencesB (D : Dates K) (G : Graph) (b : BlockId) (t : Page K) : Bool :=
  (refTargets P D G b).any (fun v => sameCls (edges P D G) v t)

def realFiles (D : Dates K) (G : Graph) (t : Page K) : Graph :=
  (activeGraph P D G).filter (fun f => decide (claim P D f = t))

def declarers (D : Dates K) (G : Graph) (t : Page K) : Graph :=
  (activeGraph P D G).filter (fun f => (P.aliasDecl f.format f.text).any
    (fun s => decide (target P D G s = t)))

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

theorem pages_exact (D : Dates K) (G : Graph) (t : Page K) :
    t ∈ pages P D G ↔ (∃ f ∈ activeGraph P D G, claim P D f = t) ∨
      (t ∈ seedTargets P D G ∧ (realFiles P D G t).isEmpty = true ∧
        ∀ e ∈ edges P D G, e.2 ≠ t) := by simp [pages, Bool.and_eq_true]


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

theorem mention_references_opened (D : Dates K) {G : Graph} {b : BlockId} {s : String}
    (h : s ∈ P.refNames (activeGraph P D G) b) : referencesB P D G b (resolve P D G s) = true :=
  (reference_exact P D G b _).mpr ⟨s,h,openTarget_reachable P D G _⟩

def outsideSnapshot (_before after : Graph) : Graph := after

theorem outside_reads (before after : Graph) : outsideSnapshot before after = after := rfl

theorem outside_content (before : Graph) (f : File) (path : String) (dir : Dir) :
    ({f with path := path, dir := dir} : File).text = f.text := rfl

end Date13
end CM
