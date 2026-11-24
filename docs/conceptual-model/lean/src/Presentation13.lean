import Consolidated13Static
set_option linter.unusedSectionVars false
namespace CM
namespace Consolidated13
open Params
variable {K L : Type} [DecidableEq K] (P : Inputs K)

inductive QueryRowKind | body | header | preamble
  deriving DecidableEq, Repr

structure QueryRowParser where
  kind : Format → String → Nat → QueryRowKind

structure QueryPresentation where
  removeBlockChildren : Bool := true
  showHeaders : Bool := false
  deriving Repr

end Consolidated13
end CM
