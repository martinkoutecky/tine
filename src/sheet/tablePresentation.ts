import type { FieldId } from "./fields";
import type { SortDir } from "../editor/queryIr";

export type SortState = { col: number; dir: 1 | -1 } | null;
export type SortKey = { kind: "number"; value: number; text: string } | { kind: "text"; text: string };
export type SchemaMenuType = "text" | "number" | "date" | "datetime" | "checkbox" | "list" | "ref";
export const SCHEMA_PROP_TYPES: SchemaMenuType[] = ["text", "number", "date", "datetime", "checkbox", "list", "ref"];

/** Keep live column widths fixed while a drag or selection changes cells.
 * O(columns); null means the browser has not measured a usable grid yet. */
export function measuredGridTracks(grid: HTMLElement, count: number): string | null {
  const cells = [...grid.children].filter((child): child is HTMLElement =>
    child instanceof HTMLElement && child.classList.contains("sheet-cell"));
  const tracks: string[] = [];
  for (const cell of cells.slice(0, count)) {
    const width = cell.getBoundingClientRect().width;
    if (width <= 0) return null;
    tracks.push(`${Math.round(width)}px`);
  }
  return tracks.length === count ? tracks.join(" ") : null;
}

/** Compare one table sort key, using numeric order only when both are numeric. */
export function compareSortKeys(a: SortKey, b: SortKey): number {
  if (a.kind === "number" && b.kind === "number") return a.value - b.value;
  return a.text.localeCompare(b.text);
}

/** Query sort property spelling for a table field. Null means this field can
 * only be sorted locally because the query engine cannot persist its order. */
export function querySortFieldName(field: FieldId | "title"): string | null {
  if (["priority", "page", "scheduled", "deadline"].includes(field)) return field;
  if (field.startsWith("prop:")) {
    const name = field.slice(5);
    return name && !/[;=\0\r\n]/.test(name) ? name : null;
  }
  return null;
}

/** Interpret a saved query column token as a sheet field. O(1), pure. */
export function queryColumnFieldId(name: string): FieldId {
  return (["state", "priority", "scheduled", "deadline", "tags", "page"].includes(name)
    ? name : `prop:${name}`) as FieldId;
}

/** Spell a sheet field in a saved query column list. Computed fields and a
 * property shadowed by a builtin cannot round-trip, so return null. */
export function queryColumnName(field: FieldId): string | null {
  if (!field.startsWith("prop:")) return field.startsWith("formula:") ? null : field;
  const name = field.slice(5);
  return name && !/[=;\0\r\n]/.test(name) &&
    !["state", "priority", "scheduled", "deadline", "tags", "page"].includes(name) ? name : null;
}

/** Reorder all visible query fields, or refuse if any cannot be written as a
 * complete `tine.columns` list. O(columns); never drops a computed column. */
export function reorderedQueryColumns(fields: readonly FieldId[], from: FieldId, target: FieldId, before: boolean): string[] | null {
  const order = [...fields];
  const source = order.indexOf(from);
  if (source < 0) return null;
  order.splice(source, 1);
  const at = order.indexOf(target);
  order.splice(at < 0 ? order.length : at + (before ? 0 : 1), 0, from);
  const names = order.map(queryColumnName);
  return names.every((name): name is string => name !== null) ? names : null;
}

/** Ascending, descending, then cleared for a saved single-key sort. */
export function nextQuerySort(current: [string, SortDir][] | undefined, field: string): [string, SortDir][] {
  if (current?.length !== 1 || current[0][0] !== field) return [[field, "asc"]];
  return current[0][1] === "asc" ? [[field, "desc"]] : [];
}
