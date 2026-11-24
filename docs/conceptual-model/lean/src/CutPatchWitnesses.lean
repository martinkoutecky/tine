import CutPatch
import CutSettingsExamples

namespace CM
namespace CutExamples

open Cut
open ListedDisk (Binding Effect Path)

private def yieldsPatch (O : Observations) (before : Graph) (E : Engine) (r : Request)
    (effects : List (Path × Effect)) (bindings : List Binding)
    (checkPlan : Plan → Bool := fun _ => true) : Bool :=
  match decideRequest O asciiLower (fun _ => false) dates E before r with
  | .success d =>
    let patch := patchOf before d
    decide (patch.effects.toList = effects ∧ patch.bindings = bindings) && checkPlan d.plan
  | _ => false

private def markdown (leaf text : String) : File := ⟨"pages/" ++ leaf ++ ".md", .pages, .md, text⟩

private def oldPage : File := markdown "Old" "- notes\n"

theorem same_name_empty_patch :
    yieldsPatch o [oldPage] ⟨fun _ => some [oldPage]⟩ ⟨"Old", "Old", none, true⟩ [] [] = true := by
  decide +kernel

theorem merge_removed_source_patch :
    yieldsPatch o plain_merge_before plain_merge_engine ⟨"A", "B", some .confirm, true⟩
      [("pages/A.md", ⟨some (bytes "- source\n"), none⟩),
       ("pages/B.md", ⟨some (bytes "- target\n"), some (bytes "- target\n- source\n")⟩)]
      [⟨"pages/B.md", "pages/B.md"⟩, ⟨"pages/A.md", "pages/B.md"⟩] = true := by
  decide +kernel

theorem respelling_only_patch :
    yieldsPatch o [oldPage] ⟨fun _ => some [markdown "old" "- notes\n"]⟩ ⟨"Old", "old", none, true⟩
      [("pages/Old.md", ⟨some (bytes "- notes\n"), none⟩), ("pages/old.md", ⟨none, some (bytes "- notes\n")⟩)]
      [⟨"pages/Old.md", "pages/old.md"⟩] = true ∧
    o.foldedPath "pages/Old.md" = o.foldedPath "pages/old.md" := by
  decide +kernel

private def namespaceObservations : Observations :=
  { o with stem := fun f => if f.path = "pages/A%2Fx.md" then "A/x" else o.stem f }

private def namespaceBefore : Graph :=
  [markdown "A%2Fsub" "- notes\n", markdown "A%2Fx" "- source\n", markdown "R" "- [[A]]\n"]

private def namespaceAfter : Graph :=
  [markdown "B%2Fsub" "- notes\n", markdown "B%2Fx" "- source\n", markdown "R" "- [[B]]\n"]

private def namespaceEffects : List (Path × Effect) :=
  [("pages/A%2Fsub.md", ⟨some (bytes "- notes\n"), none⟩),
   ("pages/A%2Fx.md", ⟨some (bytes "- source\n"), none⟩),
   ("pages/B%2Fsub.md", ⟨none, some (bytes "- notes\n")⟩),
   ("pages/B%2Fx.md", ⟨none, some (bytes "- source\n")⟩),
   ("pages/R.md", ⟨some (bytes "- [[A]]\n"), some (bytes "- [[B]]\n")⟩)]

private def namespaceBindings : List Binding :=
  [⟨"pages/A%2Fsub.md", "pages/B%2Fsub.md"⟩, ⟨"pages/A%2Fx.md", "pages/B%2Fx.md"⟩,
   ⟨"pages/R.md", "pages/R.md"⟩]

theorem namespace_two_moves_and_rewrite_patch :
    yieldsPatch namespaceObservations namespaceBefore ⟨fun _ => some namespaceAfter⟩ ⟨"A", "B", none, true⟩
      namespaceEffects namespaceBindings = true := by
  decide +kernel

theorem fileless_reference_merge_patch :
    yieldsPatch o fileless_merge_before fileless_merge_engine ⟨"A", "B", some .confirm, true⟩
      [("pages/R.md", ⟨some (bytes "- [[A]]\n"), some (bytes "- [[B]]\n")⟩)]
      [⟨"pages/R.md", "pages/R.md"⟩] (fun p => p.source.isNone && p.survivor.isSome) = true := by
  decide +kernel

private def targetPage : File := markdown "B" "- target\n"

theorem fileless_namespace_merge_moves_descendants :
    yieldsPatch namespaceObservations (namespaceBefore ++ [targetPage])
      ⟨fun _ => some (namespaceAfter ++ [targetPage])⟩ ⟨"A", "B", some .confirm, true⟩
      namespaceEffects namespaceBindings (fun p => p.source.isNone && p.survivor.isSome) = true := by
  decide +kernel

theorem merge_unchanged_survivor_binding :
    yieldsPatch o [markdown "A" "", targetPage] ⟨fun _ => some [targetPage]⟩ ⟨"A", "B", some .confirm, true⟩
      [("pages/A.md", ⟨some [], none⟩)]
      [⟨"pages/B.md", "pages/B.md"⟩, ⟨"pages/A.md", "pages/B.md"⟩] = true := by
  decide +kernel

end CutExamples
end CM
