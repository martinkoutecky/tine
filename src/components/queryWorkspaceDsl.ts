import { formFamilyForMacroName, queryMacroExtent } from "../editor/queryMacro";

/** Bare Search DSL keeps its OG meaning. A whole native macro envelope carries
 * the dialect when a query block opens Search, without a new persisted key. */
export function queryWorkspaceDsl(source: string) {
  const text = source.trim();
  const extent = queryMacroExtent(text);
  if (extent?.start === 0 && extent.end === text.length) {
    const tql = formFamilyForMacroName(extent.name) === "tql";
    return { argument: extent.argument, dialect: tql ? "macro_tql" as const : "macro_query" as const,
      macro: text, wrapped: true, name: extent.name };
  }
  return { argument: text, dialect: "macro_query" as const, macro: `{{query ${text}}}`, wrapped: false, name: "query" };
}
