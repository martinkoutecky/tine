/- W2: input-based agreement on a finite alias-closed region.

Agreeing only on files NAMED in the region is insufficient for reference-only
pages: their existence depends on incoming occurrences and file-name parents. Protect those source
files' claimant sets too. No output equality is assumed in Agreement. -/
import Decided
set_option linter.unusedSectionVars false

namespace CM
namespace Isolation
open Params
variable {K : Type} [DecidableEq K] (P : Params K)

def rawKeys (f : File) : List K :=
  (P.outline f.format f.text).flatMap (fun t =>
    (P.occ f.format t).flatMap (fun o => (P.occClosure o.2).map P.key))

/-- Raw page-creation dependencies include real-file-name parents (CM3-3).
These are input attributes, not page or query results. -/
def supportKeys (f : File) : List K := rawKeys P f ++ (P.fileNsNames f).map P.key

/-- Include source keys of incoming occurrences and file-name parents, so owner
displacement or a removed name-only source cannot change a protected page. -/
def protectedKeys (S : List K) (G H : Graph) : List K :=
  S ++ ((G ++ H).filter (fun f => (supportKeys P f).any (fun k => decide (k ∈ S)))).map
    (fun f => (P.claim f).2)

def Closed (S : List K) (G : Graph) : Prop :=
  ∀ e ∈ P.aliasEdges G, e.1 ∈ S ↔ e.2 ∈ S

structure Agreement (S : List K) (G H : Graph) : Prop where
  files : ∀ f, (P.claim f).2 ∈ protectedKeys P S G H → (f ∈ G ↔ f ∈ H)
  closedG : Closed P S G
  closedH : Closed P S H

def Clean (S : List K) (G : Graph) : Prop := ∀ k ∈ S, ¬ P.Conflicted G k

def agreementB (S : List K) (G H : Graph) : Bool :=
  (G ++ H).all (fun f => decide ((P.claim f).2 ∈ protectedKeys P S G H →
    (f ∈ G ↔ f ∈ H))) &&
  (P.aliasEdges G).all (fun e => decide (e.1 ∈ S ↔ e.2 ∈ S)) &&
  (P.aliasEdges H).all (fun e => decide (e.1 ∈ S ↔ e.2 ∈ S))

theorem agreementB_iff {S : List K} {G H : Graph} :
    agreementB P S G H = true ↔ Agreement P S G H := by
  simp only [agreementB, Bool.and_eq_true, List.all_eq_true, decide_eq_true_eq]
  constructor
  · rintro ⟨⟨hf, hG⟩, hH⟩
    refine ⟨?_, hG, hH⟩
    intro f hd
    by_cases hm : f ∈ G ++ H
    · exact hf f hm hd
    · have hnG : f ∉ G := fun h => hm (List.mem_append_left _ h)
      have hnH : f ∉ H := fun h => hm (List.mem_append_right _ h)
      simp [hnG, hnH]
  · intro A
    exact ⟨⟨fun f _ => A.files f, A.closedG⟩, A.closedH⟩

instance {S : List K} {G H : Graph} : Decidable (Agreement P S G H) :=
  decidable_of_iff (agreementB P S G H = true) (agreementB_iff P)

theorem protected_symm (S : List K) (G H : Graph) (k : K) :
    k ∈ protectedKeys P S G H ↔ k ∈ protectedKeys P S H G := by
  simp only [protectedKeys, List.mem_append, List.mem_map, List.mem_filter, List.mem_append]
  constructor <;> rintro (hs | ⟨f, ⟨hf, ht⟩, he⟩)
  · exact Or.inl hs
  · exact Or.inr ⟨f, ⟨hf.symm, ht⟩, he⟩
  · exact Or.inl hs
  · exact Or.inr ⟨f, ⟨hf.symm, ht⟩, he⟩

theorem agreement_symm {S : List K} {G H : Graph} (A : Agreement P S G H) :
    Agreement P S H G :=
  ⟨fun f hf => (A.files f ((protected_symm P S H G _).mp hf)).symm,
   A.closedH, A.closedG⟩

theorem region_protected {S : List K} {G H : Graph} {k : K} (hk : k ∈ S) :
    k ∈ protectedKeys P S G H := List.mem_append_left _ hk

theorem support_protected {S : List K} {G H : Graph} {f : File}
    (hf : f ∈ G ∨ f ∈ H) {k : K} (hk : k ∈ S) (hr : k ∈ rawKeys P f) :
    (P.claim f).2 ∈ protectedKeys P S G H := by
  apply List.mem_append_right
  exact List.mem_map.mpr ⟨f, List.mem_filter.mpr ⟨List.mem_append.mpr hf,
    List.any_eq_true.mpr ⟨k, List.mem_append_left _ hr, by simp [hk]⟩⟩, rfl⟩

theorem name_support_protected {S : List K} {G H : Graph} {f : File}
    (hf : f ∈ G ∨ f ∈ H) {k : K} (hk : k ∈ S)
    (hr : k ∈ (P.fileNsNames f).map P.key) :
    (P.claim f).2 ∈ protectedKeys P S G H := by
  apply List.mem_append_right
  exact List.mem_map.mpr ⟨f, List.mem_filter.mpr ⟨List.mem_append.mpr hf,
    List.any_eq_true.mpr ⟨k, List.mem_append_right _ hr, by simp [hk]⟩⟩, rfl⟩

theorem owner_agree (hP : P.Lawful) {S : List K} {G H : Graph}
    (A : Agreement P S G H) {p : Page K} (hp : p.2 ∈ protectedKeys P S G H) :
    P.owner G p = P.owner H p := by
  apply minBy_congr hP.file
  intro f
  simp only [P.mem_claimants]
  constructor <;> rintro ⟨hf, he⟩
  · exact ⟨(A.files f (he ▸ hp)).mp hf, he⟩
  · exact ⟨(A.files f (he ▸ hp)).mpr hf, he⟩

theorem real_agree {S : List K} {G H : Graph} (A : Agreement P S G H)
    {p : Page K} (hp : p.2 ∈ protectedKeys P S G H) : P.real G p = P.real H p := by
  apply Bool.eq_iff_iff.mpr
  simp only [P.real_iff]
  constructor <;> rintro ⟨f, hf, he⟩
  · exact ⟨f, (A.files f (he ▸ hp)).mp hf, he⟩
  · exact ⟨f, (A.files f (he ▸ hp)).mpr hf, he⟩

theorem solid_agree {S : List K} {G H : Graph} (A : Agreement P S G H)
    {p : Page K} (hp : p.2 ∈ protectedKeys P S G H) : P.solid G p = P.solid H p := by
  apply Bool.eq_iff_iff.mpr
  simp only [P.solid_iff]
  constructor <;> rintro ⟨f, hf, he, hc⟩
  · exact ⟨f, (A.files f (he ▸ hp)).mp hf, he, hc⟩
  · exact ⟨f, (A.files f (he ▸ hp)).mpr hf, he, hc⟩

theorem edge_agree (hP : P.Lawful) {S : List K} {G H : Graph}
    (A : Agreement P S G H) {a b : K} (ha : a ∈ S) :
    (a, b) ∈ P.aliasEdges G ↔ (a, b) ∈ P.aliasEdges H := by
  have step : ∀ (G H : Graph), Agreement P S G H →
      (a, b) ∈ P.aliasEdges G → (a, b) ∈ P.aliasEdges H := by
    intro G H A he
    obtain ⟨f, hf, hc, s, hs, hk⟩ := P.mem_aliasEdges.mp he
    obtain ⟨hf, ho, hkind⟩ := P.mem_declaringOwners.mp hf
    have hprot : (P.claim f).2 ∈ protectedKeys P S G H :=
      region_protected P (hc ▸ ha)
    have hm := (A.files f hprot).mp hf
    refine P.mem_aliasEdges.mpr ⟨f, P.mem_declaringOwners.mpr ⟨hm, ?_, hkind⟩, hc, s, hs, hk⟩
    unfold isOwner at ho ⊢
    rw [← owner_agree P hP A hprot]; exact ho
  exact ⟨step G H A, step H G (agreement_symm P A)⟩

theorem class_inside {S : List K} {G : Graph} (hc : Closed P S G) {k y : K}
    (hk : k ∈ S) (hr : P.aliasClass G k y) : y ∈ S := by
  induction hr with
  | refl => exact hk
  | fwd _ he ih => exact (hc _ he).mp ih
  | bwd _ he ih => exact (hc _ he).mpr ih

theorem class_agree (hP : P.Lawful) {S : List K} {G H : Graph}
    (A : Agreement P S G H) {k : K} (hk : k ∈ S) :
    P.aliasClass G k = P.aliasClass H k := by
  funext y; apply propext
  have step : ∀ (G H : Graph), Agreement P S G H →
      P.aliasClass G k y → P.aliasClass H k y := by
    intro G H A hr
    exact reach_transfer (A := fun x => x ∉ S)
      (fun a b ha he => edge_agree P hP A (by simpa using ha) |>.mp he)
      (fun z hz hn => hn (class_inside P A.closedG hk hz)) hr
  exact ⟨step G H A, step H G (agreement_symm P A)⟩

theorem declared_agree (hP : P.Lawful) {S : List K} {G H : Graph}
    (A : Agreement P S G H) {k : K} (hk : k ∈ S) :
    P.declaredKey G k = P.declaredKey H k := by
  apply Bool.eq_iff_iff.mpr
  rw [P.declaredKey_iff, P.declaredKey_iff]
  constructor <;> rintro ⟨e, he, heq⟩
  · have hs := (A.closedG e he).mpr (heq ▸ hk)
    exact ⟨e, (edge_agree P hP A hs).mp he, heq⟩
  · have hs := (A.closedH e he).mpr (heq ▸ hk)
    exact ⟨e, (edge_agree P hP A hs).mpr he, heq⟩

theorem refKeys_agree (hP : P.Lawful) {S : List K} {G H : Graph}
    (hwG : WF G) (hwH : WF H) (A : Agreement P S G H) {k : K} (hk : k ∈ S) :
    k ∈ P.refKeys G ↔ k ∈ P.refKeys H := by
  have step : ∀ (G H : Graph), WF G → WF H → Agreement P S G H →
      k ∈ P.refKeys G → k ∈ P.refKeys H := by
    intro G H hwG hwH A hr
    obtain ⟨b, hb, u, hu, he⟩ := P.mem_refKeys.mp hr
    obtain ⟨f, hf, ho, hp, hi⟩ := P.mem_blocks.mp hb
    obtain ⟨o, hoc, hou⟩ := P.mem_refNames.mp hu
    have hoc' : o ∈ P.occAt G (f.path, b.2) := by simpa [hp] using hoc
    obtain ⟨t, ht, hot⟩ := mem_occAt_outline hwG hf hoc'
    have hraw : k ∈ rawKeys P f := by
      simp only [rawKeys, List.mem_flatMap, List.mem_map]
      exact ⟨t, ht, o, hot, u, hou, he⟩
    have hprot := support_protected P (G := G) (H := H) (Or.inl hf) hk hraw
    have hfH := (A.files f hprot).mp hf
    have hoH : P.isOwner H f = true := by
      unfold isOwner at ho ⊢; rw [← owner_agree P hP A hprot]; exact ho
    have hbH : b ∈ P.blocks H := P.mem_blocks.mpr ⟨f, hfH, hoH, hp, hi⟩
    have heocc : P.occAt G b = P.occAt H b := by
      unfold occAt
      have h1 := fileAt_eq_of_mem hwG hf
      have h2 := fileAt_eq_of_mem hwH hfH
      rw [hp] at h1 h2
      rw [h1, h2]
    exact P.mem_refKeys.mpr ⟨b, hbH, u, P.mem_refNames.mpr ⟨o, heocc ▸ hoc, hou⟩, he⟩
  exact ⟨step G H hwG hwH A, step H G hwH hwG (agreement_symm P A)⟩

theorem nameKeys_agree {S : List K} {G H : Graph}
    (A : Agreement P S G H) {k : K} (hk : k ∈ S) :
    k ∈ P.nameKeys G ↔ k ∈ P.nameKeys H := by
  have step : ∀ (G H : Graph), Agreement P S G H →
      k ∈ P.nameKeys G → k ∈ P.nameKeys H := by
    intro G H A hn
    obtain ⟨f, hf, s, hs, he⟩ := P.mem_nameKeys.mp hn
    have hp := name_support_protected P (G := G) (H := H) (Or.inl hf) hk
      (List.mem_map.mpr ⟨s, hs, he⟩)
    exact P.mem_nameKeys.mpr ⟨f, (A.files f hp).mp hf, s, hs, he⟩
  exact ⟨step G H A, step H G (agreement_symm P A)⟩

theorem pageSeedKeys_agree (hP : P.Lawful) {S : List K} {G H : Graph}
    (hwG : WF G) (hwH : WF H) (A : Agreement P S G H) {k : K} (hk : k ∈ S) :
    k ∈ P.pageSeedKeys G ↔ k ∈ P.pageSeedKeys H := by
  simp only [pageSeedKeys, List.mem_append, refKeys_agree P hP hwG hwH A hk,
    nameKeys_agree P A hk]

theorem pages_agree (hP : P.Lawful) {S : List K} {G H : Graph}
    (hwG : WF G) (hwH : WF H) (A : Agreement P S G H) {p : Page K} (hp : p.2 ∈ S) :
    p ∈ Decided.pages P G ↔ p ∈ Decided.pages P H := by
  have hprot := region_protected P (G := G) (H := H) hp
  have hc : P.claimedKey G p.2 = P.claimedKey H p.2 := by
    simp only [claimedKey, real_agree P A (p := (.journal, p.2)) hprot,
      real_agree P A (p := (.page, p.2)) hprot]
  simp only [Decided.pages, P.mem_pagesR, real_agree P A hprot,
    pageSeedKeys_agree P hP hwG hwH A hp, hc, declared_agree P hP A hp]

theorem pageBlocks_agree (hP : P.Lawful) {S : List K} {G H : Graph}
    (A : Agreement P S G H) {p : Page K} (hp : p.2 ∈ S) :
    P.pageBlocks G p = P.pageBlocks H p := by
  unfold pageBlocks
  rw [owner_agree P hP A (region_protected P hp)]

theorem resolve_agree (hP : P.Lawful) {S : List K} {G H : Graph}
    (A : Agreement P S G H) {s : String} (hs : P.key s ∈ S) :
    P.resolve G s = P.resolve H s := by
  have hp := region_protected P (G := G) (H := H) hs
  have hd : ∀ p, p ∈ P.declarers G (P.key s) ↔ p ∈ P.declarers H (P.key s) := by
    intro p
    have step : ∀ (G H : Graph), Agreement P S G H →
        p ∈ P.declarers G (P.key s) → p ∈ P.declarers H (P.key s) := by
      intro G H A hm
      obtain ⟨f, hf, hk, hc⟩ := P.mem_declarers.mp hm
      obtain ⟨hmem, ho, hkind⟩ := P.mem_declaringOwners.mp hf
      obtain ⟨t, ht, he⟩ := List.any_eq_true.mp hk
      have he' : P.key t = P.key s := of_decide_eq_true he
      have hedge : ((P.claim f).2, P.key s) ∈ P.aliasEdges G :=
        P.mem_aliasEdges.mpr ⟨f, hf, rfl, t, ht, he'⟩
      have hkey := (A.closedG _ hedge).mpr hs
      have hfH := (A.files f (region_protected P hkey)).mp hmem
      have hoH : P.isOwner H f = true := by
        unfold isOwner at ho ⊢
        rw [← owner_agree P hP A (region_protected P hkey)]; exact ho
      exact P.mem_declarers.mpr ⟨f, P.mem_declaringOwners.mpr ⟨hfH, hoH, hkind⟩, hk, hc⟩
    exact ⟨step G H A, step H G (agreement_symm P A)⟩
  unfold Params.resolve resolveKey
  rw [solid_agree P A hp, solid_agree P A hp, real_agree P A hp, real_agree P A hp,
    minBy_congr hP.page hd]

theorem fileAt_pageBlock (hP : P.Lawful) {S : List K} {G H : Graph}
    (hwG : WF G) (hwH : WF H) (A : Agreement P S G H)
    {p : Page K} (hp : p.2 ∈ S) {b : BlockId} (hb : b ∈ P.pageBlocks G p) :
    fileAt G b.1 = fileAt H b.1 := by
  obtain ⟨f, ho, hpath, _⟩ := P.mem_pageBlocks.mp hb
  have hf := (P.owner_spec ho).1
  have hoH : P.owner H p = some f := by
    rw [← owner_agree P hP A (region_protected P hp)]; exact ho
  have hfH := (P.owner_spec hoH).1
  rw [hpath, fileAt_eq_of_mem hwG hf, fileAt_eq_of_mem hwH hfH]

theorem pathKeys_fileAt {L : Type} (Q : QParams K L) {G H : Graph} {b : BlockId}
    (hf : fileAt G b.1 = fileAt H b.1) :
    P.pathRefKeys Q G b = P.pathRefKeys Q H b := by
  simp [pathRefKeys, refNames, occAt, ancestors, pageOfBlock,
    List.flatMap_map, hf]

theorem references_agree (hP : P.Lawful) {S : List K} {G H : Graph}
    (hwG : WF G) (hwH : WF H) (A : Agreement P S G H)
    {p t : Page K} (hp : p.2 ∈ S) (ht : t.2 ∈ S)
    {b : BlockId} (hb : b ∈ P.pageBlocks G p) :
    P.References G b t ↔ P.References H b t := by
  have hf := fileAt_pageBlock P hP hwG hwH A hp hb
  have hn : P.refNames G b = P.refNames H b := by simp [refNames, occAt, hf]
  rw [P.references_iff_keys, P.references_iff_keys, hn]
  have hc : ∀ k, P.aliasClass G k t.2 ↔ P.aliasClass H k t.2 := by
    intro k
    constructor
    · intro h
      exact Reach.symm ((congrFun (class_agree P hP A ht) k).mp (Reach.symm h))
    · intro h
      exact Reach.symm ((congrFun (class_agree P hP A ht) k).mpr (Reach.symm h))
  simp only [hc]

theorem link_agree (hP : P.Lawful) {S : List K} {G H : Graph}
    (hwG : WF G) (hwH : WF H) (A : Agreement P S G H)
    {s t : Page K} (hs : s.2 ∈ S) (ht : t.2 ∈ S) (fav : Decided.Config) :
    Decided.Link P fav G s t ↔ Decided.Link P fav H s t := by
  have hb := pageBlocks_agree P hP A hs
  have hr : (∃ b ∈ P.pageBlocks G s, P.References G b t) ↔
      (∃ b ∈ P.pageBlocks H s, P.References H b t) := by
    constructor
    · rintro ⟨b, hm, hr⟩
      exact ⟨b, hb ▸ hm, (references_agree P hP hwG hwH A hs ht hm).mp hr⟩
    · rintro ⟨b, hm, hr⟩
      have hmG := hb ▸ hm
      exact ⟨b, hmG, (references_agree P hP hwG hwH A hs ht hmG).mpr hr⟩
  simp only [Decided.Link, Link, hr]
  rw [show s ∈ P.pagesR true G ↔ s ∈ P.pagesR true H from pages_agree P hP hwG hwH A hs,
    show t ∈ P.pagesR true G ↔ t ∈ P.pagesR true H from pages_agree P hP hwG hwH A ht]

/-- A query is scoped to S when every reference argument is in S. Literal names
do not depend on graph relations. page wrappers retain literal name semantics. -/
def Scoped {L : Type} (S : List K) : Leaf L → Prop
  | .refs s => P.key s ∈ S
  | .page l => Scoped S l
  | _ => True

/-- The unspecified leaves require an explicit locality law. Q1's original
typing as Graph → Node → Bool alone cannot exclude a graph-global leaf. -/
structure GivenAgreement {L : Type} (Q : QParams K L) (S : List K) (G H : Graph) : Prop where
  page : ∀ l p, p.2 ∈ S → Q.pageLeaf l G p = Q.pageLeaf l H p
  block : ∀ l p b, p.2 ∈ S → b ∈ P.pageBlocks G p →
    Q.blockLeaf l G b = Q.blockLeaf l H b

theorem leafPage_agree {L : Type} (Q : QParams K L) {S : List K} {G H : Graph}
    (Aq : GivenAgreement P Q S G H) (l : Leaf L) {p : Page K} (hp : p.2 ∈ S) :
    P.leafPage Q G l p = P.leafPage Q H l p := by
  induction l with
  | refs => rfl
  | name => rfl
  | page l ih => exact ih
  | given l => exact Aq.page l p hp

theorem leafBlock_agree {L : Type} (Q : QParams K L) (hP : P.Lawful)
    {S : List K} {G H : Graph} (hwG : WF G) (hwH : WF H)
    (A : Agreement P S G H) (Aq : GivenAgreement P Q S G H)
    {p : Page K} (hp : p.2 ∈ S) {b : BlockId} (hb : b ∈ P.pageBlocks G p)
    (l : Leaf L) (hl : Scoped P S l) :
    P.leafBlock3 Q G l b = P.leafBlock3 Q H l b := by
  have hf := fileAt_pageBlock P hP hwG hwH A hp hb
  cases l with
  | refs s =>
    simp only [leafBlock3, pathKeys_fileAt P Q hf]
    have he : (fun k => sameCls (P.aliasEdges G) (P.key s) k) =
        (fun k => sameCls (P.aliasEdges H) (P.key s) k) := by
      funext k
      apply Bool.eq_iff_iff.mpr
      rw [sameCls_iff, sameCls_iff]
      exact congrFun (class_agree P hP A hl) k |>.to_iff
    rw [he]
  | name => rfl
  | page l =>
    obtain ⟨f, ho, hpath, _⟩ := P.mem_pageBlocks.mp hb
    have hfg := (P.owner_spec ho).1
    have hclaim := (P.owner_spec ho).2
    have hpob : P.pageOfBlock G b = some p := by
      unfold pageOfBlock
      rw [hpath, fileAt_eq_of_mem hwG hfg]; simp [hclaim]
    have hpobH : P.pageOfBlock H b = some p := by
      unfold pageOfBlock at hpob ⊢; rw [← hf]; exact hpob
    simp only [leafBlock3, hpob, hpobH]
    exact leafPage_agree P Q Aq l hp
  | given l => exact Aq.block l p b hp hb

theorem query_bval_agree {L : Type} (q : Query L) (v w : Leaf L → Bool)
    (hl : ∀ l ∈ q.leaves, v l = w l) : q.bval v = q.bval w := by
  induction q with
  | leaf l => exact hl l (by simp [Query.leaves])
  | and q r ihq ihr | or q r ihq ihr =>
    simp only [Query.bval]
    rw [ihq (fun l hm => hl l (by simp [Query.leaves, hm])),
      ihr (fun l hm => hl l (by simp [Query.leaves, hm]))]
  | not q ih =>
    simp only [Query.bval]
    rw [ih (fun l hm => hl l (by simpa [Query.leaves] using hm))]

theorem query_page_agree {L : Type} (Q : QParams K L) (hP : P.Lawful)
    {S : List K} {G H : Graph} (hwG : WF G) (hwH : WF H)
    (A : Agreement P S G H) (Aq : GivenAgreement P Q S G H)
    (q : Query L) {p : Page K} (hp : p.2 ∈ S) :
    Node.page p ∈ Decided.den P Q G .page q ↔
      Node.page p ∈ Decided.den P Q H .page q := by
  have hev : P.evalNode3 Q G q (.page p) = P.evalNode3 Q H q (.page p) :=
    query_bval_agree q _ _ (fun l _ => leafPage_agree P Q Aq l hp)
  simp only [Decided.den, den3, P.mem_denU3 Q, univ, List.mem_map,
    Node.page.injEq, exists_eq_right, hev]
  rw [show p ∈ P.pagesR true G ↔ p ∈ P.pagesR true H from pages_agree P hP hwG hwH A hp]

theorem query_block_agree {L : Type} (Q : QParams K L) (hP : P.Lawful)
    {S : List K} {G H : Graph} (hwG : WF G) (hwH : WF H)
    (A : Agreement P S G H) (Aq : GivenAgreement P Q S G H)
    (q : Query L) (hq : ∀ l ∈ q.leaves, Scoped P S l)
    {p : Page K} (hp : p.2 ∈ S) {b : BlockId} (hb : b ∈ P.pageBlocks G p) :
    Node.block b ∈ Decided.den P Q G .block q ↔
      Node.block b ∈ Decided.den P Q H .block q := by
  have hbH : b ∈ P.pageBlocks H p := by rw [← pageBlocks_agree P hP A hp]; exact hb
  have hev : P.evalNode3 Q G q (.block b) = P.evalNode3 Q H q (.block b) :=
    query_bval_agree q _ _ (fun l hl => leafBlock_agree P Q hP hwG hwH A Aq hp hb l (hq l hl))
  simp only [Decided.den, den3, P.mem_denU3 Q, univ, List.mem_map,
    Node.block.injEq, exists_eq_right, hev]
  simp only [P.pageBlocks_sub_blocks hb, P.pageBlocks_sub_blocks hbH, true_and]

end Isolation
end CM
