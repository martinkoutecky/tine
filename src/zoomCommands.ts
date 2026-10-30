import { msg } from "./i18n";
import { node as docNode } from "./document";
import { captureHistoryEditorContext, editingId, startEditing } from "./editorController";
import { commandTargetIds } from "./blockRefActions";
import { focusedRouter, mainRouter, paneRouter } from "./panes";
import { resolveRouteBlock, type PaneRouter } from "./router";
import { pushToast } from "./toasts";

// GH #799. Keyboard twins of the bullet click (zoom in) and the breadcrumb
// (zoom out one level). They reuse the router's one zoom path, `focusBlock`:
// zoom is route state, so it joins the pane's back/forward history, and it
// never writes the graph. OG's `:editor/zoom-in` / `:editor/zoom-out`
// (handler/editor.cljs zoom-in! / zoom-out!) are the reference; both start
// unbound here because OG's non-Mac chords (alt+right / alt+left) are Tine's
// Forward / Back and OG's Mac zoom-out (mod+,) is Tine's Settings.

/** The pane a zoom acts on: the pane whose surface holds the editor, else the
 *  focused pane (a sidebar occurrence, like its bullet, zooms the focused pane). */
function zoomRouter(surface: string | null | undefined): PaneRouter {
  if (surface === "main") return mainRouter();
  const pane = surface?.startsWith("pane:") ? surface.slice(5) : null;
  return pane ? paneRouter(pane) : focusedRouter();
}

/** The editing-surface key of a pane (Page.tsx uses the same static keys). */
function paneSurface(router: PaneRouter): string {
  return router.paneId === "main" ? "main" : `pane:${router.paneId}`;
}

function keepEditing(id: string, offset: number, router: PaneRouter): void {
  // OG re-opens the editor on the block after the page navigation (setTimeout 0).
  setTimeout(() => { if (docNode(id)) startEditing(id, offset, null, paneSurface(router)); }, 0);
}

/** Zoom into the selected block, else the block being edited (the last one when
 *  several are selected). An edit continues in the zoomed view. */
export function zoomInOnTarget(): boolean {
  const ids = commandTargetIds();
  const id = ids[ids.length - 1];
  if (!id || !docNode(id)) {
    pushToast(msg("commands-select-a-block-or-edit-one-to-zoom-in"), "warn");
    return false;
  }
  const wasEditing = editingId() === id;
  const context = wasEditing ? captureHistoryEditorContext() : null;
  const router = zoomRouter(context?.surface);
  router.focusBlock(id);
  if (context) keepEditing(id, context.selectionStart, router);
  return true;
}

/** Zoom out one level in the focused pane: from the zoomed block to its parent
 *  block, or to the page when the zoomed block is a page root. A pane showing no
 *  zoomed block does nothing, so repeating the command is safe. */
export function zoomOutOneLevel(): boolean {
  const context = editingId() ? captureHistoryEditorContext() : null;
  const router = zoomRouter(context?.surface);
  const route = router.route();
  if (route.kind !== "page" || !route.block) return false;
  const zoomed = resolveRouteBlock(route);
  const parent = zoomed ? docNode(zoomed)?.parent ?? null : null;
  router.focusBlock(parent);
  // Only an edit inside this pane continues; a sidebar editor is left alone.
  if (context && context.surface === paneSurface(router)) keepEditing(context.blockId, context.selectionStart, router);
  return true;
}
