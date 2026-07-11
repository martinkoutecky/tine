import { createContext } from "solid-js";

export interface SheetCellCtx {
  gridId: string;
  /** Render-surface identity (pane / containing sheet). Omitted by legacy callers. */
  surfaceId?: string;
  /** Stable row identity for sorted/query-backed tables. */
  rowId?: string;
  /** Stable column/group identity for boards where one row can appear in multiple columns. */
  columnId?: string;
  row: number;
  col: number;
}

export const SheetCellContext = createContext<SheetCellCtx | null>(null);
