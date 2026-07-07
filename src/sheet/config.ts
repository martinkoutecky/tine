import { facetsOf } from "../render/facets";
import type { Format } from "../render/ast";

export type SheetView = "table" | "grid" | "board";

export interface SheetConfig {
  view: SheetView | null;
  groupBy: string | null;
  header: boolean;
  colWidths: ReadonlyMap<number, number>;
}

const VIEWS = new Set<SheetView>(["table", "grid", "board"]);

function parseColWidths(value: string): ReadonlyMap<number, number> {
  const out = new Map<number, number>();
  for (const part of value.split(";")) {
    const m = /^\s*(\d+)\s*=\s*(\d+)\s*$/.exec(part);
    if (!m) continue;
    out.set(Number(m[1]), Number(m[2]));
  }
  return out;
}

export function serializeColWidths(widths: ReadonlyMap<number, number>): string {
  return [...widths.entries()]
    .filter(([col, px]) => Number.isInteger(col) && col >= 0 && Number.isFinite(px) && px >= 0)
    .sort(([a], [b]) => a - b)
    .map(([col, px]) => `${col}=${Math.round(px)}`)
    .join(";");
}

export function sheetConfig(props: readonly [string, string][]): SheetConfig {
  let view: SheetView | null = null;
  let groupBy: string | null = null;
  let header = false;
  let colWidths: ReadonlyMap<number, number> = new Map();

  for (const [rawKey, rawValue] of props) {
    const key = rawKey.trim().toLowerCase();
    const value = rawValue.trim();
    if (key === "tine.view") {
      const lower = value.toLowerCase();
      view = VIEWS.has(lower as SheetView) ? (lower as SheetView) : null;
    } else if (key === "tine.group-by") {
      groupBy = value || null;
    } else if (key === "tine.header") {
      header = value.toLowerCase() === "true";
    } else if (key === "tine.col-widths") {
      colWidths = parseColWidths(value);
    }
  }

  return { view, groupBy, header, colWidths };
}

/** Sheet config straight from a block's raw text, through the ONE block-property
 *  recognizer (`facetsOf`, lsdoc-backed + memoized) — never a second `key::` /
 *  drawer line scanner here (a duplicate recognizer drifts: fence-awareness,
 *  org drawer edge cases). */
export function sheetConfigFromRaw(raw: string, format: Format): SheetConfig {
  return sheetConfig(facetsOf(raw, format).properties);
}
