import { createContext } from "solid-js";

export interface SheetCellCtx {
  gridId: string;
  row: number;
  col: number;
}

export const SheetCellContext = createContext<SheetCellCtx | null>(null);
