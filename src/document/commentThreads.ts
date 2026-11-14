// Thread membership is shared by body structure, navigation and pointer paths.
// OG sees ordinary child bullets on disk; in Tine every descendant of a comment
// belongs to that annotation, even when an external editor nested it by hand.
import { doc, formatForBlock, blockIsOpaqueSheetView } from "./model";
import { isCommentBlock, COMMENT_DATE_KEY } from "../comments";
import { facetsOf } from "../render/facets";
import type { Format } from "../types";

// Screen only the key; the parser owns which separators/layout are accepted.
const MAY_QUOTE = /tine\.quote/i;

/** A quoted comment block, as distinct from an unquoted reply in its thread. */
export function isCommentId(id: string, format: Format = formatForBlock(id)): boolean {
  const n = doc.byId[id];
  return !!n && MAY_QUOTE.test(n.raw) && isCommentBlock(facetsOf(n.raw, format).properties, n.parent !== null);
}

/** The comment whose thread holds `id`: itself or its outermost quoted comment
 * ancestor (DESIGN §2: a quoted reply stays in its root's thread), or null for
 * an ordinary block. This is the one predicate for thread membership. O(depth). */
export function threadRootOf(id: string, format: Format = formatForBlock(id)): string | null {
  return readThreadRoot(id, format);
}

type ThreadBatch = { roots: Map<string, string | null>; path: string[] };

function readThreadRoot(id: string, format: Format, batch?: ThreadBatch): string | null {
  let root: string | null = null;
  for (let at: string | null = id; at !== null; at = doc.byId[at]?.parent ?? null) {
    const known = batch?.roots.get(at);
    if (known !== undefined) { root = known ?? root; break; }
    batch?.path.push(at);
    if (isCommentId(at, format)) root = at;
  }
  if (batch) {
    let inherited = root;
    for (const at of batch.path) {
      batch.roots.set(at, inherited);
      if (at === inherited) inherited = null;
    }
    batch.path.length = 0;
  }
  return root;
}

/** Thread membership for rows in any order, using each block's page format.
 * Includes their source ancestors in the returned map. O(rows + distinct
 * ancestors), no I/O; answers belong to this call and are not retained globally. */
export function threadRootsOf(ids: readonly string[]): Map<string, string | null> {
  const batch: ThreadBatch = { roots: new Map(), path: [] };
  for (const id of ids) readThreadRoot(id, formatForBlock(id), batch);
  return batch.roots;
}

/** A dated thread opts into message outlines. Presence, rather than validity of
 * the display date, is the file's discriminator. O(depth), no I/O. */
export function datedThreadRoot(id: string): string | null {
  const root = threadRootOf(id);
  return root && hasMessageDate(root) ? root : null;
}

/** Read the message marker from parser facets, including hand-written dates. */
export function hasMessageDate(id: string): boolean {
  const n = doc.byId[id];
  return !!n && facetsOf(n.raw, formatForBlock(id)).properties.some(([key]) => key.toLowerCase() === COMMENT_DATE_KEY);
}

/** The editable outline containing this block: null for the main outline,
 * a message id for its undated body, undefined for annotation text itself.
 * Structural gestures may not cross this boundary. O(depth), no I/O. */
export function bodyScopeOf(id: string): string | null | undefined {
  const root = threadRootOf(id);
  if (!root) return null;
  if (!hasMessageDate(root) || hasMessageDate(id)) return undefined;
  for (let at = doc.byId[id]?.parent; at; at = doc.byId[at]?.parent) {
    if (hasMessageDate(at)) return at;
  }
  return undefined;
}

/** Dated replies in file pre-order, including messages under a folded body.
 * O(thread blocks), no I/O; legacy threads have no message projection. */
export function commentMessageIds(root: string): string[] {
  if (datedThreadRoot(root) !== root) return [];
  const ids: string[] = [];
  const visit = (id: string) => {
    for (const child of doc.byId[id]?.children ?? []) {
      if (hasMessageDate(child)) ids.push(child);
      visit(child);
    }
  };
  visit(root);
  return ids;
}

/** The drawn order of a dated thread: each message, its visible ordinary body,
 * then the next message. O(thread blocks) per navigation gesture;
 * caller supplies occurrence folds when the view overrides stored folds. */
export function commentThreadVisibleOrder(root: string, folded: (id: string) => boolean): string[] {
  const order: string[] = [];
  const body = (id: string) => {
    order.push(id);
    if (!folded(id) && !blockIsOpaqueSheetView(id)) {
      for (const child of doc.byId[id]?.children ?? []) if (!hasMessageDate(child)) body(child);
    }
  };
  body(root);
  if (!folded(root)) for (const id of commentMessageIds(root)) body(id);
  return order;
}
