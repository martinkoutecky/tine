import { node, splitBlock, type OutlineScope, type PendingEdit } from "../document";
import { editingId } from "../editorController";
import type { CollapseSurfaceApi } from "../components/Block";
import type { HidePolicy } from "./properties";

/** Enter's shared view policy for both editors. Splits after text in a result root insert a child
 * (OG editor.cljs outliner-insert-block!), then reveal it in the local fold
 * surface; zoom also keeps a head split inside its root. O(1) root checks plus
 * the existing page-sized split and local fold update; no additional IO.
 * `surface` is the editor's existing navigation scope, null on a primary view. */
export function splitBlockInView(id: string, offset: number, scope: OutlineScope | null,
  surface: string | null, hidden: HidePolicy, pending: PendingEdit | undefined, collapse: CollapseSurfaceApi | null): void {
  const zoom = scope?.forceExpandedRoot === id;
  const result = !!scope?.splitAsChild?.(id);
  splitBlock(id, offset, zoom || result, zoom, surface, hidden, pending);
  if (result && node(editingId() ?? "")?.parent === id) collapse?.setMany([id], false);
}
