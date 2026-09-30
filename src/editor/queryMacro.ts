// The ONE frontend answer to "is this a query macro, and where does it sit in the raw source" (SPEC §4.3.1, …

import { query_macro_extents_json, is_query_macro_name, query_macro_is_tql } from "../render/wasm/lsdoc_wasm.js";
import { QUERY_MACRO_NAMES } from "./queryMacroName";
export { QUERY_MACRO_SCAFFOLD } from "./queryMacroName";

export { QUERY_MACRO_NAMES };

/** Which grammar's literals protect a delimiter while scanning FORM text. */
export type FormFamily = "edn" | "tql";

/** The family a macro NAME implies: `query` carries OG or advanced text, `tine-query` carries TQL (§7.1). */
export function formFamilyForMacroName(name: string): FormFamily {
  return query_macro_is_tql(name) ? "tql" : "edn";
}

/** Whether `name` is one of the query macro names, case-insensitively and as a
*  WHOLE token — `{{query-foo}}` is not a query (§7.9). */
export function isQueryMacroName(name: string): boolean {
  return is_query_macro_name(name);
}

/** One query macro in the original raw source; offsets are UTF-16 indices. */
export interface MacroExtent { start: number; end: number; name: string; argument: string }

/** The first query macro in raw, or null. O(raw bytes), via the native reader. */
export function queryMacroExtent(raw: string): MacroExtent | null {
  return queryMacroExtents(raw)[0] ?? null;
}

/** Every query macro in source order. The native reader owns recognition;
 * this boundary converts byte coordinates in one forward pass, O(raw bytes). */
export function queryMacroExtents(raw: string): MacroExtent[] {
  const found = JSON.parse(query_macro_extents_json(raw)) as MacroExtent[];
  let byte = 0, unit = 0;
  const toUnits = (target: number) => {
    while (byte < target && unit < raw.length) {
      const cp = raw.codePointAt(unit)!;
      byte += cp <= 0x7f ? 1 : cp <= 0x7ff ? 2 : cp <= 0xffff ? 3 : 4;
      unit += cp > 0xffff ? 2 : 1;
    }
    return unit;
  };
  return found.map((extent) => ({ ...extent, start: toUnits(extent.start), end: toUnits(extent.end) }));
}

/** Recover the sole authored macro from an entire-block render. A property
 *  line or leading whitespace can move its raw offset; refuse ambiguity. */
export function singleQueryMacroExtent(raw: string, displayed: MacroExtent): MacroExtent | undefined {
  const extents = queryMacroExtents(raw);
  const extent = extents.length === 1 ? extents[0] : undefined;
  return extent?.name === displayed.name && extent.argument === displayed.argument ? extent : undefined;
}

const UTF8_ENCODER = new TextEncoder();

/** The extent a parsed macro node's SPAN points at, or null. */
export function queryMacroExtentAtSpan(
  raw: string,
  span: readonly [number, number] | undefined,
): MacroExtent | null {
  if (span === undefined || span[0] < 2) return null;
  const trimmed = raw.trimStart();
  const leadBytes = UTF8_ENCODER.encode(raw.slice(0, raw.length - trimmed.length)).length;
  const wanted = span[0] - 2 + leadBytes;
  for (const extent of queryMacroExtents(raw)) {
    if (UTF8_ENCODER.encode(raw.slice(0, extent.start)).length === wanted) return extent;
  }
  return null;
}
