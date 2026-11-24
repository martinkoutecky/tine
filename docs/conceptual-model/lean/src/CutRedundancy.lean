import CutNames
namespace CM
namespace Cut
open Consolidated13
variable {K : Type} [DecidableEq K] (P : Inputs K) (D : Date13.Dates K)
theorem zip_projections {A B : Type} {xs : List A} {ys : List B}
    (hl : xs.length = ys.length) :
    (xs.zip ys).map Prod.fst = xs ∧ (xs.zip ys).map Prod.snd = ys := by
  induction xs generalizing ys with
  | nil => cases ys <;> simp_all
  | cons a xs ih =>
    cases ys with
    | nil => simp at hl
    | cons b ys =>
      have ht : xs.length = ys.length := Nat.succ.inj hl
      have hi := ih ht
      simpa [List.zip_cons_cons] using hi

theorem wf_injective {G : Graph} (h : WF G) {f g : File}
    (hf : f ∈ G) (hg : g ∈ G) (hp : f.path = g.path) : f = g := by
  induction G generalizing f g with
  | nil => simp at hf
  | cons a as ih =>
    simp only [WF,List.map_cons,List.nodup_cons] at h
    rcases List.mem_cons.mp hf with hfa | hft
    · rcases List.mem_cons.mp hg with hga | hgt
      · exact hfa.trans hga.symm
      · exact False.elim (h.1 (List.mem_map.mpr ⟨g,hgt,hp.symm.trans (congrArg File.path hfa)⟩))
    · rcases List.mem_cons.mp hg with hga | hgt
      · exact False.elim (h.1 (List.mem_map.mpr ⟨f,hft,hp.trans (congrArg File.path hga)⟩))
      · exact ih h.2 hft hgt hp

theorem claim_name {G : Graph} {f : File} (hd : (Date13.claim P D f).1 ≠ .journal) :
    Date13.claim P D f = Date13.target P D G (P.name f) := by
  by_cases hj : f.dir = .journals
  · cases he : D.journalDay f.path with
    | some day => simp [Date13.claim,hj,he] at hd
    | none => simp [Date13.claim,hj,he,Date13.target]
  · simp [Date13.claim,hj,Date13.target]


theorem retained_mem {G : Graph} {p : Plan} {f : File} (h : f ∈ retained G p) : f ∈ G := by
  unfold retained at h
  split at h
  · exact (List.mem_filter.mp h).1
  · exact h

theorem retained_wf {G : Graph} (h : WF G) (p : Plan) : WF (retained G p) := by
  unfold retained
  split
  · exact (List.Nodup.sublist (by exact (List.filter_sublist).map File.path) h)
  · exact h

/-- A zip with a duplicate-free right side associates each right element with
one left element, with no writer-supplied origin certificate. -/
theorem zip_left_unique {A B : Type} {xs : List A} {ys : List B} (hn : ys.Nodup)
    {f g : A} {h : B} (hf : (f,h) ∈ xs.zip ys) (hg : (g,h) ∈ xs.zip ys) : f = g := by
  induction xs generalizing ys with
  | nil => simp at hf
  | cons x xs ih =>
    cases ys with
    | nil => simp at hf
    | cons y ys =>
      obtain ⟨hnot,hnd⟩ := List.nodup_cons.mp hn
      simp only [List.zip_cons_cons,List.mem_cons] at hf hg
      rcases hf with he | ht
      · cases he
        rcases hg with he | ht
        · cases he; rfl
        · have hm : h ∈ ys := (List.of_mem_zip ht).2
          exact False.elim (hnot hm)
      · rcases hg with he | hgt
        · cases he
          have hm : h ∈ ys := (List.of_mem_zip ht).2
          exact False.elim (hnot hm)
        · exact ih hnd ht hgt

/-- Folded output-path protection already implies ordinary output-path
uniqueness when the input paths and zip cardinality are checked. -/
theorem post_paths_nodup {O : Observations} {xs H : Graph}
    (hw : WF xs) (hl : xs.length = H.length)
    (folded : ∀ f h, (f,h) ∈ xs.zip H → ∀ g k, (g,k) ∈ xs.zip H → f ≠ g →
      h.path ≠ f.path ∨ k.path ≠ g.path → O.foldedPath h.path ≠ O.foldedPath k.path) : WF H := by
  induction xs generalizing H with
  | nil => cases H <;> simp_all [WF]
  | cons f fs ih =>
    cases H with
    | nil => simp at hl
    | cons h hs =>
      obtain ⟨hnot,hnd⟩ := List.nodup_cons.mp hw
      have hlen : fs.length = hs.length := Nat.succ.inj hl
      have sub : ∀ f h, (f,h) ∈ fs.zip hs → (f,h) ∈ (f :: fs).zip (h :: hs) := by
        intro a b hz
        exact List.mem_cons_of_mem _ hz
      refine List.nodup_cons.mpr ⟨?_,?_⟩
      · intro hm
        obtain ⟨k,hk,hkp⟩ := List.mem_map.mp hm
        have proj := (zip_projections hlen).2
        obtain ⟨pair,hpair,he⟩ := List.mem_map.mp (show k ∈ (fs.zip hs).map Prod.snd by rw [proj]; exact hk)
        rcases pair with ⟨g,k'⟩
        dsimp only at he
        subst k'
        have hgf : f ≠ g := by
          intro heq
          have gm := (List.of_mem_zip hpair).1
          subst g
          exact hnot (List.mem_map.mpr ⟨f,gm,rfl⟩)
        have fold := folded f h (List.mem_cons_self ..) g k (List.mem_cons_of_mem _ hpair) hgf
        by_cases hp : h.path = f.path
        · by_cases kp : k.path = g.path
          · exact hnot (List.mem_map.mpr ⟨g,(List.of_mem_zip hpair).1,kp.symm.trans (hkp.trans hp)⟩)
          · exact fold (Or.inr kp) (congrArg O.foldedPath hkp.symm)
        · exact fold (Or.inl hp) (congrArg O.foldedPath hkp.symm)
      · exact ih hnd hlen (fun a b hab c d hcd =>
          folded a b (List.mem_cons_of_mem _ hab) c d (List.mem_cons_of_mem _ hcd))

/-- Any changed intended claim is the image of a touched input name. -/
theorem changed_claim_image_touched {G : Graph} {old : String} {p : Plan} {f : File}
    (hf : f ∈ retained G p) (hc : intendedClaim P D old p f ≠ Date13.claim P D f) :
    ∃ n ∈ touchedNames P D G old p, Date13.target P D [] n = intendedClaim P D old p f := by
  have hd : (Date13.claim P D f).1 ≠ .journal := by
    intro hd; exact hc (by simp [intendedClaim,hd])
  have hs : some f ≠ p.survivor := by
    intro hs; exact hc (by simp [intendedClaim,hs])
  have cn := claim_name P D (G := G) hd
  have hdate : (Date13.target P D [] (P.name f)).1 ≠ .journal := by
    change (Date13.target P D G (P.name f)).1 ≠ .journal
    rw [← cn]; exact hd
  have hm : (nsMatch P.key P.urlHasOrigin old (P.name f)).isSome = true := by
    cases hh : nsMatch P.key P.urlHasOrigin old (P.name f) with
    | some i => rfl
    | none =>
      have hn : nameMap P D old p.destination (P.name f) = P.name f := by
        simp [nameMap,hdate,nsSubst,hh]
      exact False.elim (hc (by simpa [intendedClaim,hd,hs,hn,Date13.target] using cn.symm))
  have all : P.name f ∈ allNames P G :=
    List.mem_flatMap.mpr ⟨f,retained_mem hf,List.mem_cons_self ..⟩
  have touched : P.name f ∈ (old :: (old :: allNames P G).filter (fun s =>
      (nsMatch P.key P.urlHasOrigin old s).isSome && Decidable.decide ((Date13.target P D [] s).1 ≠ .journal)) ++
      (p.source.toList.flatMap (fun f => P.aliasDecl f.format f.text)).filter (fun s =>
        Decidable.decide ((Date13.target P D [] s).1 ≠ .journal))) := by
    simp [all,hm,hdate]
  refine ⟨nameMap P D old p.destination (P.name f),?_,?_⟩
  · unfold touchedNames
    apply List.mem_append_right
    exact List.mem_map.mpr ⟨P.name f,touched,rfl⟩
  · simp [intendedClaim,hd,hs]

/-- The remaining actual identity and touched-clean checks rule out every new
pair, provided paths/cardinality bind distinct output files. -/
theorem post_no_new_twins {O : Observations} {G H : Graph} {old : String} {p : Plan}
    (hw : WF H)
    (clean : ∀ n ∈ touchedNames P D G old p, Date13.conflicted P D H (Date13.target P D H n) = false)
    (identity : ∀ f h, (f,h) ∈ (retained G p).zip H → Date13.claim P D h = intendedClaim P D old p f) :
    ∀ f h, (f,h) ∈ (retained G p).zip H → ∀ g k, (g,k) ∈ (retained G p).zip H → f ≠ g →
      Date13.claim P D f ≠ Date13.claim P D g → Date13.claim P D h ≠ Date13.claim P D k := by
  intro f h hfh g k hgk hfg hclaims hout
  have hn : h ≠ k := by
    intro he
    subst k
    have nd : H.Nodup := List.Pairwise.of_map File.path (fun a b hn he => hn (congrArg File.path he)) hw
    exact hfg (zip_left_unique nd hfh hgk)
  have touched : ∃ n ∈ touchedNames P D G old p, Date13.target P D H n = Date13.claim P D h := by
    by_cases hf : intendedClaim P D old p f = Date13.claim P D f
    · have hg : intendedClaim P D old p g ≠ Date13.claim P D g := by
        intro hg
        exact hclaims ((hf.symm.trans (identity f h hfh).symm).trans
          (hout.trans ((identity g k hgk).trans hg)))
      obtain ⟨n,hm,he⟩ := changed_claim_image_touched P D ((List.of_mem_zip hgk).1) hg
      exact ⟨n,hm,he.trans ((identity g k hgk).symm.trans hout.symm)⟩
    · obtain ⟨n,hm,he⟩ := changed_claim_image_touched P D ((List.of_mem_zip hfh).1) hf
      exact ⟨n,hm,he.trans (identity f h hfh).symm⟩
  obtain ⟨n,hm,he⟩ := touched
  have bad : Date13.conflicted P D H (Date13.target P D H n) = true :=
    decide_eq_true ⟨h,(List.of_mem_zip hfh).2,k,(List.of_mem_zip hgk).2,hn,he.symm,hout.symm.trans he.symm⟩
  exact Bool.noConfusion ((clean n hm).symm.trans bad)
end Cut
end CM
