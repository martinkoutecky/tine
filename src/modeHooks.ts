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

/**
 * The graph was REBOUND in place: the backend reopened it, and paths resolved
 * against the old binding may no longer be valid. Distinct from a graph switch
 * (which resets the store) and from `bumpGraphEpoch` (a repaint signal).
 *
 * It lives HERE, in a module with no imports, because `persistence` and `ui`
 * import each other's neighbourhood: registering from `persistence` at module
 * scope ran while `ui` was still evaluating and hit the listener set in its
 * temporal dead zone, taking out fourteen suites at import time.
 * (GH #254 increment 3, round 13.)
 */
type GraphReboundListener = () => void;
const graphReboundListeners = new Set<GraphReboundListener>();
export function onGraphRebound(fn: GraphReboundListener): () => void {
  graphReboundListeners.add(fn);
  return () => graphReboundListeners.delete(fn);
}
export function notifyGraphRebound(): void {
  for (const fn of [...graphReboundListeners]) fn();
}
