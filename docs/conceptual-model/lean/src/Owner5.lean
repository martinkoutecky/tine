import Orders
set_option linter.unusedSectionVars false
set_option maxRecDepth 8192
set_option maxHeartbeats 1600000

namespace CM
namespace Owner5
open Params

def bytes (s : String) : List Nat := s.toUTF8.data.toList.map UInt8.toNat

def rankLe {α : Type} (rank : α → List Nat) (le : α → α → Bool) (a b : α) : Bool :=
  decide (rank a ≤ rank b) && (if rank a = rank b then le a b else true)

variable {K : Type} [DecidableEq K]

end Owner5
end CM
