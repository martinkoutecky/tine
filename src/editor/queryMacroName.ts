// **The macro names a query can be spelled with — the ONE list (SPEC §7.9, Y1).** Two spellings, one …
export const QUERY_MACRO_NAMES = ["query", "tine-query"] as const;

/** The macro name a query is SAVED under, given whether the OG printer can express it (Q3, §4.3). */
export function macroNameForDialect(ogExpressible: boolean): string {
  return ogExpressible ? QUERY_MACRO_NAMES[0] : QUERY_MACRO_NAMES[1];
}

/** The empty macro the slash menu and the visual-builder entry insert, and the caret offset that lands the … */
export const QUERY_MACRO_SCAFFOLD = `{{${QUERY_MACRO_NAMES[0]} }}`;
export const QUERY_MACRO_SCAFFOLD_CARET = QUERY_MACRO_SCAFFOLD.length - 2;
