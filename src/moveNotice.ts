import { msg } from "./i18n";
import { blockRef, node as docNode, type OutlineScope } from "./document";
import { openPageAtBlock } from "./router";
import { pushToastUnique } from "./toasts";

/** GH #641: a keyboard Move Up/Down made inside a reference, query or embed
 * surface moves the SOURCE block among its source siblings (OG parity, kept:
 * Martin 2026-10-06). When the surface cannot show the result (the moved block
 * is one of the surface's roots, whose own position is not drawn, or it left
 * every root's subtree), say where it went: a short, non-sticky "Moved in
 * <page>" notice whose action opens the block there. A move the surface does
 * show (a nested row among its siblings) and every move in an ordinary outline
 * (no `navOnly` scope) stay silent. Notice only: the move is already done.
 * Cost O(depth of the block) plus one toast scan. */
export function noticeMoveOutOfView(id: string, scope: OutlineScope | null | undefined): void {
  if (!scope?.navOnly) return;
  const moved = docNode(id);
  if (!moved) return;
  const roots = new Set(scope.roots);
  if (!roots.has(id)) {
    const seen = new Set<string>([id]);
    for (let parent = moved.parent; parent !== null && !seen.has(parent); parent = docNode(parent)?.parent ?? null) {
      if (roots.has(parent)) return; // still drawn inside the surface
      seen.add(parent);
    }
  }
  const ref = blockRef(id);
  pushToastUnique(msg("move-notice-moved-in", { page: String(ref.page) }), "info", {
    action: {
      label: msg("move-notice-show"),
      run: () => openPageAtBlock({ name: ref.page, pageKind: ref.pageKind, block: ref.uuid, ...(ref.path ? { path: ref.path } : {}) }),
    },
  });
}
