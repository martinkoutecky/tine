import Model
set_option linter.unusedSectionVars false

namespace CM
namespace Params

variable {K : Type} [DecidableEq K] (P : Params K)

end Params

inductive Node (K : Type) | page (p : Page K) | block (b : BlockId)
  deriving DecidableEq, Repr

namespace Params

variable {K : Type} [DecidableEq K] (P : Params K)

variable {P}

end Params
end CM
