import { batch, createSignal } from "solid-js";
import { renderedBlocks } from "./lazyObserve";
import { clearSelection } from "./store";
import { notifyEditingStarted } from "./modeHooks";

// Where to put the caret when a block starts editing. Either a concrete offset
// (clicks, splits, most callers) OR a column descriptor for cross-block Up/Down
// navigation: land `col` chars into the target's first source line (Down) or
// last visual row (Up), resolved against the TARGET's own editor value/layout
// so hidden props, calc/annotation blocks, and soft-wrapped lines land correctly.
// Where layout is unavailable, Up falls back to the last source line.
export type CaretPos = number | { col: number; edge: "first" | "last" };

export type EndEditReason =
  | "blur"
  | "delete-block"
  | "delete-selection"
  | "drag-start"
  | "graph-switch"
  | "page-navigation"
  | "query-builder"
  | "redo"
  | "select-block"
  | "undo";

const [editingId, setEditingId] = createSignal<string | null>(null);
// Which on-screen <Block> instance owns the active editor. One block uuid can
// render in several places at once (main view + sidebar + query result); without
// this they'd all mount a textarea for the same node and fight over its value.
// null = unscoped (e.g. keyboard nav) - any instance of editingId may edit.
const [editingOwner, setEditingOwner] = createSignal<string | null>(null);
// Optional surface scope for a structural edit whose destination Block instance
// does not exist yet (for example Enter inside an embed creates a new block).
// `editingOwner` cannot name that future instance, so the caller can instead
// bind the editor to the current stable SurfaceContext key.
const [editingSurface, setEditingSurface] = createSignal<string | null>(null);
const [caretTarget, setCaretTarget] = createSignal<{ id: string; offset: CaretPos } | null>(null);

// Surface-aware edit focus. A block uuid can render in SEVERAL surfaces at once
// (the main pane and each right-sidebar item). `activeSurface` tracks which
// surface's editor currently holds the caret (noted on textarea focus). An
// UNSCOPED edit (owner=null - a split or keyboard nav) mounts an editor in EVERY
// surface that renders the new block, and each one's onMount would call focus();
// without arbitration the main pane wins and the caret "disappears" out of the
// sidebar. So we stamp the new block id with the surface that had the caret, and
// only that surface focuses it (see Block's onMount). Scoped edits (owner set on
// click) only ever mount one instance, so they need no stamp.
const [activeSurface, setActiveSurface] = createSignal<string | null>(null);
const pendingFocusSurface = new Map<string, string>();

export { activeSurface, editingId, editingOwner, editingSurface };

export function takeCaretFor(id: string): CaretPos | null {
  const t = caretTarget();
  if (t && t.id === id) {
    setCaretTarget(null);
    return t.offset;
  }
  return null;
}

/** The surface that should take the caret for `id`, or undefined for "no
 *  constraint" (single-surface edit -> focus normally). */
export function focusSurfaceFor(id: string): string | undefined {
  return pendingFocusSurface.get(id);
}

export function clearFocusSurface(id: string) {
  pendingFocusSurface.delete(id);
}

export function noteSurfaceFocused(surfaceKey: string) {
  setActiveSurface(surfaceKey);
}

export function startEditing(
  id: string,
  offset: CaretPos = 0,
  owner: string | null = null,
  surface: string | null = null,
) {
  notifyEditingStarted(id, owner);
  // Latch the block so that when editing ends its body renders eagerly (no
  // deferred raw-text placeholder frame on blur). A just-created block goes
  // straight to the editor and is never rendered through AstBody first, so without
  // this it would briefly show its raw text on blur while the IntersectionObserver
  // catches up. See AstBody / src/lazyObserve.ts (P1 lazy body).
  renderedBlocks.add(id);
  clearSelection();

  // Set the editing signals atomically. `editing()` (Block.tsx) depends on BOTH
  // editingId AND editingOwner; without batching, an unscoped nav (owner=null) from a
  // previously-clicked block (editingOwner non-null) leaves a one-flush intermediate
  // window where the target's editing() is still false - so it renders its Rendered
  // view (incl. a SCHEDULED/DEADLINE date chip) for a frame before the editor mounts.
  // In WebKitGTK that intermediate chip-bearing state drops focus, so ArrowDown/Up into
  // a scheduled block lost the caret. batch() collapses the setters into one flush so
  // the target goes Rendered->Editor directly, matching the working direct-click path.
  batch(() => {
    setCaretTarget({ id, offset });
    if (owner === null) {
      // Unscoped: pin the caret to the surface that currently has it, so the new
      // block doesn't get its focus stolen by another surface rendering the same id.
      const s = activeSurface();
      if (s) pendingFocusSurface.set(id, s);
      else pendingFocusSurface.delete(id);
    } else {
      // Scoped to one rendered instance (a click) -> exactly one editor mounts; drop
      // any stale stamp so it focuses immediately.
      pendingFocusSurface.delete(id);
    }
    setEditingId(id);
    setEditingOwner(owner);
    setEditingSurface(owner === null ? surface : null);
  });
}

export function endEdit(_reason: EndEditReason) {
  batch(() => {
    setEditingId(null);
    setEditingOwner(null);
    setEditingSurface(null);
  });
}

export function endEditForSurface(reason: EndEditReason, surfaceKey: string) {
  if (!editingId()) return;
  const active = activeSurface();
  if (!active || active === surfaceKey) endEdit(reason);
}
