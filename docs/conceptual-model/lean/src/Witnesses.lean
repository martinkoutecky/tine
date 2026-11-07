/-
Kernel-checked non-vacuity witnesses for the round-1 headlines. **Construction by the independent
round-1 verifier** (Codex, 2026-10-19, `subagent-tasks/notes/model-v0-lean-r1-verify.md`, its
`Verifier.lean`), brought into the project with only the shared orders moved to `Orders.lean` and the
page-universe witnesses added.

* `lawful`, `keyLaws`: a parameter record satisfying `Params.Lawful` and K1.
* A genuine edit at path `z` (`WF`, `DiffAt`, `G ≠ G'`): the edited file changes its claimed key from
  `z` to `q`, and a previously shadowed claimant at `zz` becomes the owner of `z` (owner displacement),
  contributing the edge `(z, w)`. The class of `u` avoids the touched keys, so `T4_resolve_spec` applies
  (`locality_applies`); `resolve "x"` really changes (`resolve_changes`); the computed delta is
  `delta_computed` and `T4_delta` applies.
* `T1_set` applied to `G` and `G.reverse ++ G` (permutation with duplication).
* `demo_lawful`: the demo's parameters are lawful (round-1 nit fixed: ties broken on every field).
-/
import LegacyHeadlines

namespace CM
namespace Witnesses
open CM CM.Params

def P : Params String where
  key := id
  stem f := if f.path = "zz" then "z" else f.path
  title _ t := if t = "new" then some "q" else none
  aliasDecl _ t :=
    if t = "a" then ["b"] else if t = "u" then ["v"] else
    if t = "old" then ["x"] else if t = "new" then ["y"] else if t = "shadow" then ["w"] else []
  outline _ t := [t]
  occ _ t := [(.link, t)]
  fileLe := Orders.fileLe
  pageLe := Orders.pageLe
  refKind _ := .page
  kindPref := .page

theorem lawful : P.Lawful := ⟨Orders.fileLe_linear, Orders.pageLe_linear⟩
theorem keyLaws : KeyLaws P.key id := ⟨fun _ => rfl⟩

def fa : File := ⟨"a", .pages, .md, "a"⟩
def fu : File := ⟨"u", .pages, .md, "u"⟩
def old : File := ⟨"z", .pages, .md, "old"⟩
def new : File := ⟨"z", .pages, .md, "new"⟩
def shadow : File := ⟨"zz", .pages, .md, "shadow"⟩
def G : Graph := [fa,fu,old,shadow]
def G' : Graph := [fa,fu,new,shadow]

theorem wf : WF G ∧ WF G' := by unfold WF; decide
theorem changed : G ≠ G' := by decide
theorem diffAt : DiffAt "z" G G' := by
  intro g hg
  simp only [G, G', List.mem_cons, List.not_mem_nil, or_false]
  have h1 : g ≠ old := fun e => hg (e ▸ rfl)
  have h2 : g ≠ new := fun e => hg (e ▸ rfl)
  simp [h1,h2]

theorem edges_before : P.aliasEdges G = [("a","b"),("u","v"),("z","x")] := by decide
theorem edges_after : P.aliasEdges G' = [("a","b"),("u","v"),("q","y"),("z","w")] := by decide

theorem unaffected_component_before (y : String) : P.aliasClass G "u" y ↔ y = "u" ∨ y = "v" := by
  simp [aliasClass, cls, edges_before, Reach.cons_iff, Reach.nil_iff, eq_comm]

theorem unaffected_component_after (y : String) : P.aliasClass G' "u" y ↔ y = "u" ∨ y = "v" := by
  simp [aliasClass, cls, edges_after, Reach.cons_iff, Reach.nil_iff, eq_comm]

theorem avoided (y : String) (hy : y = "u" ∨ y = "v") : ¬ P.ClaimedOrDeclared "z" G G' y := by
  rcases hy with rfl | rfl <;>
    rintro ⟨f, hm, hp, hc⟩ <;>
    rcases hm with hm | hm <;>
    simp only [G, G', List.mem_cons, List.not_mem_nil, or_false] at hm <;>
    rcases hm with rfl | rfl | rfl | rfl <;>
    simp_all [fa, fu, old, new, shadow, claim, name, kindOf, declaresKey, P]

theorem locality_hypotheses :
    (∀ y, P.aliasClass G "u" y → ¬ P.ClaimedOrDeclared "z" G G' y) ∧
    (∀ y, P.aliasClass G' "u" y → ¬ P.ClaimedOrDeclared "z" G G' y) :=
  ⟨fun y hy => avoided y ((unaffected_component_before y).mp hy),
   fun y hy => avoided y ((unaffected_component_after y).mp hy)⟩

theorem locality_applies : P.resolve G "u" = P.resolve G' "u" ∧ P.concept G "u" = P.concept G' "u" :=
  CM.LegacyHeadlines.T4_resolve_spec P lawful diffAt "u" locality_hypotheses.1 locality_hypotheses.2

theorem resolve_changes : P.resolve G "x" ≠ P.resolve G' "x" := by decide
theorem owner_displaced : P.owner G (.page,"z") = some old ∧ P.owner G' (.page,"z") = some shadow := by decide
theorem delta_computed :
    P.deltaEdges (P.aliasEdges G) (P.affectedKeys "z" G G') G' =
    [("a","b"),("u","v"),("z","w"),("q","y")] := by decide
theorem delta_applies : ∀ e,
    e ∈ P.deltaEdges (P.aliasEdges G) (P.affectedKeys "z" G G') G' ↔ e ∈ P.aliasEdges G' :=
  (CM.LegacyHeadlines.T4_delta P lawful diffAt).1

theorem duplicate_set : SameFiles G (G.reverse ++ G) := by
  intro f
  simp
theorem duplicate_T1 : P.resolve G "x" = P.resolve (G.reverse ++ G) "x" ∧
    P.concept G "x" = P.concept (G.reverse ++ G) "x" :=
  CM.LegacyHeadlines.T1_set P lawful duplicate_set "x"

theorem demo_lawful : CM.Demo.P.Lawful := CM.Demo.lawful

/-- The page universe is graph-dependent: the empty graph has no pages; `G` has real pages and the
reference-only page `old` (the owner block at `z` mentions the unclaimed key `old`), while the
non-owner `shadow`'s block mentions `shadow`, which is not a page of `G`; after the edit `shadow`
owns `z`, so `shadow` becomes a reference-only page (owner displacement reaches the page universe). -/
theorem pages_empty : P.pagesLegacyA [] = [] := by decide
/-- Namespace closure (SPEC §3.2), evaluated by the kernel. -/
theorem nsClosure_abc : nsClosure "a/b/c" = ["a/b/c", "a", "a/b"] := by decide +kernel
theorem refOnly_before : P.refOnlyPagesLegacyA G = [(.page, "old")] := by decide +kernel
theorem pages_before : P.pagesLegacyA G = [(.page, "a"), (.page, "u"), (.page, "z"), (.page, "z"), (.page, "old")] := by decide +kernel
theorem refOnly_after : P.refOnlyPagesLegacyA G' = [(.page, "new"), (.page, "shadow")] := by decide +kernel
theorem T1_pages_witness : ∀ p, p ∈ P.pagesLegacyA (G.reverse ++ G) ↔ p ∈ P.pagesLegacyA G :=
  (CM.LegacyHeadlines.T1_pages P lawful wf.1 (fun f => (duplicate_set f).symm)).1

end Witnesses
end CM

#print axioms CM.Witnesses.lawful
#print axioms CM.Witnesses.keyLaws
#print axioms CM.Witnesses.diffAt
#print axioms CM.Witnesses.locality_hypotheses
#print axioms CM.Witnesses.locality_applies
#print axioms CM.Witnesses.resolve_changes
#print axioms CM.Witnesses.owner_displaced
#print axioms CM.Witnesses.delta_applies
#print axioms CM.Witnesses.duplicate_T1
#print axioms CM.Witnesses.demo_lawful
#print axioms CM.Witnesses.nsClosure_abc
#print axioms CM.Witnesses.pages_empty
#print axioms CM.Witnesses.pages_before
#print axioms CM.Witnesses.refOnly_after
#print axioms CM.Witnesses.T1_pages_witness
