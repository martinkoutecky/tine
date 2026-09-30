/// **The macro names a query can be spelled with — the ONE list (SPEC §7.9, Y1).**
///
/// Two spellings, one meaning: `{{query …}}` is the legacy OG DSL macro every
/// existing graph already contains, and `{{tine-query …}}` is the TQL macro the
/// printer writes when a filter is not OG-expressible (Q3). Both are queries and
/// every neighbour that RECOGNISES or WRITES one must read this list rather than
/// spelling a name inline: a packet may not write a macro name its own tree
/// cannot render, re-edit or export (Y1), and the way that used to happen was a
/// second `/\{\{query\b/` regex somewhere the first author never looked.
///
/// The frontend authoring list in `src/editor/queryMacroName.ts` is pinned to
/// this literal by `src/queryMacroNameConsistency.test.ts`. Recognition and
/// extent reading compile the same Rust source into native and wasm (I-12).
///
/// **Order is not semantics.** Readers must match the LONGEST candidate rather
/// than the first, so that reordering this array can never change which macro a
/// document scan recognises (`macro_text::macro_at`).
pub const QUERY_MACRO_NAMES: [&str; 2] = ["query", "tine-query"];
