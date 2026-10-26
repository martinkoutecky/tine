// I-4/I-12: the `filters::` value is EDN, so its structure comes from the
// shared lsdoc reader (editor/edn.ts), never from a scanner in JavaScript.
import { ednSlice, quoteEdnString, readEdn, unquoteEdnString } from "./editor/edn";

export type ReferenceFilters = Record<string, "in" | "out">;

/** OG page.cljs save-filter! writes an EDN string-keyed boolean map
 * (`true` include, `false` exclude). Anything else is unreadable: null. */
export function parseReferenceFilters(value: string): ReferenceFilters | null {
  const form = readEdn(value);
  if (!form || form.kind !== "map" || form.children.length % 2) return null;
  const bytes = new TextEncoder().encode(value);
  const result: ReferenceFilters = Object.create(null);
  for (let i = 0; i < form.children.length; i += 2) {
    const key = form.children[i], flag = ednSlice(bytes, form.children[i + 1]);
    if (key.kind !== "string" || (flag !== "true" && flag !== "false")) return null;
    // readEdn already accepted this string form, so unquoting cannot refuse.
    result[unquoteEdnString(ednSlice(bytes, key).slice(1, -1))] = flag === "true" ? "in" : "out";
  }
  return result;
}

export function writeReferenceFilters(filters: ReferenceFilters): string {
  return `{${Object.entries(filters).map(([name, state]) => `"${quoteEdnString(name)}" ${state === "in"}`).join(", ")}}`;
}
