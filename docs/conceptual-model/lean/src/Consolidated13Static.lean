import Claimant13
import Links
namespace CM
namespace Consolidated13
open Params
variable {K : Type} [DecidableEq K] (P : Inputs K)

def walk (U : List (Page K)) (step : Page K → Page K → Bool) : Nat → List (Page K) → List (Page K)
  | 0, X => X
  | k + 1, X => let Y := walk U step k X; Y ++ U.filter (fun T => Y.any (fun S => step S T))

end Consolidated13
end CM
