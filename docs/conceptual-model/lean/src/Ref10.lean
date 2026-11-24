import Decided
set_option linter.unusedSectionVars false

namespace CM
namespace Ref10
open Params

variable {K : Type} [DecidableEq K] (P : Params K)

variable {P}

variable (P)

def nsParent (s : String) : Option String :=
  if P.nameIsNamespace s then (P.occClosure s).tail.getLast? else none

end Ref10
end CM
