// Query display facts are read by query_parse in tine-core and written here
// through src/document. This module only encodes the same property grammar.
import type { ViewSettings } from "./queryIr";
export { normalizeQueryDisplayDraft, queryDisplaySettings } from "./queryDisplayDraft";
export type DisplayNamespace = "legacy" | "page" | "block";

/** The sheet builtins a column or grouping token may name bare; any other bare
 * name is an ordinary property. Mirrors `sheet_field_for_column` and `view.rs`. */
export const QUERY_COLUMN_BUILTINS: ReadonlySet<string> = new Set(["state", "priority", "scheduled", "deadline", "tags", "page"]);

/** The canonical `tine.group-field` grammar (mirror of `view.rs::canonical_group_field`):
 * a builtin, or `prop:`/`formula:` with a nonempty suffix, after trimming. Anything
 * else, the empty value included, is not a field this build can read back. */
export function canonicalGroupField(value: string): string | null {
  const token = value.trim();
  if (!token || /[\0\r\n]/.test(token)) return null;
  if (QUERY_COLUMN_BUILTINS.has(token)) return token;
  for (const prefix of ["prop:", "formula:"]) if (token.startsWith(prefix)) return token.length > prefix.length ? token : null;
  return null;
}
const keys = ["view", "sort", "group-field", "sample", "columns", "col-aggregates"] as const;

/** Encode one complete display choice as block properties. Empty list values
 * explicitly clear older text directives; a scoped choice also writes its
 * marker, which keeps the page and block answers independent. O(number of fields).
 * The caller owns the undo unit and the document write door. */
export function displayPropertyPatch(view: ViewSettings, scope: DisplayNamespace = "legacy"): [string, string | null][] {
  const prefix = scope === "legacy" ? "tine." : `tine.${scope}-`;
  const values = [
    scope === "legacy" && (view.view === "list" || view.view === undefined) ? null : view.view ?? "list",
    (view.sort ?? []).map(([field, dir]) => `${field} ${dir}`).join(";"),
    view.group_by ?? "",
    view.sample === undefined ? "" : String(view.sample),
    (view.columns ?? []).join(";"),
    (view.aggregates ?? []).map(([field, fn]) => field ? `${field}=${fn}` : fn).join(";"),
  ];
  const patch = keys.map((key, index): [string, string | null] =>
    [`${prefix}${key}`, scope === "legacy" && index !== 2 && values[index] === "" ? null : values[index]]);
  if (scope !== "legacy") patch.push([`${prefix}display`, "1"]);
  return patch;
}

/** Replace only query aggregate segments, keeping unsupported and table-only
 * segments byte for byte in their original positions. An unchanged recognized
 * list returns undefined so an unrelated edit cannot rewrite authored text. */
export function mergeQueryAggregateValue(raw: string | null, next: NonNullable<ViewSettings["aggregates"]>): string | null | undefined {
  const segments = raw?.split(";") ?? [];
  const parse = (segment: string): [string, "count" | "sum" | "avg"] | null => {
    const text = segment.trim();
    if (text.toLowerCase() === "count") return ["", "count"];
    const match = /^([^=]*)=(count|sum|avg)$/i.exec(text);
    return match ? [match[1].trim(), match[2].toLowerCase() as "count" | "sum" | "avg"] : null;
  };
  const parsed = segments.map(parse);
  const owned = parsed.filter((entry): entry is NonNullable<typeof entry> => entry !== null);
  if (JSON.stringify(owned) === JSON.stringify(next)) return undefined;
  let taken = 0;
  const out: string[] = [];
  for (let i = 0; i < segments.length; i++) {
    if (parsed[i]) {
      if (taken < next.length) {
        const [field, fn] = next[taken];
        out.push(field ? `${field}=${fn}` : fn);
      }
      taken++;
    } else if (segments[i].trim()) out.push(segments[i]);
  }
  for (; taken < next.length; taken++) {
    const [field, fn] = next[taken];
    out.push(field ? `${field}=${fn}` : fn);
  }
  return out.length ? out.join(";") : null;
}

/** A pre-split `tine.fields` value is a bare nonempty column list. Typed or
 * mixed schema text contains `=` and must never be removed by Display. */
export function isLegacyBareColumnList(value: string | null): boolean {
  if (value === null) return false;
  const tokens = value.split(";").map((part) => part.trim()).filter(Boolean);
  return tokens.length > 0 && tokens.every((token) => !/[=\0\r\n]/.test(token));
}

/** Write one independent page or block display override. An absent display
 * removes its recognized facts; an empty display keeps only the marker. Unknown
 * properties and unsupported aggregate segments are preserved. */
export function queryScopedDisplayPropertyPatch(input: {
  scope: "page" | "block";
  presentation?: ViewSettings["view"];
  display?: Omit<ViewSettings, "view">;
  properties: readonly (readonly [string, string])[];
}): [string, string | null][] {
  const prefix = `tine.${input.scope}-`;
  const current = (key: string) => input.properties.find(([name]) => name.toLowerCase() === key)?.[1];
  const writes: [string, string | null][] = [];
  const set = (name: string, value: string | null | undefined) => {
    const key = `${prefix}${name}`;
    const prior = current(key);
    if (value === undefined) { if (prior !== undefined) writes.push([key, null]); }
    else if (prior !== value) writes.push([key, value]);
  };
  set("view", input.presentation);
  set("display", input.display === undefined ? undefined : "1");
  const display = input.display;
  set("sort", display?.sort === undefined ? undefined : display.sort.map(([f, d]) => `${f} ${d}`).join(";"));
  set("group-field", display?.group_by);
  set("columns", display?.columns === undefined ? undefined : display.columns.join(";"));
  const aggregateKey = `${prefix}col-aggregates`;
  const rawAggregates = current(aggregateKey);
  const merged = mergeQueryAggregateValue(rawAggregates ?? null, display?.aggregates ?? []);
  if (display?.aggregates !== undefined) {
    if (merged !== undefined || rawAggregates === undefined)
      writes.push([aggregateKey, merged ?? (rawAggregates === undefined ? "" : null)]);
  } else if (merged !== undefined) writes.push([aggregateKey, merged]);
  else if (rawAggregates !== undefined && rawAggregates.trim() === "") writes.push([aggregateKey, null]);
  set("sample", display?.sample === undefined ? undefined : String(display.sample));
  return writes;
}
