// One expansion budget for user `:macros` (and, in rendered text, resolved
// block refs), shared by the DOM renderer (inline.tsx `UserMacroView`) and the
// Copy/Export rendered-text walk (renderedText.ts). I-22: a depth cap bounds
// nesting but not branching — a synced config.edn entry such as
// `{:m "{{m}}{{m}}{{m}}…"}` expands 10^12 times within depth 12 and hangs the
// render thread on every launch that shows it (og C3 L16). The budget bounds
// the total expansions and expanded bytes of ONE top-level expansion tree;
// past it the caller shows a visible "macro expansion limit" marker instead.

export const MAX_MACRO_EXPANSION_DEPTH = 12;
export const MAX_MACRO_EXPANSIONS = 1000;
export const MAX_MACRO_EXPANDED_BYTES = 256 * 1024;
export const MACRO_EXPANSION_LIMIT_LABEL = "macro expansion limit";

export interface ExpansionGate {
  /** Run `render` as one expansion of `bytes` expanded bytes. Past the depth
   *  cap returns `onDepth()`; past the tree's budget returns `onBudget()`.
   *  Nested calls made synchronously inside `render` share the tree budget. */
  expand<T>(bytes: number, onDepth: () => T, onBudget: () => T, render: () => T): T;
  /** Current nesting depth (0 outside any expansion). */
  depth(): number;
}

export function createExpansionGate(): ExpansionGate {
  let depth = 0;
  let expansions = 0;
  let expanded = 0;
  return {
    expand(bytes, onDepth, onBudget, render) {
      if (depth >= MAX_MACRO_EXPANSION_DEPTH) return onDepth();
      if (depth === 0) {
        expansions = 0;
        expanded = 0;
      }
      expansions++;
      expanded += bytes;
      if (expansions > MAX_MACRO_EXPANSIONS || expanded > MAX_MACRO_EXPANDED_BYTES) return onBudget();
      depth++;
      try {
        return render();
      } finally {
        depth--;
      }
    },
    depth: () => depth,
  };
}
