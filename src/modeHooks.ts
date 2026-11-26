import type { SheetCellCtx } from "./sheet/context";

type OutlineSelectionListener = (id: string) => void;
type EditingStartListener = (id: string, owner: string | null) => void;
type ModeResetListener = () => void;

const outlineSelectionListeners = new Set<OutlineSelectionListener>();
const editingStartListeners = new Set<EditingStartListener>();
const modeResetListeners = new Set<ModeResetListener>();
let clearOutlineSelection: (() => void) | null = null;

export function installClearOutlineSelection(fn: () => void): void { clearOutlineSelection = fn; }
export function notifyClearOutlineSelection(): void { clearOutlineSelection?.(); }

// The grid module answers which cell continues a restarted edit (`resumeInCell`);
// the controller cannot import `sheet/`.
type CellResume = (id: string, cell: SheetCellCtx) => { owner: string | null } | null;
let cellResume: CellResume | null = null;
export function installCellResume(fn: CellResume): void { cellResume = fn; }
export function resumeCellOwner(id: string, cell: SheetCellCtx): { owner: string | null } | null { return cellResume?.(id, cell) ?? null; }

export function registerOutlineSelectionListener(fn: OutlineSelectionListener): () => void {
  outlineSelectionListeners.add(fn);
  return () => outlineSelectionListeners.delete(fn);
}

export function registerEditingStartListener(fn: EditingStartListener): () => void {
  editingStartListeners.add(fn);
  return () => editingStartListeners.delete(fn);
}

export function registerModeResetListener(fn: ModeResetListener): () => void {
  modeResetListeners.add(fn);
  return () => modeResetListeners.delete(fn);
}

export function notifyOutlineSelectionStarted(id: string): void {
  for (const fn of outlineSelectionListeners) fn(id);
}

export function notifyEditingStarted(id: string, owner: string | null): void {
  for (const fn of editingStartListeners) fn(id, owner);
}

export function notifyModeReset(): void {
  for (const fn of modeResetListeners) fn();
}
