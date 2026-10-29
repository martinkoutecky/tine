import { createComputed, createSignal, onCleanup, useContext } from "solid-js";
import { blockPageReadOnly, node as docNode, reportQueryFoldOption, setCollapsed, storedFold } from "../document";
import { CollapseSurfaceContext, type CollapseSurfaceApi } from "./Block";
import { readOnlyFold } from "./block/readOnlyFold";

/** Whether a macro host's own bullet is folded in this surface (the surface's
 *  fold state, else the stored `collapsed::`). Its body (a page embed's page,
 *  a query's results) folds with it. */
export function hostFolded(surface: CollapseSurfaceApi | null, id: string): boolean {
  const stored = storedFold(id);
  return (blockPageReadOnly(id) ? readOnlyFold : surface)?.collapsed(id, stored) ?? stored;
}

/** A query's results fold. GH #619 Q8: for a query block this IS the bullet's
 *  fold, one state with two controls (OG query.cljs `custom-query*` reads the
 *  block's `:block/collapsed?`), so the query's arrow reads and writes the
 *  bullet's `collapsed::` (or the surface's local fold in a reference or embed
 *  surface). The `:collapsed?` option stays the default of a block that has no
 *  `collapsed::` of its own; expanding such a block writes `collapsed:: false`.
 *
 *  A read-only host shares the bullet's session-only presentation fold. A
 *  query with no host block (a journal default query) retains its existing
 *  device-local toggle (`remembered`, written back through `remember`). */
export function createQueryFold(remembered: boolean | null, sourceDefault: () => boolean,
  hostId: () => string | undefined, remember: (collapsed: boolean) => void) {
  const surface = useContext(CollapseSurfaceContext);
  // Report the authored default before the first paint, so the bullet agrees.
  createComputed(() => {
    const id = hostId();
    if (!id || !docNode(id) || !sourceDefault()) return;
    onCleanup(reportQueryFoldOption(id));
  });
  const shared = () => { const id = hostId(); return !!id && !!docNode(id); };
  const [override, setOverride] = createSignal(remembered);
  const bulletFolded = () => { const id = hostId(); return !!id && hostFolded(surface, id); };
  const collapsed = () => shared() ? bulletFolded() : bulletFolded() || (override() ?? sourceDefault());
  const toggle = () => {
    const id = hostId();
    if (shared() && id) {
      if (blockPageReadOnly(id)) readOnlyFold.toggle(id, collapsed());
      else if (surface) surface.toggle(id, collapsed());
      else setCollapsed(id, !collapsed());
      return;
    }
    const v = !collapsed();
    setOverride(v);
    remember(v);
  };
  return { collapsed, toggle };
}
