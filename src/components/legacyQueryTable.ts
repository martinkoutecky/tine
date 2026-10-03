// OG's `{:table-view? true}` result table reads three properties of the HOST block
// (components/query_table.cljs:61-109, 163-179): `query-properties:: [:block :page :k]`
// (column set and order), `query-sort-by:: k` and `query-sort-desc:: false`.
// OG parity (Discussion #617 audit #9); the same keys a Logseq graph already carries.
import { ednSlice, readEdn } from "../editor/edn";

/** OG's key normalisation for property columns (`block.cljs` property keys). */
export const columnKey = (name: string): string => name.trim().replace(/^:/, "").toLowerCase().replace(/_/g, "-");

/**
 * The columns `query-properties` names, in order, normalised; `null` when the
 * property is absent or not a readable EDN vector (OG then derives them from the
 * result, `get-keys`). An empty vector also means "derive", like OG's `(seq ...)`.
 */
export function hostColumns(raw: string | null): string[] | null {
  if (!raw) return null;
  const form = readEdn(raw);
  if (!form || form.kind !== "vector") return null;
  const names = form.children.map((child) => columnKey(ednSlice(raw, child)));
  return names.length > 0 ? [...new Set(names)] : null;
}

export interface TableSort { column: string; desc: boolean }

/** OG `get-sort-state`: no column means unsorted; `desc` is true unless `false`. */
export function hostSort(sortBy: string | null, sortDesc: string | null): TableSort | null {
  const column = sortBy ? columnKey(sortBy) : "";
  if (!column) return null;
  return { column, desc: sortDesc === null ? true : sortDesc.trim() !== "false" };
}

/** OG `locale-compare`: numbers numerically, everything else natural-numeric by locale. */
export function compareCells(a: string, b: string): number {
  const x = Number(a);
  const y = Number(b);
  if (a.trim() !== "" && b.trim() !== "" && Number.isFinite(x) && Number.isFinite(y)) return x - y;
  return a.localeCompare(b, undefined, { numeric: true });
}
