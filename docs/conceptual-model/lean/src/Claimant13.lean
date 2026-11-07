/- Literal claims, fixed kind preference and unsigned UTF-8 path order. -/
import Witness12

namespace CM
namespace Consolidated13

structure Inputs (K : Type) extends CM.Params K
instance {K : Type} : CoeOut (Inputs K) (CM.Params K) := ⟨Inputs.toParams⟩
def Inputs.ofParams {K : Type} (P : CM.Params K) : Inputs K := ⟨P⟩

variable {K : Type} [DecidableEq K] (P : Inputs K)

/-- Kind preference only resolves cross-kind conflicts; path bytes order twins.
The full-file suffix totalizes invalid equal-path inputs. -/
def claimantLe : File → File → Bool :=
  Owner5.rankLe (fun f => [if Params.kindOf f == P.kindPref then 0 else 1])
    (Owner5.rankLe (fun f => Owner5.bytes f.path) Orders.fileLe)

theorem claimantLe_linear : Linear (claimantLe P) :=
  Owner5.rankLe_linear _ (Owner5.rankLe_linear _ Orders.fileLe_linear)

theorem claimantLe_distinct_paths (f g f' g' : File)
    (hf : f'.path = f.path) (hg : g'.path = g.path) (hne : f.path ≠ g.path)
    (hfd : f'.dir = f.dir) (hgd : g'.dir = g.dir) :
    claimantLe P f' g' = claimantLe P f g := by
  have hn : Owner5.bytes f.path ≠ Owner5.bytes g.path := fun h =>
    hne (Owner5.bytes_injective h)
  simp [claimantLe, Owner5.rankLe, Params.kindOf, hf, hg, hfd, hgd, hn]

end Consolidated13
end CM
