import CutExamples
namespace CM
namespace CutExamples
open Cut
/-- A block attached to a structural header has a native header owner.
Pre-block text is already an ordinary first block in the observations. -/
theorem reader_ownership_law : nativeTable.all (fun x =>
    let c := x.2.2
    !(c.blocks.any Block.underHeader) || !c.header.isEmpty) = true := by
  decide +kernel
end CutExamples
end CM

namespace CM
namespace CutExamples
open Cut
/-- All finite native header observations agree with their key site when present. -/
theorem reader_key_field_law : nativeTable.all (fun x => x.2.2.header.all (fun h =>
    (h.role != .property || h.key.isSome) &&
    (match (textSites h.text).find? (fun s => s.kind == .propertyKey) with
    | some s => h.key == some s.name
    | none => true))) = true := by
  decide +kernel

/-- Built-in keys remain distinct observations despite having no name sites. -/
theorem builtin_keys_observed :
    ((o.read .md "icon:: star\npublic:: true\n- source\n").header.map Header.key =
      [some "icon", some "public"]) ∧
    ((o.read .md "icon:: star\npublic:: true\n- source\n").header.all (fun h =>
      (textSites h.text).all (fun s => s.kind != .propertyKey)) = true) := by
  decide +kernel
end CutExamples
end CM
