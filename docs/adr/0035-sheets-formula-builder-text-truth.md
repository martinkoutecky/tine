# 0035. Sheets formula builder text truth

- **Status:** Accepted
- **Date:** 2026-07-09

## Context

Sheets formulas are stored as `tine.formula.<name>::` expression text and may be
hand-authored. The visual builder needs query-builder-style faces, but any UI
tree that becomes a second source of truth can silently rewrite formulas it
does not fully understand.

## Decision

Formula expression text remains the authoritative state. The builder parses that
text to the existing formula AST, applies immutable AST edits only for
represented faces, deparses edited ASTs through `astToExpr`, and writes the new
text back to the editor. `astToExpr` is separate from `encodeFormulaExpr`: it
prints expression text, while encoding still only protects storage lines.

Every deparsed expression must pass the structural round-trip invariant:
`parse(astToExpr(parse(s).ast)).ast == parse(s).ast`. Formula AST nodes outside
the MVP face set render as raw-expression inputs. A root expression outside the
face set uses the original editor text, not a deparsed substitute, so saving an
unrepresented formula is verbatim unless the user edits it.

## Consequences

The builder can safely expose IF/THEN/ELSE, comparison/boolean operators,
leaves, formula refs, literals, and member transforms without claiming to model
the whole expression language. Future faces can be added by expanding the
represented-shape checks, while raw faces continue to protect formulas that are
valid but not visually modeled. The cost is an explicit deparser and a required
round-trip property test whenever formula serialization changes.
