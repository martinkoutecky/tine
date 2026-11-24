import CutConservation
namespace CM
namespace Cut
open Consolidated13
variable {K : Type} [DecidableEq K] (P : Inputs K) (D : Date13.Dates K)

private theorem twins_authorize_plan {O : Observations} {G : Graph} {r : Request}
    {p q : Plan} {a : Option Question} (h : authorize P D O G r q a = .success p) : p = q := by
  unfold authorize at h
  split at h <;> try contradiction
  split at h <;> try contradiction
  cases h
  rfl

private theorem twins_merge_source {O : Observations} {G : Graph} {r : Request}
    {s : Option File} {t : File} {a : Option Question} {p : Plan}
    (h : mergeSelection P D O G r s t a = .success p) : p.source = s := by
  unfold mergeSelection at h
  split at h
  · contradiction
  · have he := twins_authorize_plan P D h
    cases he
    rfl

private theorem twins_selected_source {O : Observations} {G : Graph} {r : Request} {p : Plan}
    (h : select P D O G r = .success p) : p.source = sourceFile P D G r.source := by
  have hc := attendance_applied h
  simp only [selectCore] at hc
  split at hc
  · contradiction
  · split at hc
    · contradiction
    · split at hc
      · contradiction
      · try dsimp only at hc
        split at hc
        · contradiction
        · split at hc
          · exact twins_merge_source P D hc
          · exact twins_merge_source P D hc
        · try dsimp only at hc
          split at hc
          · contradiction
          · split at hc
            · contradiction
            · have he := twins_authorize_plan P D hc
              cases he
              rfl

private theorem twins_source_claim {G : Graph} {old : String} {f : File}
    (h : sourceFile P D G old = some f) : Date13.claim P D f = Date13.target P D G old := by
  unfold sourceFile at h
  generalize he : Date13.claimants P D G (Date13.target P D G old) = fs at h
  cases fs with
  | nil => contradiction
  | cons a as =>
    cases as with
    | cons b bs => contradiction
    | nil =>
      cases h
      have hm : f ∈ Date13.claimants P D G (Date13.target P D G old) := by rw [he]; simp
      exact of_decide_eq_true (List.mem_filter.mp hm).2

private theorem twins_all_name {G : Graph} {f : File} (hf : f ∈ G) : P.name f ∈ allNames P G := by
  exact List.mem_flatMap.mpr ⟨f,hf,List.mem_cons_self ..⟩

private theorem twins_claim_name {G : Graph} {f : File} (hd : (Date13.claim P D f).1 ≠ .journal) :
    Date13.claim P D f = Date13.target P D G (P.name f) := by
  by_cases hj : f.dir = .journals
  · cases he : D.journalDay f.path with
    | some day => simp [Date13.claim,hj,he] at hd
    | none => simp [Date13.claim,hj,he,Date13.target]
  · simp [Date13.claim,hj,Date13.target]

private theorem twins_original_still {O : Observations} {E : Engine} {G : Graph}
    {r : Request} {d : Done} (h : decide P D O E G r = .success d)
    {k : Page K} (hc : Date13.conflicted P D G k = true) {f : File}
    (hf : f ∈ G) (hk : Date13.claim P D f = k) :
    f ∈ retained G d.plan ∧ expectedPath P D O r.source d.plan f = f.path ∧
    intendedClaim P D r.source d.plan f = k := by
  have bs := (done_name_safety P D h).1
  obtain ⟨p,hp,he,_,_⟩ := done_checked P D h
  have hsource : d.plan.source = sourceFile P D G r.source := by rw [he]; exact twins_selected_source P D hp
  have hn : some f ≠ d.plan.source := by
    intro hs
    have hs' : sourceFile P D G r.source = some f := hsource.symm.trans hs.symm
    have hc' := twins_source_claim P D hs'
    have hcl := bs.clean r.source (by simp [touchedNames])
    rw [← hc',hk,hc] at hcl
    contradiction
  have hr : f ∈ retained G d.plan := by
    unfold retained
    split
    · exact List.mem_filter.mpr ⟨hf,by simpa using hn⟩
    · exact hf
  refine ⟨hr,?_,?_⟩
  · by_cases hd : (Date13.claim P D f).1 = .journal
    · simp [expectedPath,hd]
    · have hclaim := twins_claim_name P D (G := G) hd
      have hm : nsMatch P.key P.urlHasOrigin r.source (P.name f) = none := by
        cases hm : nsMatch P.key P.urlHasOrigin r.source (P.name f) with
        | none => rfl
        | some n =>
          have ht : P.name f ∈ touchedNames P D G r.source d.plan := by
            unfold touchedNames
            apply List.mem_append_left
            apply List.mem_cons_of_mem
            apply List.mem_append_left
            exact List.mem_filter.mpr ⟨List.mem_cons_of_mem _ (twins_all_name P hf),by
              simp only [hm,Option.isSome_some,Bool.true_and,decide_eq_true_eq]
              change (Date13.target P D G (P.name f)).1 ≠ .journal
              rw [← hclaim]
              exact hd⟩
          have hh := bs.clean (P.name f) ht
          rw [← hclaim,hk,hc] at hh
          contradiction
      have hmap : nameMap P D r.source d.plan.destination (P.name f) = P.name f := by
        unfold nameMap nsSubst
        split <;> simp [hm]
      simp [expectedPath,mappedName,hmap]
  · by_cases hd : (Date13.claim P D f).1 = .journal
    · have hdk : k.1 = .journal := hk ▸ hd
      simp [intendedClaim,hk,hdk]
    · by_cases hs : some f = d.plan.survivor
      · simp [intendedClaim,hs,hk]
      · have hclaim := twins_claim_name P D (G := G) hd
        have hm : nsMatch P.key P.urlHasOrigin r.source (P.name f) = none := by
          cases hm : nsMatch P.key P.urlHasOrigin r.source (P.name f) with
          | none => rfl
          | some n =>
            have ht : P.name f ∈ touchedNames P D G r.source d.plan := by
              unfold touchedNames
              apply List.mem_append_left
              apply List.mem_cons_of_mem
              apply List.mem_append_left
              exact List.mem_filter.mpr ⟨List.mem_cons_of_mem _ (twins_all_name P hf),by
                simp only [hm,Option.isSome_some,Bool.true_and,decide_eq_true_eq]
                change (Date13.target P D G (P.name f)).1 ≠ .journal
                rw [← hclaim]
                exact hd⟩
            have hh := bs.clean (P.name f) ht
            rw [← hclaim,hk,hc] at hh
            contradiction
        have hmap : nameMap P D r.source d.plan.destination (P.name f) = P.name f := by
          unfold nameMap nsSubst
          split <;> simp [hm]
        simpa [intendedClaim,hd,hs,hmap,Date13.target] using hclaim.symm.trans hk

private theorem twins_zip_projections {A B : Type} {xs : List A} {ys : List B}
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

private theorem twins_filter_map {A B : Type} (xs : List A) (f : A → B) (pred : B → Bool) :
    (xs.filter (fun a => pred (f a))).map f = (xs.map f).filter pred := by
  induction xs with
  | nil => rfl
  | cons a xs ih => cases hp : pred (f a) <;> simp [hp,ih]

private theorem twins_wf_injective {G : Graph} (h : WF G) {f g : File}
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

private theorem twins_bytes_injective {s t : String} (h : Owner5.bytes s = Owner5.bytes t) : s = t := by
  have h' := (List.map_inj_right (fun a b => UInt8.toNat_inj.mp)).mp h
  have h'' := Array.toList_inj.mp h'
  exact String.toByteArray_inj.mp (congrArg ByteArray.mk h'')

private theorem twins_min_map {A B : Type} (le : A → A → Bool) (le' : B → B → Bool)
    (F : A → B) (xs : List A)
    (h : ∀ a ∈ xs, ∀ b ∈ xs, le' (F a) (F b) = le a b) :
    minBy le' (xs.map F) = (minBy le xs).map F := by
  induction xs with
  | nil => rfl
  | cons a xs ih =>
    have ht : ∀ x ∈ xs, ∀ y ∈ xs, le' (F x) (F y) = le x y :=
      fun x hx y hy => h x (List.mem_cons_of_mem _ hx) y (List.mem_cons_of_mem _ hy)
    simp only [List.map_cons,minBy,ih ht]
    cases hm : minBy le xs with
    | none => rfl
    | some b =>
      have hab := h a (List.mem_cons_self ..) b (List.mem_cons_of_mem _ (minBy_mem hm))
      cases he : le a b <;> simp [hab,he]

private theorem twins_filter_filter {A : Type} (xs : List A) (p q : A → Bool)
    (h : ∀ x ∈ xs, q x = true → p x = true) : (xs.filter p).filter q = xs.filter q := by
  induction xs with
  | nil => rfl
  | cons a xs ih =>
    have ht : ∀ x ∈ xs, q x = true → p x = true := fun x hx => h x (List.mem_cons_of_mem _ hx)
    have hh := ih ht
    cases hp : p a <;> cases hq : q a <;> simp [hp,hq,hh]
    exact False.elim (Bool.false_ne_true (hp.symm.trans (h a (List.mem_cons_self ..) hq)))

/-- Every previously contested name still opens the same physical path after
any done name request. No output-preservation or adapter premise is added. -/
theorem done_stationary_twins_opening {O : Observations} {E : Engine} {G : Graph}
    {r : Request} {d : Done} (h : decide P D O E G r = .success d)
    {k : Page K} (hc : Date13.conflicted P D G k = true) :
    (Date13.openingFile P D d.candidate.graph k).map File.path =
      (Date13.openingFile P D G k).map File.path := by
  let H := d.candidate.graph
  let pairs := (retained G d.plan).zip H
  let atKey := fun f => Decidable.decide (Date13.claim P D f = k)
  let zs := pairs.filter (fun z => atKey z.1)
  have bs := (done_name_safety P D h).1
  have ass := (done_name_safety P D h).2
  have proj : pairs.map Prod.fst = retained G d.plan ∧ pairs.map Prod.snd = H :=
    twins_zip_projections ass.cardinality.symm
  have first : ∀ z ∈ pairs, z.1 ∈ retained G d.plan := by
    intro z hz
    rw [← proj.1]
    exact List.mem_map_of_mem hz
  have second : ∀ z ∈ pairs, z.2 ∈ H := by
    intro z hz
    rw [← proj.2]
    exact List.mem_map_of_mem hz
  have original : ∀ f ∈ retained G d.plan, f ∈ G := by
    intro f hf
    unfold retained at hf
    split at hf
    · exact (List.mem_filter.mp hf).1
    · exact hf
  have facts : ∀ z ∈ pairs, Date13.claim P D z.1 = k →
      Date13.claim P D z.2 = k ∧ z.2.path = z.1.path ∧ z.2.dir = z.1.dir := by
    intro z hz hk
    have hf := original z.1 (first z hz)
    have keep := twins_original_still P D h hc hf hk
    have out := done_pair_observations P D h hz
    exact ⟨(ass.identity z.1 z.2 hz).trans keep.2.2,out.1.trans keep.2.1,out.2.1⟩
  have haveTwin : ∃ a ∈ G, Date13.claim P D a = k := by
    have hx : ∃ f ∈ G, ∃ g ∈ G, f ≠ g ∧ Date13.claim P D f = k ∧ Date13.claim P D g = k :=
      of_decide_eq_true hc
    obtain ⟨a,ha,b,hb,hn,hak,hbk⟩ := hx
    exact ⟨a,ha,hak⟩
  obtain ⟨a,ha,hak⟩ := haveTwin
  have ham : a ∈ pairs.map Prod.fst := by
    rw [proj.1]
    exact (twins_original_still P D h hc ha hak).1
  obtain ⟨base,hbase,hba⟩ := List.mem_map.mp ham
  have hbk : Date13.claim P D base.1 = k := hba ▸ hak
  have hbout := (facts base hbase hbk).1
  have both : ∀ z ∈ pairs, Date13.claim P D z.2 = k ↔ Date13.claim P D z.1 = k := by
    intro z hz
    constructor
    · intro hout
      apply Classical.byContradiction
      intro hnot
      have hneq : z.1 ≠ base.1 := by intro he; exact hnot (he ▸ hbk)
      have hclaims : Date13.claim P D z.1 ≠ Date13.claim P D base.1 := by simpa [hbk] using hnot
      exact (ass.noNewTwins z.1 z.2 hz base.1 base.2 hbase hneq hclaims) (hout.trans hbout.symm)
    · exact fun hinput => (facts z hz hinput).1
  have keptMatches : (retained G d.plan).filter atKey = G.filter atKey := by
    unfold retained
    split
    · rename_i hs
      apply twins_filter_filter
      intro f hf hmatch
      have hk : Date13.claim P D f = k := of_decide_eq_true hmatch
      have hm := (twins_original_still P D h hc hf hk).1
      have hh : f ∈ G.filter (fun f => some f != d.plan.source) := by
        simpa [retained,hs] using hm
      exact (List.mem_filter.mp hh).2
    · rfl
  have left : zs.map Prod.fst = G.filter atKey := by
    exact (twins_filter_map pairs Prod.fst atKey).trans (by rw [proj.1,keptMatches])
  have predicates : pairs.filter (fun z => atKey z.1) = pairs.filter (fun z => atKey z.2) := by
    apply List.filter_congr
    intro z hz
    apply Bool.eq_iff_iff.mpr
    simp only [atKey,decide_eq_true_eq]
    exact (both z hz).symm
  have right : zs.map Prod.snd = H.filter atKey := by
    change (pairs.filter (fun z => atKey z.1)).map Prod.snd = H.filter atKey
    rw [predicates,twins_filter_map,proj.2]
  have order : ∀ z ∈ zs, ∀ w ∈ zs, Date13.fileLe z.2 w.2 = Date13.fileLe z.1 w.1 := by
    intro z hz w hw
    obtain ⟨hz,hzk⟩ := List.mem_filter.mp hz
    obtain ⟨hw,hwk⟩ := List.mem_filter.mp hw
    have fz := facts z hz (of_decide_eq_true hzk)
    have fw := facts w hw (of_decide_eq_true hwk)
    by_cases hp : z.1.path = w.1.path
    · have hbefore := twins_wf_injective bs.wellFormed (original _ (first z hz)) (original _ (first w hw)) hp
      have hafter := twins_wf_injective ass.wellFormed (second z hz) (second w hw)
        (fz.2.1.trans (hp.trans fw.2.1.symm))
      rw [hbefore,hafter]
      simp [Date13.fileLe,Date13.pathLe,Owner5.rankLe,Orders.fileLe]
    · have hb : Owner5.bytes z.1.path ≠ Owner5.bytes w.1.path := fun he => hp (twins_bytes_injective he)
      simp [Date13.fileLe,Date13.pathLe,Owner5.rankLe,fz.2.1,fw.2.1,fz.2.2,fw.2.2,hb]
  have ml := twins_min_map (fun a b : File × File => Date13.fileLe a.1 b.1) Date13.fileLe Prod.fst zs
    (fun _ _ _ _ => rfl)
  have mr := twins_min_map (fun a b : File × File => Date13.fileLe a.1 b.1) Date13.fileLe Prod.snd zs order
  change (minBy Date13.fileLe (H.filter atKey)).map File.path =
    (minBy Date13.fileLe (G.filter atKey)).map File.path
  rw [← right,← left,mr,ml]
  cases hm : minBy (fun a b : File × File => Date13.fileLe a.1 b.1) zs with
  | none => rfl
  | some z =>
    have hz := List.mem_filter.mp (minBy_mem hm)
    exact congrArg some (facts z hz.1 (of_decide_eq_true hz.2)).2.1

/-- Every original physical file claiming a day has a corresponding output
with that same day and physical path, including pages-directory day files. -/
theorem done_all_day_files {O : Observations} {E : Engine} {G : Graph}
    {r : Request} {d : Done} (h : decide P D O E G r = .success d)
    {f : File} (hf : f ∈ G) (hd : (Date13.claim P D f).1 = .journal) :
    ∃ out ∈ d.candidate.graph, Date13.claim P D out = Date13.claim P D f ∧ out.path = f.path := by
  obtain ⟨p,hp,he,_,_⟩ := done_checked P D h
  have hsource : d.plan.source = sourceFile P D G r.source := by
    rw [he]
    exact twins_selected_source P D hp
  have hn : some f ≠ d.plan.source := by
    intro hs
    have hsc := twins_source_claim P D (hsource.symm.trans hs.symm)
    exact (selected_source P D hp).2 (hsc ▸ hd)
  have hret : f ∈ retained G d.plan := by
    unfold retained
    split
    · exact List.mem_filter.mpr ⟨hf,by simpa using hn⟩
    · exact hf
  have proj := twins_zip_projections (done_name_safety P D h).2.cardinality.symm
  have hm : f ∈ ((retained G d.plan).zip d.candidate.graph).map Prod.fst := by
    rw [proj.1]
    exact hret
  obtain ⟨pair,hpair,hfirst⟩ := List.mem_map.mp hm
  rcases pair with ⟨input,out⟩
  dsimp only at hfirst
  subst input
  have hout : out ∈ d.candidate.graph := by
    rw [← proj.2]
    exact List.mem_map_of_mem hpair
  exact ⟨out,hout,done_day_files P D h hpair hd⟩

end Cut
end CM
