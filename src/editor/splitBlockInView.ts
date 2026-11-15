import { node, splitBlock, type OutlineScope, type PendingEdit } from "../document";
import { editingId } from "../editorController";
import type { CollapseSurfaceApi } from "../components/Block";
import type { HidePolicy } from "./properties";

/** Enter's shared view policy for both editors. Splits after text in a result root insert a child
 * (OG editor.cljs outliner-insert-block!); a split that lands under a result row
 * is revealed in the local fold surface; zoom also keeps a head split inside its root. O(1) root checks plus
 * the existing page-sized split and local fold update; no additional IO.
 * `surface` is the editor's existing navigation scope, null on a primary view. */
export function splitBlockInView(id: string, offset: number, scope: OutlineScope | null,
  surface: string | null, hidden: HidePolicy, pending: PendingEdit | undefined, collapse: CollapseSurfaceApi | null): void {
  const zoom = scope?.forceExpandedRoot === id;
  const result = !!scope?.splitAsChild?.(id);
  splitBlock(id, offset, zoom || result, zoom, surface, hidden, pending);
  // A result group folds descendants locally (default disclosure), so a split that
  // lands under any of its rows, root or descendant, must be revealed there.
  if (scope?.splitAsChild && node(editingId() ?? "")?.parent === id) collapse?.setMany([id], false);
}
