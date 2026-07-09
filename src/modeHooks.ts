type OutlineSelectionListener = (id: string) => void;
type EditingStartListener = (id: string, owner: string | null) => void;
type ModeResetListener = () => void;

const outlineSelectionListeners = new Set<OutlineSelectionListener>();
const editingStartListeners = new Set<EditingStartListener>();
const modeResetListeners = new Set<ModeResetListener>();

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
