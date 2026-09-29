// Device-local query display choices are validated before they enter a route or
// session. This module does not parse a query or persist graph content.
import type { FriendlyPageMatchScope, ViewSettings, ViewKind } from "./queryIr";
import { canonicalGroupField } from "./queryViewProperties";

export type QueryDisplayDraft = Omit<ViewSettings, "view">;

/** Bounds of one draft: device-local disposable state, sized to keep a route small. */
export const QUERY_DISPLAY_MAX_LIST = 64;
export const QUERY_DISPLAY_MAX_FIELD = 512;
export const QUERY_DISPLAY_MAX_JSON = 65_536;
/** `ViewSettings.sample` is a `u32` at the bridge. */
export const QUERY_DISPLAY_MAX_SAMPLE = 4_294_967_295;

const MATCH_SCOPES: ReadonlySet<string> = new Set(["names", "content", "both"]);
/** The one reader of a Friendly page-membership mode; null is "unreadable", and a
 * caller drops the field rather than broadening membership. */
export function normalizeFriendlyPageMatchScope(value: unknown): FriendlyPageMatchScope | null {
  return typeof value === "string" && MATCH_SCOPES.has(value) ? value as FriendlyPageMatchScope : null;
}

const validField = (value: unknown, allowEmpty = false): value is string =>
  typeof value === "string" && value.length <= QUERY_DISPLAY_MAX_FIELD && (allowEmpty || value.length > 0)
  && value.trim() === value && !/[=;\0\r\n]/.test(value);

/** Validate a device-local query display draft. Returns fresh arrays or null;
 * no query text is parsed here. O(number of fields), bounded at 64 entries. */
export function normalizeQueryDisplayDraft(value: unknown): QueryDisplayDraft | null {
  if (!value || typeof value !== "object" || Array.isArray(value)) return null;
  const source = value as Record<string, unknown>;
  const draft: QueryDisplayDraft = {};
  const tuples = (item: unknown): item is [string, string][] =>
    Array.isArray(item) && item.length <= QUERY_DISPLAY_MAX_LIST && item.every((pair) => Array.isArray(pair) && pair.length === 2);
  if (source.sort !== undefined) {
    if (!tuples(source.sort) || !source.sort.every(([field, dir]) => validField(field) && ["asc", "desc"].includes(dir))) return null;
    draft.sort = source.sort.map(([field, dir]) => [field, dir as "asc" | "desc"]);
  }
  if (source.columns !== undefined) {
    if (!Array.isArray(source.columns) || source.columns.length > QUERY_DISPLAY_MAX_LIST || !source.columns.every((field) => validField(field))) return null;
    draft.columns = [...source.columns];
  }
  if (source.aggregates !== undefined) {
    if (!tuples(source.aggregates) || !source.aggregates.every(([field, fn]) =>
      validField(field, fn === "count") && ["count", "sum", "avg"].includes(fn))) return null;
    draft.aggregates = source.aggregates.map(([field, fn]) => [field, fn as "count" | "sum" | "avg"]);
  }
  if (source.group_by !== undefined) {
    // The empty string is the explicit clear; anything else must read back as a
    // canonical field, or it is a value this build cannot honour (never guessed).
    if (typeof source.group_by !== "string" || source.group_by.length > QUERY_DISPLAY_MAX_FIELD) return null;
    if (source.group_by !== "" && canonicalGroupField(source.group_by) === null) return null;
    draft.group_by = source.group_by;
  }
  if (source.sample !== undefined) {
    if (!Number.isSafeInteger(source.sample) || (source.sample as number) < 0 || (source.sample as number) > QUERY_DISPLAY_MAX_SAMPLE) return null;
    draft.sample = source.sample as number;
  }
  return JSON.stringify(draft).length <= QUERY_DISPLAY_MAX_JSON ? draft : null;
}

/** Merge a route draft with the parsed view, leaving presentation under the
 * route's sole authority. O(number of fields); never mutates either input. */
export function queryDisplaySettings(draft: QueryDisplayDraft | undefined, parsed: ViewSettings | undefined, presentation: ViewKind): ViewSettings {
  const source = draft ?? parsed ?? {};
  return {
    view: presentation,
    ...(source.sort === undefined ? {} : { sort: source.sort.map(([f, d]) => [f, d] as [string, "asc" | "desc"]) }),
    ...(source.group_by === undefined ? {} : { group_by: source.group_by }),
    ...(source.columns === undefined ? {} : { columns: [...source.columns] }),
    ...(source.aggregates === undefined ? {} : { aggregates: source.aggregates.map(([f, a]) => [f, a] as [string, "count" | "sum" | "avg"]) }),
    ...(source.sample === undefined ? {} : { sample: source.sample }),
  };
}
