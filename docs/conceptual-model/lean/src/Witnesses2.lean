/-
Round 2: kernel-checked witnesses and counterexamples (T3, T5, T6). All by `decide` / `decide +kernel`
(kernel evaluation; no `native_decide`, no extra axiom).

**Toy instance.** Keys are the spellings themselves (`key = id`, so K1 holds with `spell = id`). A file
text is blocks separated by `;`, a block is tokens separated by spaces: `t:X` title, `a:X` alias member,
`l:X` link, `g:X` tag, `p:X` property key, `d` one level of depth, `h` (first block) pre-block marker.
The name is the title, else the path. The rewriter `RW` rewrites `t:`, `a:`, `l:`, `g:` tokens with the
spelling map and renames an untitled file; `p:` (property keys) is SPEC §5's exception. `RWna` is the
same without the `alias::` rewrite (Tine today, SPEC §9). With `key = id` the key map of a spelling
map `m` is `m` itself.
-/
import Lean
import Links
import Query
import Orders

namespace CM
namespace Witnesses2
open CM CM.Params

/-! ## The toy instance -/

def splitOnC (c : Char) : List Char → List (List Char)
  | [] => [[]]
  | x :: xs =>
    let r := splitOnC c xs
    if x = c then [] :: r
    else match r with
      | h :: t => (x :: h) :: t
      | [] => [[x]]

def splitStr (c : Char) (s : String) : List String := (splitOnC c s.toList).map String.ofList
def blocksOf (t : String) : List String := splitStr ';' t
def toks (b : String) : List String := (splitStr ' ' b).filter (fun t => !t.isEmpty)
def allToks (t : String) : List String := (blocksOf t).flatMap toks

def tokVal (c : Char) (tok : String) : Option String :=
  match tok.toList with
  | x :: ':' :: rest => if x = c then some (String.ofList rest) else none
  | _ => none

def occTok (tok : String) : Option (OccKind × String) :=
  match tokVal 'l' tok with
  | some x => some (.link, x)
  | none => match tokVal 'g' tok with
    | some x => some (.tag, x)
    | none => (tokVal 'p' tok).map (fun x => (.propertyKey, x))

def T : Params String where
  key := id
  stem f := f.path
  title _ t := (allToks t).findSome? (tokVal 't')
  aliasDecl _ t := (allToks t).filterMap (tokVal 'a')
  outline _ t := blocksOf t
  occ _ b := (toks b).filterMap occTok
  fileLe := Orders.fileLe
  pageLe := Orders.pageLe
  refKind _ := .page
  kindPref := .page

theorem lawful : T.Lawful := ⟨Orders.fileLe_linear, Orders.pageLe_linear⟩
theorem keyLaws : KeyLaws T.key id := ⟨fun _ => rfl⟩

def rwTok (alias : Bool) (m : String → String) (tok : String) : String :=
  match tok.toList with
  | x :: ':' :: rest =>
    if x = 't' || (alias && x = 'a') || x = 'l' || x = 'g' then String.ofList (x :: ':' :: (m (String.ofList rest)).toList)
    else tok
  | _ => tok

def rwText (alias : Bool) (m : String → String) (t : String) : String :=
  ";".intercalate ((blocksOf t).map (fun b => " ".intercalate ((splitStr ' ' b).map (rwTok alias m))))

def covers (k : OccKind) : Bool := k == .link || k == .tag || k == .pageEmbed || k == .linkableValue

def mkRW (alias : Bool) : Rewriter where
  covers := covers
  apply m f := { f with
    path := if (T.title f.format f.text).isNone then m f.path else f.path
    text := rwText alias m f.text }

/-- The rewriter of SPEC §5 (with the `alias::` rewrite). -/
def RW : Rewriter := mkRW true
/-- Tine today: no `alias::` rewrite. -/
def RWna : Rewriter := mkRW false

def pg (path text : String) : File := ⟨path, .pages, .md, text⟩
def fav : Page String := (.page, "favorites")

/-- A finite pigeonhole: an injection from a duplicate-free list into a list. -/
theorem length_le_of_inj {α β : Type} [DecidableEq β] {A : List α} (hA : A.Nodup) (f : α → β) :
    ∀ {B : List β}, (∀ x ∈ A, f x ∈ B) → (∀ x ∈ A, ∀ y ∈ A, f x = f y → x = y) → A.length ≤ B.length := by
  induction A with
  | nil => intros; simp
  | cons a A ih =>
    intro B hin hinj
    rw [List.nodup_cons] at hA
    have ha := hin a (List.mem_cons_self ..)
    have hB : (B.erase (f a)).length = B.length - 1 := List.length_erase_of_mem ha
    have hpos : 0 < B.length := List.length_pos_of_mem ha
    have := ih hA.2 (B := B.erase (f a))
      (fun x hx => by
        have hne : f x ≠ f a := fun e =>
          hA.1 (hinj x (List.mem_cons_of_mem _ hx) a (List.mem_cons_self ..) e ▸ hx)
        exact (List.mem_erase_of_ne hne).mpr (hin x (List.mem_cons_of_mem _ hx)))
      (fun x hx y hy e => hinj x (List.mem_cons_of_mem _ hx) y (List.mem_cons_of_mem _ hy) e)
    simp only [List.length_cons]; omega

/-- A page bijection in SPEC T3's sense: into, injective, onto. -/
def PageBij (σ' : Page String → Page String) (A B : List (Page String)) : Prop :=
  (∀ p ∈ A, σ' p ∈ B) ∧ (∀ p ∈ A, ∀ q ∈ A, σ' p = σ' q → p = q) ∧ (∀ q ∈ B, ∃ p ∈ A, σ' p = q)

/-- No bijection exists between two universes whose distinct members (`A₀ ⊆ A`, `B₀` covering `B`)
are counted differently. -/
theorem no_bij_lt {A B A₀ B₀ : List (Page String)} (hA₀ : A₀.Nodup) (hA₀A : ∀ p ∈ A₀, p ∈ A)
    (hBB₀ : ∀ q ∈ B, q ∈ B₀) (hlt : B₀.length < A₀.length) : ¬ ∃ σ', PageBij σ' A B := by
  rintro ⟨σ', hin, hinj, _⟩
  have := length_le_of_inj hA₀ σ' (B := B₀) (fun p hp => hBB₀ _ (hin p (hA₀A p hp)))
    (fun p hp q hq e => hinj p (hA₀A p hp) q (hA₀A q hq) e)
  omega

/-- A finite counting fact: a duplicate-free list all of whose members have a preimage in `A`
is no longer than `A`. -/
theorem length_le_of_surj {α β : Type} [DecidableEq α] (f : α → β) :
    ∀ (C : List β) {A : List α}, C.Nodup → (∀ q ∈ C, ∃ p ∈ A, f p = q) → C.length ≤ A.length := by
  intro C
  induction C with
  | nil => intros; simp
  | cons c C ih =>
    intro A hC hpre
    rw [List.nodup_cons] at hC
    obtain ⟨p, hp, ep⟩ := hpre c (List.mem_cons_self ..)
    have hlen : (A.erase p).length = A.length - 1 := List.length_erase_of_mem hp
    have hpos : 0 < A.length := List.length_pos_of_mem hp
    have := ih (A := A.erase p) hC.2 (fun q hq => by
      obtain ⟨p', hp', e'⟩ := hpre q (List.mem_cons_of_mem _ hq)
      have hne : p' ≠ p := fun e => by
        have : q = c := by rw [← e', ← ep, e]
        exact hC.1 (this ▸ hq)
      exact ⟨p', (List.mem_erase_of_ne hne).mpr hp', e'⟩)
    simp only [List.length_cons]; omega

theorem no_bij_gt {A B A₀ B₀ : List (Page String)} (hB₀ : B₀.Nodup) (hB₀B : ∀ q ∈ B₀, q ∈ B)
    (hAA₀ : ∀ p ∈ A, p ∈ A₀) (hlt : A₀.length < B₀.length) : ¬ ∃ σ', PageBij σ' A B := by
  rintro ⟨σ', _, _, honto⟩
  have := length_le_of_surj σ' B₀ (A := A₀) hB₀ (fun q hq => by
    obtain ⟨p, hp, e⟩ := honto q (hB₀B q hq); exact ⟨p, hAA₀ p hp, e⟩)
  omega

/-! ## W1: the T3 / T6 replacement claims apply (non-vacuity)

`old` is titled (file `f-old`), `old/sub` is a real child, `p` declares `old` as an alias, `c` links
`old/sub` (closure: `old`), has a property key `q`, tags `old`, and links the reference-only child
`old/ref`. Rename `old → new`. -/

def W1 : Graph := [pg "f-old" "h t:old;l:c", pg "old/sub" "l:p", pg "p" "h a:old",
  pg "c" "l:old/sub p:q;d g:old;d d l:old/ref"]
def m1 : String → String := nsSubst T.key T.urlHasOrigin "old" "new"
def W1' : Graph := T.rename RW W1 "old" "new"

theorem W1'_eq : W1' = [pg "f-old" "h t:new;l:c", pg "new/sub" "l:p", pg "p" "h a:new",
  pg "c" "l:new/sub p:q;d g:new;d d l:new/ref"] := by decide +kernel

theorem W1_allowed : T.renameAllowedLegacy W1 "old" "new" = true := by decide +kernel

theorem W1_hyps : T.RenameHyps RW m1 m1 W1 where
  wf := by unfold WF; decide +kernel
  wf' := by unfold WF; decide +kernel
  laws := { dir := by decide +kernel, format := by decide +kernel, name := by decide +kernel,
            path := by decide +kernel, occ := by decide +kernel, alias := by decide +kernel,
            order := by decide +kernel }
  keys := { alias := by decide +kernel, names := by decide +kernel, ns := by decide +kernel, inj := by decide +kernel,
            refKind := by decide +kernel }

theorem W1_xfree : T.GraphExceptionFree RW m1 W1 := by unfold GraphExceptionFree ExceptionFree; decide +kernel

theorem W1_fav : ∀ r, ∀ p ∈ T.pagesR r W1, σp m1 p = fav ↔ p = fav := by decide +kernel

/-- The block of `c` tagging `old` references `old` and, through the alias, `p`; the theorem carries both. -/
theorem W1_refs :
    T.References W1 ("c", 1) (.page, "old") ∧ T.References W1 ("c", 1) (.page, "p") ∧
    T.References W1' ("c", 1) (.page, "new") ∧ T.References W1' ("c", 1) (.page, "p") := by
  have hb : ("c", 1) ∈ T.blocks W1 := by decide +kernel
  have h1 : T.References W1 ("c", 1) (.page, "old") := (T.referencesB_iff _ _ _).mp (by decide +kernel)
  have h2 : T.References W1 ("c", 1) (.page, "p") := (T.referencesB_iff _ _ _).mp (by decide +kernel)
  have hbm : blockMap RW m1 W1 ("c", 1) = ("c", 1) := by decide +kernel
  have e1 := references_rename W1_hyps hb (W1_xfree _ hb) (t := (.page, "old")) (Or.inl (by decide +kernel))
  have e2 := references_rename W1_hyps hb (W1_xfree _ hb) (t := (.page, "p")) (Or.inl (by decide +kernel))
  rw [hbm] at e1 e2
  exact ⟨h1, h2, e1.mp h1, e2.mp h2⟩

/-- The page universes are in bijection under `σp m1`, under both readings of P1 (the reference-only
child `old/ref` moves to `new/ref`). -/
theorem W1_pages (r : Bool) :
    (∀ p ∈ T.pagesR r W1, σp m1 p ∈ T.pagesR r W1') ∧ (∀ q ∈ T.pagesR r W1', ∃ p ∈ T.pagesR r W1, σp m1 p = q) ∧
    (∀ p ∈ T.pagesR r W1, ∀ q ∈ T.pagesR r W1, σp m1 p = σp m1 q → p = q) :=
  ⟨fun _ hp => pagesR_rename_fwd W1_hyps W1_xfree hp, fun _ hq => pagesR_rename_onto W1_hyps W1_xfree hq,
   fun _ hp _ hq e => pagesR_rename_inj W1_hyps hp hq e⟩

theorem W1_pages_eval : (T.pagesR false W1').contains (.page, "new/ref") = true ∧
    (T.pagesR false W1).contains (.page, "old/ref") = true := by decide +kernel

/-- T6 applied: `c → old` before, `c → new` after (and the link through the alias to `p` survives). -/
theorem W1_link : T.Link false fav W1 (.page, "c") (.page, "old") ∧
    T.Link false fav W1' (σp m1 (.page, "c")) (σp m1 (.page, "old")) ∧
    T.Link false fav W1' (.page, "c") (.page, "p") := by
  have h : T.Link false fav W1 (.page, "c") (.page, "old") := (T.linkB_iff).mp (by decide +kernel)
  have hc : ((.page, "c") : Page String) ∈ T.pagesR false W1 := by decide +kernel
  have ho : ((.page, "old") : Page String) ∈ T.pagesR false W1 := by decide +kernel
  have hp : ((.page, "p") : Page String) ∈ T.pagesR false W1 := by decide +kernel
  have hcp : T.Link false fav W1 (.page, "c") (.page, "p") := (T.linkB_iff).mp (by decide +kernel)
  have e := (link_rename W1_hyps W1_xfree (W1_fav false) hc hp).mp hcp
  exact ⟨h, (link_rename W1_hyps W1_xfree (W1_fav false) hc ho).mp h, e⟩

/-! ## C1 (pre-dispatch T3): renaming a real page onto a reference-only one

`a` is real, `b` is reference-only. SPEC §5's precondition allows `rename a b`. Before: pages
`a, c, x, b`; after: `b, c, x`. No bijection exists, under either reading of P1. The failing T3
hypothesis is freshness (`KeyHyps.inj`). -/

def C1 : Graph := [pg "a" "l:x", pg "c" "l:b"]
def mC1 : String → String := nsSubst T.key T.urlHasOrigin "a" "b"
def C1' : Graph := T.rename RW C1 "a" "b"

theorem C1_allowed : T.renameAllowedLegacy C1 "a" "b" = true := by decide +kernel
theorem C1'_eq : C1' = [pg "b" "l:x", pg "c" "l:b"] := by decide +kernel

theorem C1_no_bij (r : Bool) : ¬ ∃ σ', PageBij σ' (T.pagesR r C1) (T.pagesR r C1') := by
  cases r <;>
  exact no_bij_lt (A₀ := [(.page, "a"), (.page, "c"), (.page, "x"), (.page, "b")])
    (B₀ := [(.page, "b"), (.page, "c"), (.page, "x")]) (by decide) (by decide +kernel) (by decide +kernel) (by decide)

theorem C1_laws : T.RwLaws RW mC1 mC1 C1 :=
  { dir := by decide +kernel, format := by decide +kernel, name := by decide +kernel,
    path := by decide +kernel, occ := by decide +kernel, alias := by decide +kernel,
    order := by decide +kernel }

theorem C1_not_fresh : ¬ T.KeyHyps RW mC1 mC1 C1 := fun h =>
  absurd (h.inj "a" (by decide +kernel) "b" (by decide +kernel) (by decide +kernel)) (by decide)

/-- The one-file variant (smallest by file count: an empty graph has no real `old`). -/
def C1min : Graph := [pg "a" "l:b"]
theorem C1min_wf : WF C1min ∧ WF (T.rename RW C1min "a" "b") := by unfold WF; decide +kernel
theorem C1min_allowed : T.renameAllowedLegacy C1min "a" "b" = true := by decide +kernel
theorem C1min_no_bij (r : Bool) :
    ¬ ∃ σ', PageBij σ' (T.pagesR r C1min) (T.pagesR r (T.rename RW C1min "a" "b")) := by
  cases r <;> exact no_bij_lt (A₀ := [(.page, "a"), (.page, "b")]) (B₀ := [(.page, "b")])
    (by decide) (by decide +kernel) (by decide +kernel) (by decide)

/-- `renameAllowedLegacy` implements SPEC's class refusal, which the subtree clause alone misses
(verifier round 2): `a` declares alias `a/b`, `a/b` is real and in `a`'s subtree. -/
def CR : Graph := [pg "a" "a:a/b", pg "a/b" ""]
theorem class_refusal : WF CR ∧ T.real CR (.page, "a/b") = true ∧ T.conceptB CR "a" "a/b" = true ∧
    T.renameAllowedLegacy CR "a" "a/b" = false := by unfold WF; decide +kernel

/-! ## C2: renaming across namespaces adds a namespace page

`rename x n/y`: before, pages `x, c`; after, `n/y`, `c` and the reference-only namespace page `n`
(closure of `[[n/y]]`). The failing hypothesis is namespace compatibility (`KeyHyps.ns`). -/

def C2 : Graph := [pg "x" "l:c", pg "c" "l:x"]
def mC2 : String → String := nsSubst T.key T.urlHasOrigin "x" "n/y"
def C2' : Graph := T.rename RW C2 "x" "n/y"

theorem C2_allowed : T.renameAllowedLegacy C2 "x" "n/y" = true := by decide +kernel
theorem C2'_eq : C2' = [pg "n/y" "l:c", pg "c" "l:n/y"] := by decide +kernel

theorem C2_no_bij (r : Bool) : ¬ ∃ σ', PageBij σ' (T.pagesR r C2) (T.pagesR r C2') := by
  cases r <;>
  exact no_bij_gt (B₀ := [(.page, "n/y"), (.page, "c"), (.page, "n")]) (A₀ := [(.page, "x"), (.page, "c")])
    (by decide) (by decide +kernel) (by decide +kernel) (by decide)

/-- After the rename `c`'s block references the page `n`, which no page before the rename maps to. -/
theorem C2_new_ref : T.References C2' ("c", 0) (.page, "n") :=
  (T.referencesB_iff _ _ _).mp (by decide +kernel)

theorem C2_laws : T.RwLaws RW mC2 mC2 C2 :=
  { dir := by decide +kernel, format := by decide +kernel, name := by decide +kernel,
    path := by decide +kernel, occ := by decide +kernel, alias := by decide +kernel,
    order := by decide +kernel }

theorem C2_not_ns : ¬ T.KeyHyps RW mC2 mC2 C2 := fun h =>
  absurd ((h.ns (pg "c" "l:x") (by decide) "l:x" (by decide +kernel) (.link, "x") (by decide +kernel) rfl).1
    "n" (by decide +kernel)) (by decide +kernel)

/-! ## C3: SPEC §5's literal rewriting set misses reference-only children

SPEC rewrites the spellings of the *moved pages* (here only the real `old`). The reference-only child
`old/x` is not rewritten, so `c` still references `old` through the closure of `old/x`, and `old`
reappears as a reference-only page. The repair is `nsSubst`: rewrite every spelling in `old`'s subtree. -/

def C3 : Graph := [pg "old" "", pg "c" "l:old/x"]
/-- SPEC's literal spelling map (`litMapLegacy`): only the moved real page's own spelling. -/
def mC3 : String → String := T.litMapLegacy C3 "old" "new"
def C3' : Graph := renameBy RW mC3 C3

theorem C3_wf : WF C3 ∧ WF C3' := by unfold WF; decide +kernel
theorem C3_allowed : T.renameAllowedLegacy C3 "old" "new" = true := by decide +kernel
theorem C3'_literal : C3' = T.renameLiteralLegacy RW C3 "old" "new" := rfl

theorem C3'_eq : C3' = [pg "new" "", pg "c" "l:old/x"] := by decide +kernel

theorem C3_no_bij (r : Bool) : ¬ ∃ σ', PageBij σ' (T.pagesR r C3) (T.pagesR r C3') := by
  cases r <;>
  exact no_bij_gt (B₀ := [(.page, "new"), (.page, "c"), (.page, "old/x"), (.page, "old")])
    (A₀ := [(.page, "old"), (.page, "c"), (.page, "old/x")]) (by decide) (by decide +kernel) (by decide +kernel) (by decide)

/-- `c` referenced the renamed page before and does not reference it after. -/
theorem C3_lost : T.References C3 ("c", 0) (.page, "old") ∧ ¬ T.References C3' ("c", 0) (.page, "new") :=
  ⟨(T.referencesB_iff _ _ _).mp (by decide +kernel),
   fun h => absurd ((T.referencesB_iff _ _ _).mpr h) (by decide +kernel)⟩

/-- With the repaired map (`nsSubst`), the same rename satisfies every T3 hypothesis. -/
theorem C3_repaired : T.RenameHyps RW (nsSubst T.key T.urlHasOrigin "old" "new") (nsSubst T.key T.urlHasOrigin "old" "new") C3 where
  wf := by unfold WF; decide +kernel
  wf' := by unfold WF; decide +kernel
  laws := { dir := by decide +kernel, format := by decide +kernel, name := by decide +kernel,
            path := by decide +kernel, occ := by decide +kernel, alias := by decide +kernel,
            order := by decide +kernel }
  keys := { alias := by decide +kernel, names := by decide +kernel, ns := by decide +kernel, inj := by decide +kernel,
            refKind := by decide +kernel }

/-! ## C4 (pre-dispatch T6): a property-key occurrence survives the rename

`s` has property key `a` (an uncovered occurrence kind). Every T3 hypothesis holds except
exception-freeness. `link s a` holds before; `link s b` fails after; `a` reappears as a
reference-only page, so there is not even a page bijection. -/

def C4 : Graph := [pg "a" "", pg "s" "p:a"]
def mC4 : String → String := nsSubst T.key T.urlHasOrigin "a" "b"
def C4' : Graph := T.rename RW C4 "a" "b"

theorem C4'_eq : C4' = [pg "b" "", pg "s" "p:a"] := by decide +kernel
theorem C4_allowed : T.renameAllowedLegacy C4 "a" "b" = true := by decide +kernel

theorem C4_hyps : T.RenameHyps RW mC4 mC4 C4 where
  wf := by unfold WF; decide +kernel
  wf' := by unfold WF; decide +kernel
  laws := { dir := by decide +kernel, format := by decide +kernel, name := by decide +kernel,
            path := by decide +kernel, occ := by decide +kernel, alias := by decide +kernel,
            order := by decide +kernel }
  keys := { alias := by decide +kernel, names := by decide +kernel, ns := by decide +kernel, inj := by decide +kernel,
            refKind := by decide +kernel }

theorem C4_not_xfree : ¬ T.GraphExceptionFree RW mC4 C4 := fun h =>
  absurd (h ("s", 0) (by decide +kernel) (.propertyKey, "a") (by decide +kernel) rfl "a" (by decide +kernel))
    (by decide +kernel)

theorem C4_link (r : Bool) : T.Link r fav C4 (.page, "s") (.page, "a") ∧
    ¬ T.Link r fav C4' (.page, "s") (.page, "b") := by
  cases r
  · exact ⟨(T.linkB_iff).mp (by decide +kernel), fun h => absurd ((T.linkB_iff).mpr h) (by decide +kernel)⟩
  · exact ⟨(T.linkB_iff).mp (by decide +kernel), fun h => absurd ((T.linkB_iff).mpr h) (by decide +kernel)⟩

theorem C4_no_bij (r : Bool) : ¬ ∃ σ', PageBij σ' (T.pagesR r C4) (T.pagesR r C4') := by
  cases r <;>
  exact no_bij_gt (B₀ := [(.page, "b"), (.page, "s"), (.page, "a")]) (A₀ := [(.page, "a"), (.page, "s")])
    (by decide) (by decide +kernel) (by decide +kernel) (by decide)

/-! ## C5: renaming onto Favorites

`link` excludes source Favorites (SPEC §6.2). Renaming `a` to `favorites` satisfies every T3
hypothesis and exception-freeness, yet kills `a → c`. -/

def C5 : Graph := [pg "a" "l:c", pg "c" ""]
def mC5 : String → String := nsSubst T.key T.urlHasOrigin "a" "favorites"
def C5' : Graph := T.rename RW C5 "a" "favorites"

theorem C5_allowed : T.renameAllowedLegacy C5 "a" "favorites" = true := by decide +kernel

theorem C5_hyps : T.RenameHyps RW mC5 mC5 C5 where
  wf := by unfold WF; decide +kernel
  wf' := by unfold WF; decide +kernel
  laws := { dir := by decide +kernel, format := by decide +kernel, name := by decide +kernel,
            path := by decide +kernel, occ := by decide +kernel, alias := by decide +kernel,
            order := by decide +kernel }
  keys := { alias := by decide +kernel, names := by decide +kernel, ns := by decide +kernel, inj := by decide +kernel,
            refKind := by decide +kernel }

theorem C5_xfree : T.GraphExceptionFree RW mC5 C5 := by unfold GraphExceptionFree ExceptionFree; decide +kernel

theorem C5_link : T.Link false fav C5 (.page, "a") (.page, "c") ∧
    ¬ T.Link false fav C5' (σp mC5 (.page, "a")) (σp mC5 (.page, "c")) :=
  ⟨(T.linkB_iff).mp (by decide +kernel), fun h => absurd ((T.linkB_iff).mpr h) (by decide +kernel)⟩

/-! ## C6: dropping the alias law (Tine today). Not a refutation of SPEC T3

SPEC §5 mandates the `alias::` rewrite; `RWna` omits it, so C6 refutes only the variant of T3 without
the alias law (the documented code discrepancy, SPEC §9), showing that `RwLaws.alias` is needed.
`p` declares `old`; `c` links `old`, so it references `p`. `RWna` renames `old` but leaves
`alias:: old`: afterwards `c` links `new`, which is not in `p`'s class. With `RW` (the SPEC rewrite)
the reference survives (`C6_spec_keeps`). -/

def C6 : Graph := [pg "p" "a:old", pg "old" "", pg "c" "l:old"]
def mC6 : String → String := nsSubst T.key T.urlHasOrigin "old" "new"
def C6' : Graph := T.rename RWna C6 "old" "new"

theorem C6'_eq : C6' = [pg "p" "a:old", pg "new" "", pg "c" "l:new"] := by decide +kernel

theorem C6_lost : T.References C6 ("c", 0) (.page, "p") ∧ ¬ T.References C6' ("c", 0) (.page, "p") :=
  ⟨(T.referencesB_iff _ _ _).mp (by decide +kernel),
   fun h => absurd ((T.referencesB_iff _ _ _).mpr h) (by decide +kernel)⟩

theorem C6_spec_keeps : T.References (T.rename RW C6 "old" "new") ("c", 0) (.page, "p") :=
  (T.referencesB_iff _ _ _).mp (by decide +kernel)

theorem C6_no_alias_law : ¬ T.RwLaws RWna mC6 mC6 C6 := fun h =>
  absurd (h.alias (pg "p" "a:old") (by decide)) (by decide +kernel)

/-! ## The SPEC §5 exception list is incomplete: task markers and priorities

`TODO` / `[#A]` occurrences reference pages (round 1, `OccKind.taskMarker`, `.priority`) but no
rewrite can change them (they are fixed vocabulary). A rename of the page `todo` therefore breaks
`References` exactly like a property key. Shown with an extended toy reading `k:X` as a task marker. -/

def occTokK (tok : String) : Option (OccKind × String) :=
  match tokVal 'k' tok with
  | some x => some (.taskMarker, x)
  | none => occTok tok

def TK : Params String := { T with occ := fun _ b => (toks b).filterMap occTokK }

def C7 : Graph := [pg "todo" "", pg "s" "k:todo"]
def C7' : Graph := TK.rename RW C7 "todo" "chores"

theorem C7_lost : TK.References C7 ("s", 0) (.page, "todo") ∧ ¬ TK.References C7' ("s", 0) (.page, "chores") :=
  ⟨(TK.referencesB_iff _ _ _).mp (by decide +kernel),
   fun h => absurd ((TK.referencesB_iff _ _ _).mpr h) (by decide +kernel)⟩

/-! ## C8: O7 — a journal and a page claiming the moved key

SPEC §5: journals never move. A journal at path `old` and the page `p` titled `old` share a key (O7).
`c` links `old`, so it references both. A journal-respecting rewriter (`RWj`) moves the page and
rewrites the link; the journal stays `(journal, old)`, an unmoved page, and `c` no longer references
it. (The replacement T3 excludes this case: its `RwLaws.name` would require the journal to move as
well.) Graph from the round-2 verifier. -/

def RWj : Rewriter where
  covers := covers
  apply m f := if f.dir = .journals then { f with text := rwText true m f.text } else RW.apply m f

def C8 : Graph := [⟨"old", .journals, .md, ""⟩, pg "p" "t:old", pg "c" "l:old"]
def C8' : Graph := T.rename RWj C8 "old" "new"

theorem C8_wf : WF C8 ∧ WF C8' := by unfold WF; decide +kernel
theorem C8_allowed : T.renameAllowedLegacy C8 "old" "new" = true := by decide +kernel

theorem C8_lost : T.References C8 ("c", 0) (.journal, "old") ∧ ¬ T.References C8' ("c", 0) (.journal, "old") ∧
    ((.journal, "old") : Page String) ∈ T.pagesR false C8' :=
  ⟨(T.referencesB_iff _ _ _).mp (by decide +kernel),
   fun h => absurd ((T.referencesB_iff _ _ _).mpr h) (by decide +kernel), by decide +kernel⟩

/-! ## `KeyHyps.ns` is not implied by a shared namespace parent for an arbitrary key

(Verifier round 2.) With `key = length` (K1 holds with `spell n = "x"^n`), rename `a/x → a/yy`
(shared parent `a`), `σ 3 = 4`: the occurrence `bcd` has `old`'s key, is rewritten to `a/yy`, and its
closure keys `[4, 1]` differ from the image `[4]` of its old closure keys. A shared parent suffices only
under further key laws (e.g. `key` respecting `/`-segments), which the model does not assume. -/

def lenKey (s : String) : Nat := s.toList.length
def lenSpell (n : Nat) : String := String.ofList (List.replicate n 'x')
theorem lenKey_law : KeyLaws lenKey lenSpell := by
  constructor; intro n; simp [lenKey, lenSpell]
def lenMove (n : Nat) : Nat := if n = 3 then 4 else n
theorem shared_parent_not_sufficient :
    lenKey "a/x" ≠ lenKey "a/yy" ∧
    (nsClosure (nsSubst lenKey (fun _ => false) "a/x" "a/yy" "bcd")).map lenKey = [4, 1] ∧
    (nsClosure "bcd").map (fun s => lenMove (lenKey s)) = [4] := by decide +kernel

/-! ## T5 witnesses

Toy query parameters: depth = number of `d` tokens; the first block is a pre-block when it has the
token `h`; given leaves are `hasProp k` (block: a property key `k`; page: its pre-block has one — the
page's properties are the pre-block's, as in Tine) and `content w` (block: token `w`; page: false,
Tine's content leaf is block-only). -/

inductive GL | hasProp (k : String) | content (w : String)
  deriving Repr

def blockToks (G : Graph) (b : BlockId) : List String :=
  match fileAt G b.1 with
  | none => []
  | some f => match (blocksOf f.text)[b.2]? with | some t => toks t | none => []

def Q : QParams String GL where
  levels _ t := (blocksOf t).map (fun b => (toks b).countP (· == "d"))
  preBlock _ t := match blocksOf t with | b :: _ => (toks b).contains "h" | [] => false
  blockLeaf l G b := match l with
    | .hasProp k => (T.occAt G b).contains (.propertyKey, k)
    | .content w => (blockToks G b).contains w
  pageLeaf l G p := match l with
    | .hasProp k => match T.preBlockOf Q0 G p with
      | some b => (T.occAt G b).contains (.propertyKey, k)
      | none => false
    | .content _ => false
where Q0 : QParams String GL := ⟨fun _ t => (blocksOf t).map (fun b => (toks b).countP (· == "d")),
  fun _ t => match blocksOf t with | b :: _ => (toks b).contains "h" | [] => false,
  fun _ _ _ => false, fun _ _ _ => false⟩

def GQ : Graph := [pg "a" "h p:type;l:b;d l:c", pg "b" "l:a", pg "p" "h a:x", pg "c" "l:x"]

theorem GQ_wf : WF GQ := by unfold WF; decide

/-- Path references read ancestors and the page: block `a#2` (child of `a#1`, which links `b`). -/
theorem GQ_pathRefs : T.pathRefKeys Q GQ ("a", 2) = ["c", "b", "a"] := by decide +kernel

/-- O6: `refs` matches by key with no alias expansion. `c` links `x`, an alias of `p`: it references
`p` (SPEC §3), but the query leaf `refs p` is false on it. -/
theorem GQ_O6 : T.References GQ ("c", 0) (.page, "p") ∧ T.leafBlockLegacy Q GQ (.refs "p") ("c", 0) = false ∧
    T.leafBlockLegacy Q GQ (.refs "x") ("c", 0) = true :=
  ⟨(T.referencesB_iff _ _ _).mp (by decide +kernel), by decide +kernel, by decide +kernel⟩

/-- P1: the two readings differ on the declared, file-less, referenced key `x`. -/
theorem GQ_P1 : ((.page, "x") : Page String) ∈ T.pagesR false GQ ∧ ((.page, "x") : Page String) ∉ T.pagesR true GQ := by
  decide +kernel

def leaves0 : List (Leaf GL) :=
  [.refs "a", .refs "b", .name "a", .name "b", .page (.refs "a"), .page (.name "a"),
   .given (.hasProp "type"), .given (.content "h")]

/-- The model computes the kind-uniform leaves (both readings agree here): `refs a`, `name a` and
`content h` are not kind-uniform on `GQ` (page `a` has a pre-block), the rest are. -/
theorem GQ_uniform (r : Bool) : leaves0.map (T.kindUniformBLegacy Q r GQ) =
    [false, true, false, true, true, true, true, false] := by
  cases r <;> decide +kernel

theorem GQ_refs_not_uniform : ¬ T.KindUniformLegacy Q false GQ (.refs "a") :=
  refs_not_kindUniform (P := T) Q GQ_wf (p := (.page, "a")) (by decide +kernel) (b := ("a", 0)) (by decide +kernel) rfl

def qU : Query GL := .and (.leaf (.given (.hasProp "type"))) (.not (.leaf (.refs "b")))
def qN : Query GL := .leaf (.name "a")

/-- T5d applied: all leaves of `qU` are kind-uniform, and page `a` and its pre-block `a#0` are both in. -/
theorem GQ_query_uniform :
    (Node.page (.page, "a") ∈ T.denLegacy Q false GQ .page qU ↔ Node.block ("a", 0) ∈ T.denLegacy Q false GQ .block qU) ∧
    Node.page (.page, "a") ∈ T.denLegacy Q false GQ .page qU := by
  refine ⟨query_uniform (P := T) Q GQ_wf (fun l hl => ?_) (by decide +kernel) (by decide +kernel), by decide +kernel⟩
  simp only [qU, Query.leaves, List.mem_append, List.mem_singleton] at hl
  rcases hl with rfl | rfl
  · exact (T.kindUniformB_iff Q).mp (by decide +kernel)
  · exact (T.kindUniformB_iff Q).mp (by decide +kernel)

/-- The hypothesis is needed: `name a` is not kind-uniform, and page `a` is in `qN`'s page reading
while its pre-block is not in the block reading. -/
theorem GQ_needs_uniform : Node.page (.page, "a") ∈ T.denLegacy Q false GQ .page qN ∧
    Node.block ("a", 0) ∉ T.denLegacy Q false GQ .block qN := by decide +kernel

/-- T5d is sufficient, not necessary (verifier round 2): `name a ∨ page(name a)` has the non-uniform
leaf `name a`, yet its two readings agree at every page/pre-block pair (non-constantly). -/
def qAbsorb : Query GL := .or (.leaf (.name "a")) (.leaf (.page (.name "a")))
theorem GQ_coherent_nonuniform (r : Bool) : T.kindUniformBLegacy Q r GQ (.name "a") = false ∧
    Node.page (.page, "a") ∈ T.denLegacy Q r GQ .page qAbsorb ∧ Node.block ("a", 0) ∈ T.denLegacy Q r GQ .block qAbsorb ∧
    Node.page (.page, "p") ∉ T.denLegacy Q r GQ .page qAbsorb ∧ Node.block ("p", 0) ∉ T.denLegacy Q r GQ .block qAbsorb := by
  cases r <;> decide +kernel

/-- T5b spot check: grouping by kind displays exactly the denotation. -/
theorem GQ_view : (displayed ((View.group (fun n : Node String => match n with | .page _ => "page" | .block _ => "block")).apply
    (T.denLegacy Q false GQ .page qU))).length = (T.denLegacy Q false GQ .page qU).length := by decide +kernel

/-! ## T6 witnesses -/

def GL3 : Graph := [pg "a" "l:b", pg "c" "l:b", pg "b" ""]

/-- `near … both` is out ∪ in, not the undirected ball: from `a`, two steps reach `c` only by going
forward then backward. -/
theorem GL3_near_ne_ball : ((.page, "c") : Page String) ∈ T.ballL false fav GL3 2 [(.page, "a")] ∧
    ((.page, "c") : Page String) ∉ T.nearL false fav GL3 .both 2 [(.page, "a")] := by decide +kernel

def GA : Graph := [pg "s" "l:x", pg "p" "a:x"]

/-- A link to an alias spelling is a link to the alias class; P1 decides whether `x` is a page of its
own (reading A) or not (reading B). -/
theorem GA_links : T.Link false fav GA (.page, "s") (.page, "p") ∧ T.Link true fav GA (.page, "s") (.page, "p") ∧
    T.Link false fav GA (.page, "s") (.page, "x") ∧ ¬ T.Link true fav GA (.page, "s") (.page, "x") :=
  ⟨(T.linkB_iff).mp (by decide +kernel), (T.linkB_iff).mp (by decide +kernel), (T.linkB_iff).mp (by decide +kernel),
   fun h => absurd ((T.linkB_iff).mpr h) (by decide +kernel)⟩

/-! ## The refutations, as headlines -/

theorem C1_wf : WF C1 := by unfold WF; decide
theorem C4_wf : WF C4 := by unfold WF; decide

/-- **SPEC T3 refuted** (bijection part), under either reading of P1: SPEC §5's precondition does not
give a page bijection, even for a lawful rewriter (C1; `C1_laws`). -/
theorem T3_spec_refuted (r : Bool) : ¬ ∀ (G : Graph) (old new : String), WF G →
    T.renameAllowedLegacy G old new = true → ∃ σ', PageBij σ' (T.pagesR r G) (T.pagesR r (T.rename RW G old new)) :=
  fun h => C1_no_bij r (h C1 "a" "b" C1_wf C1_allowed)

/-- **SPEC T3 refuted** (references part, C3): with SPEC §5's literal rewriting set (`renameLiteralLegacy`,
the toy rewriter `RW`), on a valid graph and an allowed rename, a block of `G` referencing `old` does
not reference `new` afterwards. -/
theorem T3_spec_refs_refuted : ¬ ∀ (G : Graph) (old new : String) (b : BlockId), WF G →
    T.renameAllowedLegacy G old new = true → b ∈ T.blocks G → T.References G b (.page, old) →
    T.References (T.renameLiteralLegacy RW G old new) (blockMap RW (T.litMapLegacy G old new) G b) (.page, new) :=
  fun h => C3_lost.2 (by
    have := h C3 "old" "new" ("c", 0) C3_wf.1 C3_allowed (by decide +kernel) C3_lost.1
    have e : blockMap RW (T.litMapLegacy C3 "old" "new") C3 ("c", 0) = ("c", 0) := by decide +kernel
    rw [e] at this
    exact this)

/-- **SPEC T6 refuted** (pre-dispatch counterexample C4), under either reading of P1, even when every
T3 hypothesis except exception-freeness holds (`C4_hyps`). -/
theorem T6_spec_refuted (r : Bool) : ¬ ∀ (G : Graph) (old new : String) (S : Page String), WF G →
    T.renameAllowedLegacy G old new = true → T.Link r fav G S (.page, old) →
    T.Link r fav (T.rename RW G old new) (σp (nsSubst T.key T.urlHasOrigin old new) S) (.page, new) :=
  fun h => (C4_link r).2 (by
    have := h C4 "a" "b" (.page, "s") C4_wf C4_allowed (C4_link r).1
    exact this)

/-- **The Favorites condition of T6 is needed** (C5): every T3 hypothesis and exception-freeness hold. -/
theorem T6_needs_fav : T.RenameHyps RW mC5 mC5 C5 ∧ T.GraphExceptionFree RW mC5 C5 ∧
    T.Link false fav C5 (.page, "a") (.page, "c") ∧
    ¬ T.Link false fav C5' (σp mC5 (.page, "a")) (σp mC5 (.page, "c")) :=
  ⟨C5_hyps, C5_xfree, C5_link⟩

end Witnesses2
end CM

#print axioms CM.Witnesses2.W1_hyps
#print axioms CM.Witnesses2.W1_xfree
#print axioms CM.Witnesses2.W1_refs
#print axioms CM.Witnesses2.W1_pages
#print axioms CM.Witnesses2.W1_link
#print axioms CM.Witnesses2.C1_no_bij
#print axioms CM.Witnesses2.C1_laws
#print axioms CM.Witnesses2.C1_not_fresh
#print axioms CM.Witnesses2.C2_no_bij
#print axioms CM.Witnesses2.C2_laws
#print axioms CM.Witnesses2.C2_not_ns
#print axioms CM.Witnesses2.C2_new_ref
#print axioms CM.Witnesses2.C3_no_bij
#print axioms CM.Witnesses2.C3_lost
#print axioms CM.Witnesses2.C3_repaired
#print axioms CM.Witnesses2.C4_hyps
#print axioms CM.Witnesses2.C4_not_xfree
#print axioms CM.Witnesses2.C4_link
#print axioms CM.Witnesses2.C4_no_bij
#print axioms CM.Witnesses2.C5_link
#print axioms CM.Witnesses2.C6_lost
#print axioms CM.Witnesses2.C6_no_alias_law
#print axioms CM.Witnesses2.C7_lost
#print axioms CM.Witnesses2.GQ_coherent_nonuniform
#print axioms CM.Witnesses2.shared_parent_not_sufficient
#print axioms CM.Witnesses2.C8_wf
#print axioms CM.Witnesses2.C6_spec_keeps
#print axioms CM.Witnesses2.class_refusal
#print axioms CM.Witnesses2.C1min_no_bij
#print axioms CM.Witnesses2.C8_lost
#print axioms CM.Witnesses2.GQ_pathRefs
#print axioms CM.Witnesses2.GQ_O6
#print axioms CM.Witnesses2.GQ_P1
#print axioms CM.Witnesses2.GQ_uniform
#print axioms CM.Witnesses2.GQ_refs_not_uniform
#print axioms CM.Witnesses2.GQ_query_uniform
#print axioms CM.Witnesses2.GQ_needs_uniform
#print axioms CM.Witnesses2.GQ_view
#print axioms CM.Witnesses2.GL3_near_ne_ball
#print axioms CM.Witnesses2.GA_links
#print axioms CM.Witnesses2.T3_spec_refuted
#print axioms CM.Witnesses2.T3_spec_refs_refuted
#print axioms CM.Witnesses2.T6_spec_refuted
#print axioms CM.Witnesses2.T6_needs_fav

open Lean Elab Command in
-- The same cone audit as `Headlines`, over the round-2 witnesses and refutations.
run_cmd do
  let env ← getEnv
  let mut pending : List Name := [``CM.Witnesses2.W1_hyps, ``CM.Witnesses2.W1_xfree, ``CM.Witnesses2.W1_refs, ``CM.Witnesses2.W1_pages, ``CM.Witnesses2.W1_link, ``CM.Witnesses2.C1_no_bij, ``CM.Witnesses2.C1_laws, ``CM.Witnesses2.C1_not_fresh, ``CM.Witnesses2.C2_no_bij, ``CM.Witnesses2.C2_laws, ``CM.Witnesses2.C2_not_ns, ``CM.Witnesses2.C2_new_ref, ``CM.Witnesses2.C3_no_bij, ``CM.Witnesses2.C3_lost, ``CM.Witnesses2.C3_repaired, ``CM.Witnesses2.C4_hyps, ``CM.Witnesses2.C4_not_xfree, ``CM.Witnesses2.C4_link, ``CM.Witnesses2.C4_no_bij, ``CM.Witnesses2.C5_link, ``CM.Witnesses2.C6_lost, ``CM.Witnesses2.C6_no_alias_law, ``CM.Witnesses2.C7_lost, ``CM.Witnesses2.GQ_coherent_nonuniform, ``CM.Witnesses2.shared_parent_not_sufficient, ``CM.Witnesses2.C8_wf, ``CM.Witnesses2.C6_spec_keeps, ``CM.Witnesses2.class_refusal, ``CM.Witnesses2.C1min_no_bij, ``CM.Witnesses2.C8_lost, ``CM.Witnesses2.GQ_pathRefs, ``CM.Witnesses2.GQ_O6, ``CM.Witnesses2.GQ_P1, ``CM.Witnesses2.GQ_uniform, ``CM.Witnesses2.GQ_refs_not_uniform, ``CM.Witnesses2.GQ_query_uniform, ``CM.Witnesses2.GQ_needs_uniform, ``CM.Witnesses2.GQ_view, ``CM.Witnesses2.GL3_near_ne_ball, ``CM.Witnesses2.GA_links, ``CM.Witnesses2.T3_spec_refuted, ``CM.Witnesses2.T3_spec_refs_refuted, ``CM.Witnesses2.T6_spec_refuted, ``CM.Witnesses2.T6_needs_fav]
  let mut seen : NameSet := {}
  let mut axs : NameSet := {}
  while !pending.isEmpty do
    let n := pending.head!
    pending := pending.tail!
    if seen.contains n then continue
    seen := seen.insert n
    let some info := env.checked.get.find? n | throwError "missing checked constant {n}"
    if info.isUnsafe then throwError "unsafe dependency {n}"
    if let .axiomInfo _ := info then
      axs := axs.insert n
      unless n == ``propext || n == ``Classical.choice || n == ``Quot.sound do
        throwError "nonstandard axiom {n}"
    pending := info.type.getUsedConstants.toList ++ pending
    if let some v := info.value? (allowOpaque := true) then
      pending := v.getUsedConstants.toList ++ pending
    if let .inductInfo i := info then pending := i.ctors ++ pending
  logInfo m!"WITNESS CONE: {seen.size} constants; axioms {axs.toList}"
