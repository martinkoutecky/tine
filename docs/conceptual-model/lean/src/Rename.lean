/-
Rename (SPEC §5) and T3.

**What is given, what is defined.** As with the parser (`occ`, `title`, `aliasDecl`, `outline`), the
model does not transcribe the text rewriter. A `Rewriter` is given: which occurrence kinds it covers,
and a function rewriting one file under a *spelling map* `m : String → String`. Its laws (`RwLaws`) are
SPEC §5's bullet list read through the extractors: covered occurrences get the substituted spelling,
the other occurrences are left as they are, `alias::` members are rewritten (DESIGN Q3, Martin), the
moved files are rebound to their new names, and nothing else changes.

The model defines (executably):
* `nsMatch`, `nsSubst` — the spelling map of `rename old new`: a spelling whose namespace prefix (or
  itself) has `old`'s key gets that prefix replaced by `new` (`old/x` ↦ `new/x`).
* `renameAllowedLegacy` — SPEC §5's precondition (all three clauses).
* `litMapLegacy`, `renameLiteralLegacy` — SPEC §5's rewriting set read literally (refuted by C3).
* `renameBy R m G` (rewrite every file under `m`) and `rename R G old new = renameBy R (nsSubst …) G`.
* `blockMap` — the image `b'` of a block `b`.

T3 is proved for any spelling map `m` and key map `σ` satisfying the hypotheses `RenameHyps`, which
the witnesses check on concrete graphs for `m = nsSubst …`. The counterexamples (`Witnesses2.lean`)
show failure modes when freshness, namespace compatibility, the alias law, exception-freeness
or the Favorites condition is dropped; the bundle is sufficient, not shown minimal (order has no such example;
`Witnesses4.name_parent_kind_needed` covers a parent that changes kind).
-/
import Universe

set_option linter.unusedSectionVars false

namespace CM

/-! ## The spelling map of a rename -/

/-- The least number of leading `/`-segments of `s` whose nonempty spelling has `old`'s key, if any:
`some i` means the first `i + 1` segments name `old`. -/
def nsMatch {K : Type} [DecidableEq K] (key : String → K) (url : String → Bool) (old s : String) : Option Nat :=
  let parts := (Params.splitSlash s.toList).map String.ofList
  if isNsName url s then
    (List.range parts.length).find? (fun i => let partName := "/".intercalate (parts.take (i + 1));
      !partName.isEmpty && decide (key partName = key old))
  else if key s = key old then some 0 else none

/-- The spelling map of `rename old new` (SPEC §5): replace the namespace prefix naming `old` by
`new`; other spellings are unchanged. Round 10 (S15): a spelling that is not a namespace name
(`isNsName`: `./x`, `../x/y`, an origin-bearing URL) has no namespace prefixes, so it is in `old`'s
subtree only when it is `old` itself, and then it is replaced whole. -/
def nsSubst {K : Type} [DecidableEq K] (key : String → K) (url : String → Bool) (old new s : String) : String :=
  let parts := (Params.splitSlash s.toList).map String.ofList
  match nsMatch key url old s with
  | none => s
  | some i => if !isNsName url s || i + 1 = parts.length then new
      else new ++ "/" ++ "/".intercalate (parts.drop (i + 1))

/-- How a rewrite treats one occurrence: covered kinds get the substituted spelling. -/
def rwOcc (covers : OccKind → Bool) (m : String → String) (o : OccKind × String) : OccKind × String :=
  if covers o.1 then (o.1, m o.2) else o

theorem rwOcc_fst (c : OccKind → Bool) (m : String → String) (o : OccKind × String) :
    (rwOcc c m o).1 = o.1 := by
  unfold rwOcc; split <;> rfl

/-- The given text rewriter of a rename. `covers k` says whether occurrences of kind `k` are rewritten
(the generic occurrence coverage; the decided instance also covers property keys).
Stored queries have a separate round-6 extractor contract; block references remain id-based. `apply m f` is file `f` after the rewrite under spelling map
`m` (occurrences, `alias::` members, and, for a file claiming a moved page, its title or file name). -/
structure Rewriter where
  covers : OccKind → Bool
  apply : (String → String) → File → File

/-- Rewrite every file under the spelling map `m`. -/
def renameBy (R : Rewriter) (m : String → String) (G : Graph) : Graph := G.map (R.apply m)

namespace Params

/-- The image of block `b = (path, i)`: the same position in the rewritten file. -/
def blockMap (R : Rewriter) (m : String → String) (G : Graph) (b : BlockId) : BlockId :=
  match fileAt G b.1 with
  | some f => ((R.apply m f).path, b.2)
  | none => b

variable {K : Type} [DecidableEq K] (P : Params K)

/-- The pages a key map moves: `σp σ (κ, k) = (κ, σ k)`. -/
def σp (σ : K → K) (p : Page K) : Page K := (p.1, σ p.2)

/-- `rename G old new` (SPEC §5). Total; `renameAllowedLegacy` is SPEC's precondition. -/
def rename (R : Rewriter) (G : Graph) (old new : String) : Graph := renameBy R (nsSubst P.key P.urlHasOrigin old new) G

/-- The files `rename old new` moves: page-kind files whose name is in `old`'s namespace subtree
(journals never move). -/
def movedFiles (G : Graph) (old : String) : List File :=
  G.filter (fun f => decide (kindOf f = .page) && (nsMatch P.key P.urlHasOrigin old (P.name f)).isSome)

/-- SPEC §5's precondition: `old` is a real page; no real page outside `old`'s subtree claims `new`
or any moved name; and `new` is not a real member of `class old` (renaming onto a real alias is a
merge, which SPEC refuses). The last clause is not implied by the second: `a` declaring alias `a/b`
with `a/b` real passes the subtree clause (`Witnesses2.class_refusal`). -/
def renameAllowedLegacy (G : Graph) (old new : String) : Bool :=
  P.real G (.page, P.key old) &&
  G.all (fun g => (decide (kindOf g = .page) && (nsMatch P.key P.urlHasOrigin old (P.name g)).isSome) ||
    (P.movedFiles G old).all (fun f => decide ((P.claim g).2 ≠ P.key (nsSubst P.key P.urlHasOrigin old new (P.name f))))) &&
  !(P.claimedKey G (P.key new) && sameCls (P.aliasEdges G) (P.key old) (P.key new))

/-- SPEC §5's rewriting set read literally: a spelling is rewritten only when it names a moved (real)
page. Counterexample C3 shows this misses reference-only children; `nsSubst` is the repair. -/
def litMapLegacy (G : Graph) (old new : String) (s : String) : String :=
  if (P.movedFiles G old).any (fun f => decide (P.key (P.name f) = P.key s)) then nsSubst P.key P.urlHasOrigin old new s else s

/-- `rename` with SPEC §5's literal rewriting set. -/
def renameLiteralLegacy (R : Rewriter) (G : Graph) (old new : String) : Graph := renameBy R (P.litMapLegacy G old new) G

/-! ## Hypotheses of T3 -/

/-- The rewriter's laws on the files of `G` (SPEC §5 through the extractors), for spelling map `m`
and key map `σ`. -/
structure RwLaws (R : Rewriter) (m : String → String) (σ : K → K) (G : Graph) : Prop where
  dir : ∀ f ∈ G, (R.apply m f).dir = f.dir
  format : ∀ f ∈ G, (R.apply m f).format = f.format
  /-- The rewritten file claims the renamed page (title rebind or file rename). -/
  name : ∀ f ∈ G, P.key (P.name (R.apply m f)) = σ (P.key (P.name f))
  /-- A file whose page does not move keeps its path. -/
  path : ∀ f ∈ G, σ (P.claim f).2 = (P.claim f).2 → (R.apply m f).path = f.path
  /-- Same outline; each block's occurrences are rewritten by `rwOcc` (covered kinds get `m`). -/
  occ : ∀ f ∈ G, ((P.outline f.format f.text).map (P.occ f.format)).map (List.map (rwOcc R.covers m)) =
    (P.outline (R.apply m f).format (R.apply m f).text).map (P.occ (R.apply m f).format)
  /-- `alias::` members are rewritten (DESIGN Q3, Martin; Tine does not yet). -/
  alias : ∀ f ∈ G, P.aliasDecl (R.apply m f).format (R.apply m f).text = (P.aliasDecl f.format f.text).map m
  /-- The owner order among the claimants of one page is unchanged (open point O1: T3 needs this). -/
  order : ∀ f ∈ G, ∀ g ∈ G, P.claim f = P.claim g → P.fileLe (R.apply m f) (R.apply m g) = P.fileLe f g

/-- The key-level hypotheses of T3. -/
structure KeyHyps (R : Rewriter) (m : String → String) (σ : K → K) (G : Graph) : Prop where
  /-- `σ` is the key-level meaning of `m` on alias spellings. -/
  alias : ∀ f ∈ G, ∀ s ∈ P.aliasDecl f.format f.text, P.key (m s) = σ (P.key s)
  /-- Namespace closure commutes with the rewrite, up to keys, on covered occurrences: the keys of
  the closure of `m s` are exactly `σ` of the keys of the closure of `s`. (Fails when `old` and `new`
  have different namespace parents, and for SPEC §5's literal rewriting set: witnesses C2, C3.) -/
  ns : ∀ f ∈ G, ∀ t ∈ P.outline f.format f.text, ∀ o ∈ P.occ f.format t, R.covers o.1 = true →
    (∀ k ∈ (P.occClosure (m o.2)).map P.key, k ∈ (P.occClosure o.2).map (fun u => σ (P.key u))) ∧
    (∀ k ∈ (P.occClosure o.2).map (fun u => σ (P.key u)), k ∈ (P.occClosure (m o.2)).map P.key)
  /-- CM3-3: file-name parents, unlike occurrences, also create pages. This
  explicit extractor-level compatibility is needed only for preservation. -/
  names : ∀ f ∈ G,
    (∀ k ∈ (P.fileNsNames (R.apply m f)).map P.key,
      k ∈ (P.fileNsNames f).map (fun u => σ (P.key u))) ∧
    (∀ k ∈ (P.fileNsNames f).map (fun u => σ (P.key u)),
      k ∈ (P.fileNsNames (R.apply m f)).map P.key)
  /-- `σ` is injective on the keys in play: the new names are fresh (stronger than SPEC §5's
  precondition, which allows renaming onto a reference-only page: witness C1). -/
  inj : ∀ a ∈ P.inPlayKeys G, ∀ b ∈ P.inPlayKeys G, σ a = σ b → a = b
  /-- A moved reference-only page keeps its kind (O3). -/
  refKind : ∀ k ∈ P.pageSeedKeys G, P.refKind (σ k) = P.refKind k

/-- All hypotheses of T3, for `G' = renameBy R m G`. -/
structure RenameHyps (R : Rewriter) (m : String → String) (σ : K → K) (G : Graph) : Prop where
  wf : WF G
  wf' : WF (renameBy R m G)
  laws : P.RwLaws R m σ G
  keys : P.KeyHyps R m σ G

/-- No uncovered occurrence of block `b` names (through its namespace closure) a key that `σ` moves.
This is SPEC §7's "except for occurrences of the §5 exception kinds", made precise. -/
def ExceptionFree (R : Rewriter) (σ : K → K) (G : Graph) (b : BlockId) : Prop :=
  ∀ o ∈ P.occAt G b, R.covers o.1 = false → ∀ u ∈ P.occClosure o.2, σ (P.key u) = P.key u

/-- The image of a set of keys. -/
def img (σ : K → K) (C : K → Prop) : K → Prop := fun y => ∃ x, C x ∧ σ x = y

/-! ## Generic lemmas -/

end Params

theorem nodup_map_inj {α β : Type} {h : α → β} {l : List α} (hn : (l.map h).Nodup) {x y : α}
    (hx : x ∈ l) (hy : y ∈ l) (e : h x = h y) : x = y := by
  induction l with
  | nil => simp at hx
  | cons a t ih =>
    rw [List.map_cons, List.nodup_cons] at hn
    obtain ⟨hna, hnt⟩ := hn
    rcases List.mem_cons.mp hx with hx | hx <;> rcases List.mem_cons.mp hy with hy | hy
    · rw [hx, hy]
    · rw [hx] at e; exact absurd (by rw [e]; exact List.mem_map_of_mem hy) hna
    · rw [hy] at e; exact absurd (by rw [← e]; exact List.mem_map_of_mem hx) hna
    · exact ih hnt hx hy

theorem minBy_map {α β : Type} {le : α → α → Bool} {le' : β → β → Bool} {h : α → β} {l : List α}
    (hle : ∀ x ∈ l, ∀ y ∈ l, le' (h x) (h y) = le x y) : minBy le' (l.map h) = (minBy le l).map h := by
  induction l with
  | nil => rfl
  | cons a t ih =>
    have ih' := ih (fun x hx y hy => hle x (List.mem_cons_of_mem _ hx) y (List.mem_cons_of_mem _ hy))
    simp only [List.map_cons, minBy, ih']
    cases hm : minBy le t with
    | none => rfl
    | some m0 =>
      simp only [Option.map_some]
      rw [hle a (List.mem_cons_self ..) m0 (List.mem_cons_of_mem _ (minBy_mem hm))]
      split <;> rfl

namespace Reach

variable {α β : Type} {E : List (α × α)}

/-- Image of an edge list under a map. -/
def mapE (σ : α → β) (E : List (α × α)) : List (β × β) := E.map (fun e => (σ e.1, σ e.2))

theorem map_fwd (σ : α → β) {a b : α} (h : Reach E a b) : Reach (mapE σ E) (σ a) (σ b) := by
  induction h with
  | refl => exact refl _
  | fwd _ e ih => exact fwd ih (List.mem_map_of_mem (f := fun e => (σ e.1, σ e.2)) e)
  | bwd _ e ih => exact bwd ih (List.mem_map_of_mem (f := fun e => (σ e.1, σ e.2)) e)

/-- A walk in the image graph from `σ a` lifts to a walk from `a`, when `σ` is injective on a set `S`
containing `a` and every edge end. -/
theorem map_bwd (σ : α → β) (S : α → Prop) (hS : ∀ e ∈ E, S e.1 ∧ S e.2)
    (hinj : ∀ x y, S x → S y → σ x = σ y → x = y) {a : α} (ha : S a) {y : β}
    (h : Reach (mapE σ E) (σ a) y) : ∃ b, S b ∧ Reach E a b ∧ σ b = y := by
  induction h with
  | refl => exact ⟨a, ha, refl _, rfl⟩
  | @fwd u c _ e ih =>
    obtain ⟨b, hb, r, eb⟩ := ih
    obtain ⟨e0, he0, ee⟩ := List.mem_map.mp e
    simp only [Prod.mk.injEq] at ee
    have : b = e0.1 := hinj _ _ hb (hS e0 he0).1 (eb.trans ee.1.symm)
    subst this
    exact ⟨e0.2, (hS e0 he0).2, fwd r he0, ee.2⟩
  | @bwd u c _ e ih =>
    obtain ⟨b, hb, r, eb⟩ := ih
    obtain ⟨e0, he0, ee⟩ := List.mem_map.mp e
    simp only [Prod.mk.injEq] at ee
    have : b = e0.2 := hinj _ _ hb (hS e0 he0).2 (eb.trans ee.2.symm)
    subst this
    exact ⟨e0.1, (hS e0 he0).1, bwd r he0, ee.1⟩

end Reach

namespace Params

variable {K : Type} [DecidableEq K] {P : Params K}
variable {R : Rewriter} {m : String → String} {σ : K → K} {G : Graph}

/-! ## Files, owners, blocks under the rewrite -/

theorem inPlay_inj (H : P.KeyHyps R m σ G) {a b : K} (ha : P.InPlay G a) (hb : P.InPlay G b)
    (e : σ a = σ b) : a = b :=
  H.inj a ((P.inPlay_iff).mp ha) b ((P.inPlay_iff).mp hb) e

theorem claim_apply (hL : P.RwLaws R m σ G) {f : File} (hf : f ∈ G) :
    P.claim (R.apply m f) = σp σ (P.claim f) := by
  unfold claim σp
  have hk : kindOf (R.apply m f) = kindOf f := by unfold kindOf; rw [hL.dir f hf]
  rw [hk, hL.name f hf]

theorem apply_inj (hw' : WF (renameBy R m G)) {f g : File} (hf : f ∈ G) (hg : g ∈ G)
    (e : R.apply m f = R.apply m g) : f = g := by
  unfold WF renameBy at hw'
  rw [List.map_map] at hw'
  exact nodup_map_inj hw' hf hg (by simp [e])

theorem σp_inj (H : P.KeyHyps R m σ G) {p q : Page K} (hp : P.InPlay G p.2) (hq : P.InPlay G q.2)
    (e : σp σ p = σp σ q) : p = q := by
  obtain ⟨κ, a⟩ := p; obtain ⟨l, b⟩ := q
  simp only [σp, Prod.mk.injEq] at e
  simp only at hp hq
  rw [e.1, inPlay_inj H hp hq e.2]

theorem claimants_rename (H : P.RenameHyps R m σ G) {p : Page K} (hp : P.InPlay G p.2) :
    P.claimants (renameBy R m G) (σp σ p) = (P.claimants G p).map (R.apply m) := by
  unfold claimants renameBy
  rw [List.filter_map]
  congr 1
  apply List.filter_congr
  intro f hf
  simp only [Function.comp, claim_apply H.laws hf]
  apply decide_eq_decide.mpr
  constructor
  · intro e; exact σp_inj H.keys (P.inPlay_claim hf) hp e
  · intro e; rw [e]

theorem owner_rename (H : P.RenameHyps R m σ G) {p : Page K} (hp : P.InPlay G p.2) :
    P.owner (renameBy R m G) (σp σ p) = (P.owner G p).map (R.apply m) := by
  unfold owner
  rw [claimants_rename H hp]
  apply minBy_map
  intro x hx y hy
  rw [P.mem_claimants] at hx hy
  exact H.laws.order x hx.1 y hy.1 (hx.2.trans hy.2.symm)

theorem isOwner_rename (H : P.RenameHyps R m σ G) {f : File} (hf : f ∈ G) :
    P.isOwner (renameBy R m G) (R.apply m f) = P.isOwner G f := by
  unfold isOwner
  rw [claim_apply H.laws hf, owner_rename H (P.inPlay_claim hf)]
  apply decide_eq_decide.mpr
  constructor
  · intro e
    obtain ⟨g, hg, eg⟩ := Option.map_eq_some_iff.mp e
    have hgG := (P.owner_spec hg).1
    rw [hg, apply_inj H.wf' hgG hf eg]
  · intro e; rw [e]; rfl

theorem mem_renameBy {f' : File} : f' ∈ renameBy R m G ↔ ∃ f ∈ G, R.apply m f = f' := by
  simp [renameBy]

theorem declaringOwners_rename (H : P.RenameHyps R m σ G) :
    P.declaringOwners (renameBy R m G) = (P.declaringOwners G).map (R.apply m) := by
  unfold declaringOwners
  unfold renameBy
  rw [List.filter_map]
  congr 1
  apply List.filter_congr
  intro f hf
  have hk : kindOf (R.apply m f) = kindOf f := by unfold kindOf; rw [H.laws.dir f hf]
  simp only [Function.comp]
  rw [← renameBy, isOwner_rename H hf, hk]

theorem aliasEdges_rename (H : P.RenameHyps R m σ G) (e : K × K) :
    e ∈ P.aliasEdges (renameBy R m G) ↔ e ∈ Reach.mapE σ (P.aliasEdges G) := by
  obtain ⟨a, b⟩ := e
  rw [P.mem_aliasEdges, declaringOwners_rename H]
  unfold Reach.mapE
  rw [List.mem_map]
  constructor
  · rintro ⟨f', hf', ha, s', hs', hb⟩
    obtain ⟨f, hf, rfl⟩ := List.mem_map.mp hf'
    have hfG := ((P.mem_declaringOwners).mp hf).1
    rw [H.laws.alias f hfG] at hs'
    obtain ⟨s, hs, rfl⟩ := List.mem_map.mp hs'
    refine ⟨((P.claim f).2, P.key s), (P.mem_aliasEdges).mpr ⟨f, hf, rfl, s, hs, rfl⟩, ?_⟩
    rw [claim_apply H.laws hfG] at ha
    rw [← ha, ← hb, H.keys.alias f hfG s hs]; rfl
  · rintro ⟨⟨a0, b0⟩, he, ee⟩
    obtain ⟨f, hf, ha, s, hs, hb⟩ := (P.mem_aliasEdges).mp he
    have hfG := ((P.mem_declaringOwners).mp hf).1
    simp only [Prod.mk.injEq] at ee
    refine ⟨R.apply m f, List.mem_map_of_mem hf, ?_, m s, ?_, ?_⟩
    · rw [claim_apply H.laws hfG]; simp only [σp]; rw [ha]; exact ee.1
    · rw [H.laws.alias f hfG]; exact List.mem_map_of_mem hs
    · rw [H.keys.alias f hfG s hs, hb]; exact ee.2

/-- Alias classes are carried by `σ`: for keys in play, `σ a ~ σ b` after iff `a ~ b` before. -/
theorem aliasClass_rename (H : P.RenameHyps R m σ G) {a b : K} (ha : P.InPlay G a) (hb : P.InPlay G b) :
    P.aliasClass (renameBy R m G) (σ a) (σ b) ↔ P.aliasClass G a b := by
  unfold aliasClass cls
  rw [Reach.congr (aliasEdges_rename H)]
  constructor
  · intro h
    have hS : ∀ e ∈ P.aliasEdges G, P.InPlay G e.1 ∧ P.InPlay G e.2 := fun e he =>
      ⟨Or.inr (Or.inr ⟨e, he, Or.inl rfl⟩), Or.inr (Or.inr ⟨e, he, Or.inr rfl⟩)⟩
    obtain ⟨c, hc, r, ec⟩ := Reach.map_bwd σ (P.InPlay G) hS (fun x y hx hy => inPlay_inj H.keys hx hy) ha h
    rw [inPlay_inj H.keys hc hb ec] at r; exact r
  · exact Reach.map_fwd σ

/-- The class of `σ a` after is the image of the class of `a` before. -/
theorem aliasClass_rename_img (H : P.RenameHyps R m σ G) {a : K} (ha : P.InPlay G a) :
    P.aliasClass (renameBy R m G) (σ a) = img σ (P.aliasClass G a) := by
  funext y; apply propext
  unfold aliasClass cls img
  rw [Reach.congr (aliasEdges_rename H)]
  constructor
  · intro h
    have hS : ∀ e ∈ P.aliasEdges G, P.InPlay G e.1 ∧ P.InPlay G e.2 := fun e he =>
      ⟨Or.inr (Or.inr ⟨e, he, Or.inl rfl⟩), Or.inr (Or.inr ⟨e, he, Or.inr rfl⟩)⟩
    obtain ⟨c, _, r, ec⟩ := Reach.map_bwd σ (P.InPlay G) hS (fun x y hx hy => inPlay_inj H.keys hx hy) ha h
    exact ⟨c, r, ec⟩
  · rintro ⟨c, r, rfl⟩; exact Reach.map_fwd σ r

theorem occAt_some {G : Graph} {f : File} {i : Nat} (hw : WF G) (hf : f ∈ G) :
    P.occAt G (f.path, i) = (((P.outline f.format f.text)[i]?).map (P.occ f.format)).getD [] := by
  unfold occAt
  rw [fileAt_eq_of_mem hw hf]
  simp only
  cases (P.outline f.format f.text)[i]? <;> rfl

theorem occAt_rename (H : P.RenameHyps R m σ G) {f : File} (hf : f ∈ G) (i : Nat) :
    P.occAt (renameBy R m G) ((R.apply m f).path, i) =
      (P.occAt G (f.path, i)).map (rwOcc R.covers m) := by
  rw [occAt_some H.wf hf, occAt_some H.wf' (List.mem_map_of_mem hf)]
  have h := congrArg (fun l => l[i]?) (H.laws.occ f hf)
  simp only [List.getElem?_map] at h
  rw [← h]
  cases (P.outline f.format f.text)[i]? <;> rfl

theorem outline_length_rename (hL : P.RwLaws R m σ G) {f : File} (hf : f ∈ G) :
    (P.outline (R.apply m f).format (R.apply m f).text).length = (P.outline f.format f.text).length := by
  have h := congrArg List.length (hL.occ f hf)
  simp only [List.length_map] at h
  exact h.symm

theorem blockMap_eq (hw : WF G) {f : File} (hf : f ∈ G) (i : Nat) :
    blockMap R m G (f.path, i) = ((R.apply m f).path, i) := by
  unfold blockMap; rw [fileAt_eq_of_mem hw hf]

/-- Blocks of the renamed graph are exactly the images of the blocks of `G`. -/
theorem blocks_rename (H : P.RenameHyps R m σ G) (b' : BlockId) :
    b' ∈ P.blocks (renameBy R m G) ↔ ∃ b ∈ P.blocks G, blockMap R m G b = b' := by
  rw [P.mem_blocks]
  constructor
  · rintro ⟨f', hf', ho, ep, hi⟩
    obtain ⟨f, hf, rfl⟩ := mem_renameBy.mp hf'
    refine ⟨(f.path, b'.2), (P.mem_blocks).mpr ⟨f, hf, ?_, rfl, ?_⟩, ?_⟩
    · rw [← isOwner_rename H hf]; exact ho
    · rw [← outline_length_rename H.laws hf]; exact hi
    · rw [blockMap_eq H.wf hf, ep]
  · rintro ⟨b, hb, rfl⟩
    obtain ⟨f, hf, ho, ep, hi⟩ := (P.mem_blocks).mp hb
    obtain ⟨π, i⟩ := b
    simp only at ep hi; subst ep
    rw [blockMap_eq H.wf hf]
    exact ⟨R.apply m f, List.mem_map_of_mem hf, by rw [isOwner_rename H hf]; exact ho, rfl,
      by rw [outline_length_rename H.laws hf]; exact hi⟩

/-- `blockMap` is injective on the blocks of `G` (so it is a bijection onto the renamed blocks). -/
theorem blockMap_inj (H : P.RenameHyps R m σ G) {b c : BlockId} (hb : b ∈ P.blocks G) (hc : c ∈ P.blocks G)
    (e : blockMap R m G b = blockMap R m G c) : b = c := by
  obtain ⟨f, hf, _, ef, _⟩ := (P.mem_blocks).mp hb
  obtain ⟨g, hg, _, eg, _⟩ := (P.mem_blocks).mp hc
  obtain ⟨π, i⟩ := b; obtain ⟨ρ, j⟩ := c
  simp only at ef eg; subst ef; subst eg
  rw [blockMap_eq H.wf hf, blockMap_eq H.wf hg] at e
  simp only [Prod.mk.injEq] at e
  have : R.apply m f = R.apply m g :=
    eq_of_path_eq H.wf' (List.mem_map_of_mem hf) (List.mem_map_of_mem hg) e.1
  rw [apply_inj H.wf' hf hg this, e.2]

/-! ## References under the rewrite -/

theorem mem_occAt_outline {G : Graph} {f : File} {i : Nat} {o : OccKind × String} (hw : WF G) (hf : f ∈ G)
    (ho : o ∈ P.occAt G (f.path, i)) : ∃ t ∈ P.outline f.format f.text, o ∈ P.occ f.format t := by
  rw [occAt_some hw hf] at ho
  cases ht : (P.outline f.format f.text)[i]? with
  | none => rw [ht] at ho; simp at ho
  | some t =>
    rw [ht] at ho
    exact ⟨t, List.mem_of_getElem? ht, ho⟩

theorem inPlay_refName {G : Graph} {b : BlockId} (hb : b ∈ P.blocks G) {s : String}
    (hs : s ∈ P.refNames G b) : P.InPlay G (P.key s) :=
  Or.inr (Or.inl (List.mem_append_left _ ((P.mem_refKeys).mpr ⟨b, hb, s, hs, rfl⟩)))

/-- Covered occurrences: the closure keys of the image block are `σ` of those of the block. -/
theorem covered_keys_rename (H : P.RenameHyps R m σ G) {f : File} (hf : f ∈ G) (i : Nat) (k' : K) :
    (∃ o' ∈ P.occAt (renameBy R m G) ((R.apply m f).path, i), R.covers o'.1 = true ∧
        ∃ u' ∈ P.occClosure o'.2, P.key u' = k') ↔
      (∃ o ∈ P.occAt G (f.path, i), R.covers o.1 = true ∧ ∃ u ∈ P.occClosure o.2, σ (P.key u) = k') := by
  rw [occAt_rename H hf]
  constructor
  · rintro ⟨o', ho', hc, u', hu', ek⟩
    obtain ⟨o, ho, rfl⟩ := List.mem_map.mp ho'
    rw [rwOcc_fst] at hc
    have hrw : rwOcc R.covers m o = (o.1, m o.2) := by unfold rwOcc; rw [if_pos hc]
    rw [hrw] at hu'
    obtain ⟨t, ht, hot⟩ := mem_occAt_outline H.wf hf ho
    have := (H.keys.ns f hf t ht o hot hc).1 (P.key u') (List.mem_map_of_mem hu')
    obtain ⟨u, hu, eu⟩ := List.mem_map.mp this
    exact ⟨o, ho, hc, u, hu, eu.trans ek⟩
  · rintro ⟨o, ho, hc, u, hu, ek⟩
    obtain ⟨t, ht, hot⟩ := mem_occAt_outline H.wf hf ho
    have := (H.keys.ns f hf t ht o hot hc).2 (σ (P.key u)) (List.mem_map_of_mem (f := fun u => σ (P.key u)) hu)
    obtain ⟨u', hu', eu⟩ := List.mem_map.mp this
    have hrw : rwOcc R.covers m o = (o.1, m o.2) := by unfold rwOcc; rw [if_pos hc]
    refine ⟨rwOcc R.covers m o, List.mem_map_of_mem ho, by rw [rwOcc_fst]; exact hc, u', ?_, eu.trans ek⟩
    rw [hrw]; exact hu'

/-- Uncovered occurrences of an exception-free block: unchanged, with keys fixed by `σ`. -/
theorem uncovered_keys_rename (H : P.RenameHyps R m σ G) {f : File} (hf : f ∈ G) (i : Nat)
    (hx : P.ExceptionFree R σ G (f.path, i)) (k' : K) :
    (∃ o' ∈ P.occAt (renameBy R m G) ((R.apply m f).path, i), R.covers o'.1 = false ∧
        ∃ u' ∈ P.occClosure o'.2, P.key u' = k') ↔
      (∃ o ∈ P.occAt G (f.path, i), R.covers o.1 = false ∧ ∃ u ∈ P.occClosure o.2, σ (P.key u) = k') := by
  rw [occAt_rename H hf]
  have hrw : ∀ o : OccKind × String, R.covers o.1 = false → rwOcc R.covers m o = o := fun o hc => by
    unfold rwOcc; rw [if_neg (by simp [hc])]
  constructor
  · rintro ⟨o', ho', hc, u', hu', ek⟩
    obtain ⟨o, ho, rfl⟩ := List.mem_map.mp ho'
    rw [rwOcc_fst] at hc
    rw [hrw o hc] at hu'
    exact ⟨o, ho, hc, u', hu', by rw [hx o ho hc u' hu']; exact ek⟩
  · rintro ⟨o, ho, hc, u, hu, ek⟩
    refine ⟨rwOcc R.covers m o, List.mem_map_of_mem ho, by rw [rwOcc_fst]; exact hc, u, ?_, ?_⟩
    · rw [hrw o hc]; exact hu
    · rw [← hx o ho hc u hu]; exact ek

/-- All closure keys of an exception-free block are carried by `σ`. -/
theorem refName_keys_rename (H : P.RenameHyps R m σ G) {f : File} (hf : f ∈ G) (i : Nat)
    (hx : P.ExceptionFree R σ G (f.path, i)) (k' : K) :
    (∃ s' ∈ P.refNames (renameBy R m G) ((R.apply m f).path, i), P.key s' = k') ↔
      (∃ s ∈ P.refNames G (f.path, i), σ (P.key s) = k') := by
  have e1 : ∀ (G0 : Graph) (b : BlockId) (g : String → K),
      (∃ s ∈ P.refNames G0 b, g s = k') ↔
        (∃ o ∈ P.occAt G0 b, R.covers o.1 = true ∧ ∃ u ∈ P.occClosure o.2, g u = k') ∨
        (∃ o ∈ P.occAt G0 b, R.covers o.1 = false ∧ ∃ u ∈ P.occClosure o.2, g u = k') := by
    intro G0 b g
    simp only [P.mem_refNames]
    constructor
    · rintro ⟨s, ⟨o, ho, hs⟩, e⟩
      cases hc : R.covers o.1
      · exact Or.inr ⟨o, ho, hc, s, hs, e⟩
      · exact Or.inl ⟨o, ho, hc, s, hs, e⟩
    · rintro (⟨o, ho, _, u, hu, e⟩ | ⟨o, ho, _, u, hu, e⟩) <;> exact ⟨u, ⟨o, ho, hu⟩, e⟩
  rw [e1, e1 G (f.path, i) (fun s => σ (P.key s)), covered_keys_rename H hf i k',
    uncovered_keys_rename H hf i hx k']

/-- **T3 core, exception-filtered.** Through covered occurrences only, a block of `G` references `t`
iff its image references `σp t` (no exception-freeness needed). -/
theorem refsBy_covers_rename (H : P.RenameHyps R m σ G) {b : BlockId} (hb : b ∈ P.blocks G)
    {t : Page K} (ht : P.InPlay G t.2) :
    P.RefsBy R.covers G b t ↔ P.RefsBy R.covers (renameBy R m G) (blockMap R m G b) (σp σ t) := by
  obtain ⟨f, hf, _, ep, _⟩ := (P.mem_blocks).mp hb
  obtain ⟨π, i⟩ := b; simp only at ep; subst ep
  rw [blockMap_eq H.wf hf]
  unfold RefsBy
  have hinP : ∀ o ∈ P.occAt G (f.path, i), ∀ u ∈ P.occClosure o.2, P.InPlay G (P.key u) := fun o ho u hu =>
    inPlay_refName hb ((P.mem_refNames).mpr ⟨o, ho, hu⟩)
  constructor
  · rintro ⟨o, ho, hc, u, hu, hcl⟩
    obtain ⟨o', ho', hc', u', hu', ek⟩ := (covered_keys_rename H hf i (σ (P.key u))).mpr ⟨o, ho, hc, u, hu, rfl⟩
    refine ⟨o', ho', hc', u', hu', ?_⟩
    rw [ek]; exact (aliasClass_rename H (hinP o ho u hu) ht).mpr hcl
  · rintro ⟨o', ho', hc', u', hu', hcl⟩
    obtain ⟨o, ho, hc, u, hu, ek⟩ := (covered_keys_rename H hf i (P.key u')).mp ⟨o', ho', hc', u', hu', rfl⟩
    refine ⟨o, ho, hc, u, hu, ?_⟩
    rw [← ek] at hcl; exact (aliasClass_rename H (hinP o ho u hu) ht).mp hcl

/-- **T3 core.** For an exception-free block, `b` references `t` iff `b'` references `σp t`. -/
theorem references_rename (H : P.RenameHyps R m σ G) {b : BlockId} (hb : b ∈ P.blocks G)
    (hx : P.ExceptionFree R σ G b) {t : Page K} (ht : P.InPlay G t.2) :
    P.References G b t ↔ P.References (renameBy R m G) (blockMap R m G b) (σp σ t) := by
  obtain ⟨f, hf, _, ep, _⟩ := (P.mem_blocks).mp hb
  obtain ⟨π, i⟩ := b; simp only at ep; subst ep
  rw [blockMap_eq H.wf hf, P.references_iff_keys, P.references_iff_keys]
  constructor
  · rintro ⟨s, hs, hcl⟩
    obtain ⟨s', hs', ek⟩ := (refName_keys_rename H hf i hx (σ (P.key s))).mpr ⟨s, hs, rfl⟩
    refine ⟨s', hs', ?_⟩
    rw [ek]; exact (aliasClass_rename H (inPlay_refName hb hs) ht).mpr hcl
  · rintro ⟨s', hs', hcl⟩
    obtain ⟨s, hs, ek⟩ := (refName_keys_rename H hf i hx (P.key s')).mp ⟨s', hs', rfl⟩
    refine ⟨s, hs, ?_⟩
    rw [← ek] at hcl; exact (aliasClass_rename H (inPlay_refName hb hs) ht).mp hcl

/-- **T3 core, SPEC's literal form** `refs b' = σ(refs b)`: the concepts the image block references
are exactly the images (under `σ`) of the concepts `b` references. -/
theorem refs_rename (H : P.RenameHyps R m σ G) {b : BlockId} (hb : b ∈ P.blocks G)
    (hx : P.ExceptionFree R σ G b) :
    P.refs (renameBy R m G) (blockMap R m G b) = fun C => ∃ C₀, P.refs G b C₀ ∧ C = img σ C₀ := by
  obtain ⟨f, hf, _, ep, _⟩ := (P.mem_blocks).mp hb
  obtain ⟨π, i⟩ := b; simp only at ep; subst ep
  funext C; apply propext
  rw [blockMap_eq H.wf hf]
  unfold refs
  simp only [P.concept_eq_class]
  constructor
  · rintro ⟨s', hs', rfl⟩
    obtain ⟨s, hs, ek⟩ := (refName_keys_rename H hf i hx (P.key s')).mp ⟨s', hs', rfl⟩
    refine ⟨_, ⟨s, hs, rfl⟩, ?_⟩
    rw [← ek, aliasClass_rename_img H (inPlay_refName hb hs)]
  · rintro ⟨_, ⟨s, hs, rfl⟩, rfl⟩
    obtain ⟨s', hs', ek⟩ := (refName_keys_rename H hf i hx (σ (P.key s))).mpr ⟨s, hs, rfl⟩
    refine ⟨s', hs', ?_⟩
    rw [ek, aliasClass_rename_img H (inPlay_refName hb hs)]

/-! ## Pages under the rewrite -/

theorem claimedKey_rename (H : P.RenameHyps R m σ G) (k' : K) :
    P.claimedKey (renameBy R m G) k' = true ↔ ∃ k, P.claimedKey G k = true ∧ σ k = k' := by
  rw [P.claimedKey_iff]
  constructor
  · rintro ⟨f', hf', e⟩
    obtain ⟨f, hf, rfl⟩ := mem_renameBy.mp hf'
    rw [claim_apply H.laws hf] at e
    exact ⟨(P.claim f).2, (P.claimedKey_iff).mpr ⟨f, hf, rfl⟩, e⟩
  · rintro ⟨k, hk, rfl⟩
    obtain ⟨f, hf, e⟩ := (P.claimedKey_iff).mp hk
    exact ⟨R.apply m f, List.mem_map_of_mem hf, by rw [claim_apply H.laws hf]; simp [σp, e]⟩

theorem claimedKey_rename_σ (H : P.RenameHyps R m σ G) {k : K} (hk : P.InPlay G k) :
    P.claimedKey (renameBy R m G) (σ k) = P.claimedKey G k := by
  apply Bool.eq_iff_iff.mpr
  rw [claimedKey_rename H]
  constructor
  · rintro ⟨k0, h0, e⟩
    rw [inPlay_inj H.keys (Or.inl h0) hk e] at h0; exact h0
  · intro h; exact ⟨k, h, rfl⟩

theorem real_rename (H : P.RenameHyps R m σ G) (q : Page K) :
    P.real (renameBy R m G) q = true ↔ ∃ p, P.real G p = true ∧ σp σ p = q := by
  rw [P.real_iff]
  constructor
  · rintro ⟨f', hf', e⟩
    obtain ⟨f, hf, rfl⟩ := mem_renameBy.mp hf'
    rw [claim_apply H.laws hf] at e
    exact ⟨P.claim f, (P.real_iff).mpr ⟨f, hf, rfl⟩, e⟩
  · rintro ⟨p, hp, rfl⟩
    obtain ⟨f, hf, rfl⟩ := (P.real_iff).mp hp
    exact ⟨R.apply m f, List.mem_map_of_mem hf, claim_apply H.laws hf⟩

/-- The whole graph is exception-free. -/
def GraphExceptionFree (R : Rewriter) (σ : K → K) (G : Graph) : Prop :=
  ∀ b ∈ P.blocks G, P.ExceptionFree R σ G b

theorem refKeys_rename (H : P.RenameHyps R m σ G) (hx : P.GraphExceptionFree R σ G) (k' : K) :
    k' ∈ P.refKeys (renameBy R m G) ↔ ∃ k ∈ P.refKeys G, σ k = k' := by
  rw [P.mem_refKeys]
  constructor
  · rintro ⟨b', hb', s', hs', e⟩
    obtain ⟨b, hb, rfl⟩ := (blocks_rename H b').mp hb'
    obtain ⟨f, hf, _, ep, _⟩ := (P.mem_blocks).mp hb
    obtain ⟨π, i⟩ := b; simp only at ep; subst ep
    rw [blockMap_eq H.wf hf] at hs'
    obtain ⟨s, hs, ek⟩ := (refName_keys_rename H hf i (hx _ hb) k').mp ⟨s', hs', e⟩
    exact ⟨P.key s, (P.mem_refKeys).mpr ⟨_, hb, s, hs, rfl⟩, ek⟩
  · rintro ⟨k, hk, rfl⟩
    obtain ⟨b, hb, s, hs, rfl⟩ := (P.mem_refKeys).mp hk
    obtain ⟨f, hf, _, ep, _⟩ := (P.mem_blocks).mp hb
    obtain ⟨π, i⟩ := b; simp only at ep; subst ep
    obtain ⟨s', hs', ek⟩ := (refName_keys_rename H hf i (hx _ hb) (σ (P.key s))).mpr ⟨s, hs, rfl⟩
    refine ⟨_, (blocks_rename H _).mpr ⟨_, hb, rfl⟩, s', ?_, ek⟩
    rw [blockMap_eq H.wf hf]; exact hs'

theorem nameKeys_rename (H : P.RenameHyps R m σ G) (k : K) :
    k ∈ P.nameKeys (renameBy R m G) ↔ ∃ u ∈ P.nameKeys G, σ u = k := by
  simp only [nameKeys, renameBy, List.flatMap_map, List.mem_flatMap]
  constructor
  · rintro ⟨f, hf, hk⟩
    obtain ⟨u, hu, he⟩ := List.mem_map.mp ((H.keys.names f hf).1 k hk)
    exact ⟨P.key u, ⟨f, hf, List.mem_map_of_mem hu⟩, he⟩
  · rintro ⟨u, ⟨f, hf, hu⟩, rfl⟩
    obtain ⟨s, hs, rfl⟩ := List.mem_map.mp hu
    exact ⟨f, hf, (H.keys.names f hf).2 _ (List.mem_map.mpr ⟨s, hs, rfl⟩)⟩

theorem pageSeedKeys_rename (H : P.RenameHyps R m σ G)
    (hx : P.GraphExceptionFree R σ G) (k : K) :
    k ∈ P.pageSeedKeys (renameBy R m G) ↔ ∃ u ∈ P.pageSeedKeys G, σ u = k := by
  simp only [pageSeedKeys, List.mem_append, refKeys_rename H hx, nameKeys_rename H]
  constructor
  · rintro (⟨u, hu, he⟩ | ⟨u, hu, he⟩)
    · exact ⟨u, Or.inl hu, he⟩
    · exact ⟨u, Or.inr hu, he⟩
  · rintro ⟨u, hu | hu, he⟩
    · exact Or.inl ⟨u, hu, he⟩
    · exact Or.inr ⟨u, hu, he⟩

theorem declaredKey_rename_σ (H : P.RenameHyps R m σ G) {k : K} (hk : P.InPlay G k) :
    P.declaredKey (renameBy R m G) (σ k) = P.declaredKey G k := by
  apply Bool.eq_iff_iff.mpr
  rw [P.declaredKey_iff, P.declaredKey_iff]
  constructor
  · rintro ⟨e', he', ee⟩
    obtain ⟨e, he, rfl⟩ := List.mem_map.mp ((aliasEdges_rename H e').mp he')
    exact ⟨e, he, inPlay_inj H.keys (Or.inr (Or.inr ⟨e, he, Or.inr rfl⟩)) hk ee⟩
  · rintro ⟨e, he, rfl⟩
    exact ⟨(σ e.1, σ e.2), (aliasEdges_rename H _).mpr (List.mem_map_of_mem he), rfl⟩

/-- **T3 (pages), forward.** `σp` maps the page universe of `G` into that of `G'` (either reading of P1). -/
theorem pagesR_rename_fwd (H : P.RenameHyps R m σ G) (hx : P.GraphExceptionFree R σ G) {r : Bool}
    {p : Page K} (hp : p ∈ P.pagesR r G) : σp σ p ∈ P.pagesR r (renameBy R m G) := by
  have hpl := P.inPlay_of_pagesR hp
  rcases (P.mem_pagesR).mp hp with h | ⟨e, hk, hc, hd⟩
  · exact (P.mem_pagesR).mpr (Or.inl ((real_rename H _).mpr ⟨p, h, rfl⟩))
  · refine (P.mem_pagesR).mpr (Or.inr ⟨?_, (pageSeedKeys_rename H hx _).mpr ⟨p.2, hk, rfl⟩, ?_, ?_⟩)
    · simp only [σp]; rw [H.keys.refKind p.2 hk]; rw [e]
    · simp only [σp]; rw [claimedKey_rename_σ H hpl]; exact hc
    · simp only [σp]; rw [declaredKey_rename_σ H hpl]; exact hd

/-- **T3 (pages), onto.** Every page of `G'` is the image of a page of `G`. -/
theorem pagesR_rename_onto (H : P.RenameHyps R m σ G) (hx : P.GraphExceptionFree R σ G) {r : Bool}
    {q : Page K} (hq : q ∈ P.pagesR r (renameBy R m G)) : ∃ p ∈ P.pagesR r G, σp σ p = q := by
  rcases (P.mem_pagesR).mp hq with h | ⟨e, hk, hc, hd⟩
  · obtain ⟨p, hp, rfl⟩ := (real_rename H _).mp h
    exact ⟨p, (P.mem_pagesR).mpr (Or.inl hp), rfl⟩
  · obtain ⟨k, hk0, ek⟩ := (pageSeedKeys_rename H hx _).mp hk
    have hkp : P.InPlay G k := Or.inr (Or.inl hk0)
    refine ⟨(P.refKind k, k), (P.mem_pagesR).mpr (Or.inr ⟨rfl, hk0, ?_, ?_⟩), ?_⟩
    · rw [← claimedKey_rename_σ H hkp, ek]; exact hc
    · simp only; rw [← declaredKey_rename_σ H hkp, ek]; exact hd
    · obtain ⟨κ, k'⟩ := q
      simp only at ek e ⊢
      simp only [σp, Prod.mk.injEq] at e ⊢
      refine ⟨?_, ek⟩
      rw [e.1, ← ek, H.keys.refKind k hk0]

/-- **T3 (pages), injective.** -/
theorem pagesR_rename_inj (H : P.RenameHyps R m σ G) {r : Bool} {p q : Page K} (hp : p ∈ P.pagesR r G)
    (hq : q ∈ P.pagesR r G) (e : σp σ p = σp σ q) : p = q :=
  σp_inj H.keys (P.inPlay_of_pagesR hp) (P.inPlay_of_pagesR hq) e

/-- **T3 (blocks of pages).** The blocks of `σp p` are the images of the blocks of `p`. -/
theorem pageBlocks_rename (H : P.RenameHyps R m σ G) {p : Page K} (hp : P.InPlay G p.2) :
    P.pageBlocks (renameBy R m G) (σp σ p) = (P.pageBlocks G p).map (blockMap R m G) := by
  unfold pageBlocks
  rw [owner_rename H hp]
  cases ho : P.owner G p with
  | none => rfl
  | some f =>
    have hf := (P.owner_spec ho).1
    simp only [Option.map_some, List.map_map]
    rw [outline_length_rename H.laws hf]
    apply List.map_congr_left
    intro i _
    simp only [Function.comp]
    rw [blockMap_eq H.wf hf]

/-- **T3 (pages that do not move keep exactly their blocks).** -/
theorem pageBlocks_fixed (H : P.RenameHyps R m σ G) {p : Page K} (hp : P.InPlay G p.2) (hfix : σ p.2 = p.2) :
    P.pageBlocks (renameBy R m G) p = P.pageBlocks G p := by
  have e : σp σ p = p := by obtain ⟨κ, k⟩ := p; simp only [σp] at hfix ⊢; rw [hfix]
  have := pageBlocks_rename H hp
  rw [e] at this
  rw [this]
  unfold pageBlocks
  cases ho : P.owner G p with
  | none => rfl
  | some f =>
    have ⟨hf, hc⟩ := P.owner_spec ho
    simp only [List.map_map]
    apply List.map_congr_left
    intro i _
    simp only [Function.comp]
    rw [blockMap_eq H.wf hf, H.laws.path f hf (by rw [hc]; exact hfix)]

end Params
end CM
