import { createSignal, useContext } from "solid-js";
import { node as docNode, setCollapsed } from "../document";
import { CollapseSurfaceContext, type CollapseSurfaceApi } from "./Block";

/** Whether a macro host's own bullet is folded in this surface (the surface's
 *  fold state, else the stored `collapsed::`). Its body (a page embed's page,
 *  a query's results) folds with it. */
export function hostFolded(surface: CollapseSurfaceApi | null, id: string): boolean {
  const stored = docNode(id)?.collapsed ?? false;
  return surface?.collapsed(id, stored) ?? stored;
}

/** A query's results fold: the query's own toggle (`remembered`, written back
 *  through `remember`), its `:collapsed?` option as the default, and the host
 *  bullet. GH #619 c6: folding the bullet folds the results, as OG query.cljs
 *  `custom-query*` reads the block's `:block/collapsed?`. */
export function createQueryFold(remembered: boolean | null, sourceDefault: () => boolean,
  hostId: () => string | undefined, remember: (collapsed: boolean) => void) {
  const surface = useContext(CollapseSurfaceContext);
  const [override, setOverride] = createSignal(remembered);
  const bulletFolded = () => { const id = hostId(); return !!id && hostFolded(surface, id); };
  const collapsed = () => bulletFolded() || (override() ?? sourceDefault());
  const toggle = () => {
    const v = !collapsed();
    // Expanding a query whose bullet is folded unfolds the bullet, or the arrow would do nothing.
    const id = hostId();
    if (!v && id && bulletFolded()) {
      if (surface) surface.toggle(id, true);
      else setCollapsed(id, false);
    }
    setOverride(v);
    remember(v);
  };
  return { collapsed, toggle };
}
