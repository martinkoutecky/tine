// Device-local query display choices are validated before they enter a route or
// session. This module does not parse a query or persist graph content.
import type { ViewSettings, ViewKind } from "./queryIr";

export type QueryDisplayDraft = Omit<ViewSettings, "view">;
const validField = (value: unknown, allowEmpty = false): value is string =>
  typeof value === "string" && value.length <= 512 && (allowEmpty || value.length > 0)
  && value.trim() === value && !/[=;\0\r\n]/.test(value);

/** Validate a device-local query display draft. Returns fresh arrays or null;
 * no query text is parsed here. O(number of fields), bounded at 64 entries. */
export function normalizeQueryDisplayDraft(value: unknown): QueryDisplayDraft | null {
  if (!value || typeof value !== "object" || Array.isArray(value)) return null;
  const source = value as Record<string, unknown>;
  const draft: QueryDisplayDraft = {};
  const tuples = (item: unknown): item is [string, string][] =>
    Array.isArray(item) && item.length <= 64 && item.every((pair) => Array.isArray(pair) && pair.length === 2);
  if (source.sort !== undefined) {
    if (!tuples(source.sort) || !source.sort.every(([field, dir]) => validField(field) && ["asc", "desc"].includes(dir))) return null;
    draft.sort = source.sort.map(([field, dir]) => [field, dir as "asc" | "desc"]);
  }
  if (source.columns !== undefined) {
    if (!Array.isArray(source.columns) || source.columns.length > 64 || !source.columns.every((field) => validField(field))) return null;
    draft.columns = [...source.columns];
  }
  if (source.aggregates !== undefined) {
    if (!tuples(source.aggregates) || !source.aggregates.every(([field, fn]) =>
      validField(field, fn === "count") && ["count", "sum", "avg"].includes(fn))) return null;
    draft.aggregates = source.aggregates.map(([field, fn]) => [field, fn as "count" | "sum" | "avg"]);
  }
  if (source.group_by !== undefined) {
    if (typeof source.group_by !== "string" || source.group_by.length > 512 || /[\0\r\n]/.test(source.group_by)) return null;
    draft.group_by = source.group_by;
  }
  if (source.sample !== undefined) {
    if (!Number.isSafeInteger(source.sample) || (source.sample as number) < 0 || (source.sample as number) > 4294967295) return null;
    draft.sample = source.sample as number;
  }
  return JSON.stringify(draft).length <= 65536 ? draft : null;
}

/** Merge a route draft with the parsed view, leaving presentation under the
 * route's sole authority. O(number of fields); never mutates either input. */
export function queryDisplaySettings(draft: QueryDisplayDraft | undefined, parsed: ViewSettings, presentation: ViewKind): ViewSettings {
  const source = draft ?? parsed;
  return {
    view: presentation,
    ...(source.sort === undefined ? {} : { sort: source.sort.map(([f, d]) => [f, d] as [string, "asc" | "desc"]) }),
    ...(source.group_by === undefined ? {} : { group_by: source.group_by }),
    ...(source.columns === undefined ? {} : { columns: [...source.columns] }),
    ...(source.aggregates === undefined ? {} : { aggregates: source.aggregates.map(([f, a]) => [f, a] as [string, "count" | "sum" | "avg"]) }),
    ...(source.sample === undefined ? {} : { sample: source.sample }),
  };
}
