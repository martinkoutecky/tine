/-
Evaluating the model on small graphs (`#eval`). The extractors here are toy stand-ins for lsdoc (one
property per line, blocks are lines starting with `- `, occurrences are `[[...]]` and `#word`), enough
to exercise the definitions; they are not claimed to be Tine's parser. The orders are the lawful ones
of `Orders.lean` (path, then text, dir, format; key, then kind), so `P.Lawful` holds (`Demo.lawful`)
and every headline applies to `P`.
-/
import Query3 -- Build ordering: at most two concurrent module compilers.
import Orders

namespace CM
namespace Demo
open CM

def chars (s : String) : List Char := s.toList

/-- Strip a prefix, if present. -/
def dropPrefix? (pre s : String) : Option String :=
  if pre.toList.isPrefixOf s.toList then some (String.ofList (s.toList.drop pre.length)) else none

def lines (t : String) : List String := t.splitOn "\n"

def prop? (name : String) (t : String) : Option String :=
  (lines t).findSome? (dropPrefix? (name ++ ":: "))

def splitCommas (v : String) : List String := (v.splitOn ",").map (·.trimAscii.copy) |>.filter (!·.isEmpty)

/-- `[[x]]` links and `#tag` tags in a line. -/
partial def occsOf (cs : List Char) : List (OccKind × String) :=
  match cs with
  | '[' :: '[' :: rest =>
    let inner := rest.takeWhile (· ≠ ']')
    let after := rest.dropWhile (· ≠ ']')
    (OccKind.link, String.ofList inner) :: occsOf (after.drop 2)
  | '#' :: rest =>
    let w := rest.takeWhile (fun c => c ≠ ' ' && c ≠ ',')
    (OccKind.tag, String.ofList w) :: occsOf (rest.drop w.length)
  | _ :: rest => occsOf rest
  | [] => []

def stemOf (f : File) : String :=
  let base := (f.path.splitOn "/").getLast!
  (base.splitOn ".").head!

def P : Params String where
  key s := s.trimAscii.copy.toLower
  stem := stemOf
  title _ t := prop? "title" t
  aliasDecl _ t := match prop? "alias" t with | some v => splitCommas v | none => []
  outline _ t := (lines t).filterMap (dropPrefix? "- ")
  occ _ t := occsOf t.toList
  fileLe := Orders.fileLe
  pageLe := Orders.pageLe
  refKind _ := .page
  kindPref := .page

theorem lawful : P.Lawful := ⟨Orders.fileLe_linear, Orders.pageLe_linear⟩

def pg (path text : String) : File := ⟨path, .pages, .md, text⟩

-- The chain of SPEC §4.3: A declares B, B declares X, no X file.
def fA := pg "pages/A.md" "alias:: B\n- about [[X]] and #b"
def fB := pg "pages/B.md" "alias:: X\n- B's block links [[A/sub/leaf]]"
def fC := pg "pages/C.md" "- see [[x]]"
def G1 : Graph := [fA, fB, fC]

#eval P.resolve G1 "X"            -- (page, "b"): X opens B, not A
#eval P.resolve G1 " x "          -- same, through key
#eval P.conceptKeys G1 "X"        -- the class {b, a, x}
#eval P.resolve G1 "A"            -- the real page wins
#eval P.pagesLegacyA G1                  -- real a, b, c; reference-only x and the namespace pages a/sub/leaf, a/sub
#eval P.nodesLegacyA G1
#eval P.refNames G1 ("pages/B.md", 0)   -- namespace closure: A/sub/leaf, A, A/sub
#eval P.referencesB G1 ("pages/C.md", 0) (.page, "a")  -- true: x is in A's class

-- Listing order does not matter (T1), spot check.
#eval (P.resolve G1.reverse "X", P.conceptKeys G1.reverse "X")

-- An alias page with its own file: Old has a file and is an alias of P (SPEC §4.3 consequence).
def G2 : Graph := [pg "pages/P.md" "alias:: Old\n- p", pg "pages/Old.md" "- old content"]
#eval P.resolve G2 "Old"          -- (page, "old"): opens Old
#eval P.conceptB G2 "Old" "p"     -- true: Old's references count with P

-- Two claimants: the owner is the least under `fileLe`; the other one's blocks belong to no page.
def G3 : Graph := [pg "pages/b.md" "title:: Z\n- one", pg "pages/a.md" "title:: Z\n- two"]
#eval (P.owner G3 (.page, "z")).map File.path   -- pages/a.md
#eval P.blocks G3                                -- only pages/a.md's block

-- Locality: B drops its alias. The delta-updated adjacency equals the rebuild (T4 corollary).
def G1' : Graph := [fA, pg "pages/B.md" "- B now declares nothing", fC]
#eval P.resolve G1' "X"           -- (page, "x"): reference-only now; key x's old class contained b
#eval P.affectedKeys "pages/B.md" G1 G1'
#eval (P.deltaEdges (P.aliasEdges G1) (P.affectedKeys "pages/B.md" G1 G1') G1', P.aliasEdges G1')

end Demo
end CM
