import Presentation13
namespace CM
namespace Decided
open Params
variable {K : Type} [DecidableEq K] (P : Params K)

structure Config where
  favoritesPage : String
  deriving DecidableEq, Repr

end Decided
end CM
