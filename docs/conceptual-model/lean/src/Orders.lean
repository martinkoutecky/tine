/-
Lawful (linear) orders on files and pages, by an injective encoding into lexicographically ordered
lists of strings. Construction by the independent round-1 verifier (Codex, 2026-10-19,
`model-v0-lean-r1-verify.md`), brought into the project unchanged in substance.
The file order compares path, then text, then directory, then format: every field breaks ties, so it
is antisymmetric on all `File` values (`Params.Lawful` quantifies over all of them).
-/
import Model
import Std

namespace CM
namespace Orders

def fileCode (f : File) : List String :=
  [f.path, f.text, match f.dir with | .journals => "j" | .pages => "p",
   match f.format with | .md => "m" | .org => "o"]

theorem fileCode_inj (f g : File) (h : fileCode f = fileCode g) : f = g := by
  cases f with | mk p d m t =>
    cases g with | mk p' d' m' t' =>
      cases d <;> cases d' <;> cases m <;> cases m' <;> simp_all [fileCode]

def pageCode (p : Page String) : List String :=
  [p.2, match p.1 with | .journal => "j" | .page => "p"]

theorem pageCode_inj (p q : Page String) (h : pageCode p = pageCode q) : p = q := by
  obtain ⟨k, s⟩ := p
  obtain ⟨l, t⟩ := q
  cases k <;> cases l <;> simp_all [pageCode]

/-- Any injective encoding into `List String` gives a linear order. -/
theorem liftLinear {α : Type} (code : α → List String)
    (inj : ∀ a b, code a = code b → a = b) :
    Linear (fun a b => decide (code a ≤ code b)) where
  total a b := by simpa using (Std.le_total (a := code a) (b := code b))
  antisymm a b h1 h2 := inj a b (Std.le_antisymm (by simpa using h1) (by simpa using h2))
  trans a b c h1 h2 := by
    simpa using (Std.le_trans (by simpa using h1 : code a ≤ code b) (by simpa using h2 : code b ≤ code c))

def fileLe (f g : File) : Bool := decide (fileCode f ≤ fileCode g)
def pageLe (p q : Page String) : Bool := decide (pageCode p ≤ pageCode q)

theorem fileLe_linear : Linear fileLe := liftLinear fileCode fileCode_inj
theorem pageLe_linear : Linear pageLe := liftLinear pageCode pageCode_inj

end Orders
end CM
