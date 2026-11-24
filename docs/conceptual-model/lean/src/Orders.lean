import Model
namespace CM
namespace Orders

def fileCode (f : File) : List String :=
  [f.path, f.text, match f.dir with | .journals => "j" | .pages => "p",
   match f.format with | .md => "m" | .org => "o"]

def fileLe (f g : File) : Bool := decide (fileCode f ≤ fileCode g)
end Orders
end CM
