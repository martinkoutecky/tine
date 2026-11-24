import Query
set_option linter.unusedSectionVars false

namespace CM
namespace Params

variable {K : Type} [DecidableEq K] (P : Params K)

inductive NearDir | out | inn | both
  deriving DecidableEq, Repr

variable {P}

end Params
end CM
